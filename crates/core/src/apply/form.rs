//! Driving the application page: read fields, set values, press the next/submit buttons, take
//! screenshots. Everything here acts on field ids and button kinds chosen by deterministic code; none
//! of it takes an instruction from page text or from the model.

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, Viewport};
use chromiumoxide::page::ScreenshotParams;
use chromiumoxide::Page;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::json;

use super::ats::Adapter;
use super::detect::PageSignals;
use super::fields::{FieldKind, FormField};
use super::js;

/// Run a script and read its JSON result.
pub async fn eval<T: DeserializeOwned>(page: &Page, script: String) -> Result<T> {
    let result = page.evaluate_expression(script).await.map_err(|e| anyhow!("{e}"))?;
    result.into_value::<T>().map_err(|e| anyhow!("unexpected script result: {e}"))
}

/// Give a freshly loaded or navigated page a moment to render.
pub async fn settle(page: &Page) {
    for _ in 0..25 {
        let ready: Result<String> = eval(page, "document.readyState".to_string()).await;
        if ready.is_ok_and(|s| s == "complete") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// Tag and describe the visible fields of the current step.
pub async fn extract(page: &Page, adapter: &Adapter, step: usize) -> Result<Vec<FormField>> {
    eval(page, js::extract(step, adapter.root)).await.context("reading the form fields")
}

/// Like [`extract`], but waits (a lazily rendered form) until at least one field is there.
pub async fn extract_waiting(page: &Page, adapter: &Adapter, step: usize, wait: Duration) -> Result<Vec<FormField>> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        // The page may be navigating: errors are retried until the deadline.
        match extract(page, adapter, step).await {
            Ok(fields) if !fields.is_empty() => return Ok(fields),
            Ok(fields) if tokio::time::Instant::now() >= deadline => return Ok(fields),
            Err(e) if tokio::time::Instant::now() >= deadline => return Err(e),
            _ => tokio::time::sleep(Duration::from_millis(400)).await,
        }
    }
}

