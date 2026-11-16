//! Driving the application page: read fields, set values, press the next/submit buttons, take
//! screenshots. Everything here acts on field ids and button kinds chosen by deterministic code; none
//! of it takes an instruction from page text or from the model.

use std::path::Path;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use super::ats::Adapter;
use super::detect::PageSignals;
use super::fields::{FieldKind, FormField};
use super::js;
use crate::engine::browser::{Clip, Tab};

/// Longest a polling script may take (the page may be navigating or busy).
const POLL_TIMEOUT: Duration = Duration::from_secs(5);

/// Run a script and read its JSON result.
pub async fn eval<T: DeserializeOwned>(tab: &Tab, script: String) -> Result<T> {
    tab.eval_as(&script).await.context("unexpected script result")
}

/// [`eval`], given up after a few seconds: for loops that poll a page that may be navigating.
pub async fn eval_quick<T: DeserializeOwned>(tab: &Tab, script: String) -> Result<T> {
    match tokio::time::timeout(POLL_TIMEOUT, eval(tab, script)).await {
        Ok(result) => result,
        Err(_) => bail!("the page did not answer in time"),
    }
}

/// Give a freshly loaded or navigated page a moment to render.
pub async fn settle(tab: &Tab) {
    tab.wait_loaded(Duration::from_secs(5)).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// Tag and describe the visible fields of the current step.
pub async fn extract(tab: &Tab, adapter: &Adapter, step: usize) -> Result<Vec<FormField>> {
    eval_quick(tab, js::extract(step, adapter.root)).await.context("reading the form fields")
}

/// Like [`extract`], but waits (a lazily rendered form) until at least one field is there.
pub async fn extract_waiting(tab: &Tab, adapter: &Adapter, step: usize, wait: Duration) -> Result<Vec<FormField>> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // The page may be navigating: errors are retried until the deadline.
        match extract(tab, adapter, step).await {
            Ok(fields) if !fields.is_empty() => return Ok(fields),
            Ok(fields) if tokio::time::Instant::now() >= deadline => return Ok(fields),
            Err(e) if tokio::time::Instant::now() >= deadline => return Err(e),
            _ => tokio::time::sleep(Duration::from_millis(400)).await,
        }
    }
}

pub async fn signals(tab: &Tab, adapter: &Adapter) -> Result<PageSignals> {
    eval_quick(tab, js::signals(adapter.root)).await.context("reading the page")
}

/// Which button kinds the page offers right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Apply,
    Next,
    Submit,
}

impl Button {
    fn kind(self) -> &'static str {
        match self {
            Button::Apply => "apply",
            Button::Next => "next",
            Button::Submit => "submit",
        }
    }
}

#[derive(Deserialize)]
struct Found {
    found: bool,
    #[serde(default)]
    text: String,
}

/// Tag the button of `kind` if the page has one; returns its text.
pub async fn find_button(tab: &Tab, adapter: &Adapter, button: Button) -> Result<Option<String>> {
    let (selectors, text, root): (&[&str], &str, &[&str]) = match button {
        Button::Apply => (adapter.apply_button, adapter.apply_text, &[]),
        Button::Next => (adapter.next, adapter.next_text, adapter.root),
        Button::Submit => (adapter.submit, adapter.submit_text, adapter.root),
    };
    let found: Found = eval(tab, js::find_button(button.kind(), root, selectors, text)).await?;
    Ok(found.found.then_some(found.text))
}

/// Click the button tagged by [`find_button`]. A real mouse click first; a script click as the fallback
/// (for buttons without a box to click, such as ones the page hides behind an overlay).
pub async fn click_button(tab: &Tab, button: Button) -> Result<()> {
    let selector = format!("[data-jp-btn=\"{}\"]", button.kind());
    if let Err(e) = tab.click(&selector).await {
        tracing::debug!("the {} button took no mouse click ({e:#}): clicking it by script", button.kind());
        let clicked: bool =
            eval(tab, format!("(() => {{ const b = document.querySelector('{selector}'); if (!b) return false; b.click(); return true; }})()")).await?;
        if !clicked {
            bail!("the {} button vanished", button.kind());
        }
    }
    Ok(())
}

/// One value to set.
#[derive(Clone, Debug)]
pub struct FillItem {
    pub id: String,
    pub kind: FieldKind,
    pub value: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct FillResult {
    pub id: String,
    pub ok: bool,
    /// The value the page holds now.
    pub now: String,
    pub error: Option<String>,
}

/// Set every item. Text, selects, radios and checkboxes go through one script; file inputs get the
/// file as if the user picked it ([`Tab::set_files`]); type-ahead dropdowns are typed into and an
/// option is clicked.
pub async fn fill(tab: &Tab, items: &[FillItem]) -> Result<Vec<FillResult>> {
    let mut results = Vec::with_capacity(items.len());
    let scripted: Vec<serde_json::Value> = items
        .iter()
        .filter(|i| !matches!(i.kind, FieldKind::File | FieldKind::Combobox))
        .map(|i| json!({"id": i.id, "type": i.kind.as_str(), "value": i.value}))
        .collect();
    if !scripted.is_empty() {
        results.extend(eval::<Vec<FillResult>>(tab, js::fill(&json!(scripted))).await.context("filling the form")?);
    }
    for item in items {
        match item.kind {
            FieldKind::File => results.push(attach_file(tab, &item.id, &item.value).await),
            FieldKind::Combobox => results.push(fill_combobox(tab, &item.id, &item.value).await),
            _ => {}
        }
    }
    Ok(results)
}

fn selector(id: &str) -> String {
    format!("[data-jp-id=\"{id}\"]")
}

async fn attach_file(tab: &Tab, id: &str, path: &str) -> FillResult {
    let outcome: Result<String> = async {
        let abs = std::fs::canonicalize(path).with_context(|| format!("the file {path} does not exist"))?;
        tab.set_files(&selector(id), std::slice::from_ref(&abs)).await.context("attaching the file")?;
        // Upload widgets often take the file and reset the input: the page then shows the name instead.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let base = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name: String = eval(
            tab,
            format!(
                "(() => {{ const e = document.querySelector('{}'); if (e && e.files && e.files[0]) return e.files[0].name; \
                 return document.body && document.body.innerText.includes({}) ? {} : ''; }})()",
                selector(id),
                json!(base),
                json!(base)
            ),
        )
        .await?;
        Ok(name)
    }
    .await;
    match outcome {
        Ok(name) if !name.is_empty() => FillResult { id: id.into(), ok: true, now: name, error: None },
        // The browser took the file but the widget shows nothing (yet): many upload widgets upload
        // asynchronously and reset the input. The screenshot shows the result.
        Ok(_) => FillResult { id: id.into(), ok: true, now: String::new(), error: None },
        Err(e) => FillResult { id: id.into(), ok: false, now: String::new(), error: Some(format!("{e:#}")) },
    }
}

async fn fill_combobox(tab: &Tab, id: &str, value: &str) -> FillResult {
    let outcome: Result<bool> = async {
        tab.click(&selector(id)).await.context("opening the dropdown")?;
        tab.type_text(value).await.context("typing in the dropdown")?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let pick = format!(
            "(() => {{ const v = {}.trim().toLowerCase(); \
              const opts = [...document.querySelectorAll('[role=option]')].filter((o) => o.offsetParent !== null); \
              const t = (o) => (o.innerText || o.textContent || '').trim().toLowerCase(); \
              const o = opts.find((o) => t(o) === v) || opts.find((o) => t(o).includes(v) || v.includes(t(o))); \
              if (!o) return false; o.click(); return true; }})()",
            json!(value)
        );
        let picked: bool = eval(tab, pick).await?;
        if !picked {
            // Do not leave half-typed text behind.
            let _ = tab.press_key("Escape").await;
        }
        Ok(picked)
    }
    .await;
    match outcome {
        Ok(true) => FillResult { id: id.into(), ok: true, now: value.into(), error: None },
        Ok(false) => FillResult { id: id.into(), ok: false, now: String::new(), error: Some("no dropdown option matched".into()) },
        Err(e) => FillResult { id: id.into(), ok: false, now: String::new(), error: Some(format!("{e:#}")) },
    }
}

/// How many options of a type-ahead dropdown are worth showing to the model.
const MAX_COMBOBOX_OPTIONS: usize = 60;

/// Open each type-ahead dropdown once to read its options (they only exist while the list is open), so
/// the answer can be chosen from, and validated against, the real list. Long lists (countries) and
/// search-as-you-type lists stay free text.
pub async fn enrich_comboboxes(tab: &Tab, fields: &mut [FormField]) {
    for f in fields.iter_mut().filter(|f| f.kind == FieldKind::Combobox && f.options.is_empty()) {
        let options: Result<Vec<String>> = async {
            tab.click(&selector(&f.id)).await?;
            tokio::time::sleep(Duration::from_millis(250)).await;
            let options: Vec<String> = eval(tab, js::options()).await?;
            let _ = tab.press_key("Escape").await;
            Ok(options)
        }
        .await;
        if let Ok(options) = options {
            if !options.is_empty() && options.len() <= MAX_COMBOBOX_OPTIONS {
                f.options = options;
            }
        }
    }
}

/// PNG of the filled form: the form's own area when it can be found, else the whole page.
pub async fn screenshot_form(tab: &Tab, adapter: &Adapter, path: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    let rect: Option<Rect> = eval(tab, js::root_rect(adapter.root)).await.ok().flatten();
    let Some(r) = rect else { return screenshot(tab, path).await };
    let x = (r.x - 12.0).max(0.0);
    let y = (r.y - 12.0).max(0.0);
    let clip = Clip { x, y, width: (r.w + 24.0).min(1600.0), height: (r.h + 24.0).min(16000.0), scale: 1.0 };
    match tab.screenshot(Some(clip)).await {
        Ok(bytes) => write_png(path, &bytes),
        Err(_) => screenshot(tab, path).await,
    }
}

/// PNG of the whole page (at most 4000 x 16000 px), or of what is in view when its size cannot be read.
pub async fn screenshot(tab: &Tab, path: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Size {
        w: f64,
        h: f64,
    }
    let script = "(() => { const d = document.documentElement, b = document.body; \
        return { w: Math.max(d ? d.scrollWidth : 0, b ? b.scrollWidth : 0), h: Math.max(d ? d.scrollHeight : 0, b ? b.scrollHeight : 0) }; })()";
    let size: Option<Size> = eval_quick(tab, script.to_string()).await.ok();
    let clip = size
        .filter(|s| s.w >= 1.0 && s.h >= 1.0)
        .map(|s| Clip { x: 0.0, y: 0.0, width: s.w.min(4000.0), height: s.h.min(16000.0), scale: 1.0 });
    let bytes = tab.screenshot(clip).await.context("taking a screenshot")?;
    write_png(path, &bytes)
}

fn write_png(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// The URL of an embedded ATS form (Greenhouse/Lever/Ashby iframe) on a company's own careers page.
pub async fn embedded_form_url(tab: &Tab) -> Option<String> {
    eval::<Option<String>>(tab, js::embedded_form()).await.ok().flatten().filter(|u| u.starts_with("http"))
}

/// Stable text of the field list; changes when the form moves to another step.
pub fn signature(fields: &[FormField]) -> String {
    fields.iter().map(|f| format!("{}:{}", f.kind.as_str(), f.label)).collect::<Vec<_>>().join("|")
}