pub async fn signals(page: &Page, adapter: &Adapter) -> Result<PageSignals> {
    eval(page, js::signals(adapter.root)).await.context("reading the page")
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
pub async fn find_button(page: &Page, adapter: &Adapter, button: Button) -> Result<Option<String>> {
    let (selectors, text, root): (&[&str], &str, &[&str]) = match button {
        Button::Apply => (adapter.apply_button, adapter.apply_text, &[]),
        Button::Next => (adapter.next, adapter.next_text, adapter.root),
        Button::Submit => (adapter.submit, adapter.submit_text, adapter.root),
    };
    let found: Found = eval(page, js::find_button(button.kind(), root, selectors, text)).await?;
    Ok(found.found.then_some(found.text))
}

/// Click the button tagged by [`find_button`]. A real mouse click first; a script click as the fallback
/// (for buttons covered by overlays).
pub async fn click_button(page: &Page, button: Button) -> Result<()> {
    let selector = format!("[data-jp-btn=\"{}\"]", button.kind());
    let el = page.find_element(selector.as_str()).await.map_err(|e| anyhow!("the {} button vanished: {e}", button.kind()))?;
    let _ = el.scroll_into_view().await;
    if el.click().await.is_err() {
        let clicked: bool =
            eval(page, format!("(() => {{ const b = document.querySelector('{selector}'); if (!b) return false; b.click(); return true; }})()")).await?;
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

/// Set every item. Text, selects, radios and checkboxes go through one script; file inputs use
/// `DOM.setFileInputFiles`; type-ahead dropdowns are typed into and an option is clicked.
pub async fn fill(page: &Page, items: &[FillItem]) -> Result<Vec<FillResult>> {
    let mut results = Vec::with_capacity(items.len());
    let scripted: Vec<serde_json::Value> = items
        .iter()
        .filter(|i| !matches!(i.kind, FieldKind::File | FieldKind::Combobox))
        .map(|i| json!({"id": i.id, "type": i.kind.as_str(), "value": i.value}))
        .collect();
    if !scripted.is_empty() {
        results.extend(eval::<Vec<FillResult>>(page, js::fill(&json!(scripted))).await.context("filling the form")?);
    }
    for item in items {
        match item.kind {
            FieldKind::File => results.push(attach_file(page, &item.id, &item.value).await),
            FieldKind::Combobox => results.push(fill_combobox(page, &item.id, &item.value).await),
            _ => {}
        }
    }
    Ok(results)
}

fn selector(id: &str) -> String {
    format!("[data-jp-id=\"{id}\"]")
}

async fn attach_file(page: &Page, id: &str, path: &str) -> FillResult {
    let outcome: Result<String> = async {
        let abs = std::fs::canonicalize(path).with_context(|| format!("the file {path} does not exist"))?;
        let el = page.find_element(selector(id)).await.map_err(|e| anyhow!("file input not found: {e}"))?;
        let params = SetFileInputFilesParams {
            files: vec![abs.to_string_lossy().into_owned()],
            node_id: None,
            backend_node_id: Some(el.backend_node_id),
            object_id: None,
        };
        page.execute(params).await.map_err(|e| anyhow!("attaching the file: {e}"))?;
        // Upload widgets often take the file and reset the input: the page then shows the name instead.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let base = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let name: String = eval(
            page,
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

async fn fill_combobox(page: &Page, id: &str, value: &str) -> FillResult {
    let outcome: Result<bool> = async {
        let el = page.find_element(selector(id)).await.map_err(|e| anyhow!("dropdown not found: {e}"))?;
        let _ = el.scroll_into_view().await;
        el.click().await.map_err(|e| anyhow!("opening the dropdown: {e}"))?;
        el.type_str(value).await.map_err(|e| anyhow!("typing in the dropdown: {e}"))?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let pick = format!(
            "(() => {{ const v = {}.trim().toLowerCase(); \
              const opts = [...document.querySelectorAll('[role=option]')].filter((o) => o.offsetParent !== null); \
              const t = (o) => (o.innerText || o.textContent || '').trim().toLowerCase(); \
              const o = opts.find((o) => t(o) === v) || opts.find((o) => t(o).includes(v) || v.includes(t(o))); \
              if (!o) return false; o.click(); return true; }})()",
            json!(value)
        );
        let picked: bool = eval(page, pick).await?;
        if !picked {
            // Do not leave half-typed text behind.
            let _ = el.press_key("Escape").await;
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
pub async fn enrich_comboboxes(page: &Page, fields: &mut [FormField]) {
    for f in fields.iter_mut().filter(|f| f.kind == FieldKind::Combobox && f.options.is_empty()) {
        let options: Result<Vec<String>> = async {
            let el = page.find_element(selector(&f.id)).await.map_err(|e| anyhow!("{e}"))?;
            let _ = el.scroll_into_view().await;
            el.click().await.map_err(|e| anyhow!("{e}"))?;
            tokio::time::sleep(Duration::from_millis(250)).await;
            let options: Vec<String> = eval(page, js::options()).await?;
            let _ = el.press_key("Escape").await;
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
pub async fn screenshot_form(page: &Page, adapter: &Adapter, path: &Path) -> Result<()> {
    #[derive(Deserialize)]
    struct Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }
    let rect: Option<Rect> = eval(page, js::root_rect(adapter.root)).await.ok().flatten();
    let Some(r) = rect else { return screenshot(page, path).await };
    let x = (r.x - 12.0).max(0.0);
    let y = (r.y - 12.0).max(0.0);
    let clip = Viewport { x, y, width: (r.w + 24.0).min(1600.0), height: (r.h + 24.0).min(16000.0), scale: 1.0 };
    let params = ScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .clip(clip)
        .capture_beyond_viewport(true)
        .build();
    match page.screenshot(params).await {
        Ok(bytes) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
        }
        Err(_) => screenshot(page, path).await,
    }
}

/// Full-page PNG of the current page.
pub async fn screenshot(page: &Page, path: &Path) -> Result<()> {
    let params = ScreenshotParams::builder().format(CaptureScreenshotFormat::Png).full_page(true).build();
    let bytes = page.screenshot(params).await.map_err(|e| anyhow!("taking a screenshot: {e}"))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
}

/// The URL of an embedded ATS form (Greenhouse/Lever/Ashby iframe) on a company's own careers page.
pub async fn embedded_form_url(page: &Page) -> Option<String> {
    eval::<Option<String>>(page, js::embedded_form()).await.ok().flatten().filter(|u| u.starts_with("http"))
}

/// Stable text of the field list; changes when the form moves to another step.
pub fn signature(fields: &[FormField]) -> String {
    fields.iter().map(|f| format!("{}:{}", f.kind.as_str(), f.label)).collect::<Vec<_>>().join("|")
}
