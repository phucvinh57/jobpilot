//! A tab in the user's own Chrome, opened and driven through the JobPilot extension ([`Bridge`]):
//! what discovery, the session check and apply drive.
//!
//! Every method is a call to the extension (`tab.*`, or `cdp.send` with an allowlisted Chrome
//! DevTools method; see `extension/PROTOCOL.md`). A failed call is an `anyhow::Error` holding a
//! [`BridgeError`] (a closed tab is [`BridgeError::NotOwned`]).

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use super::bridge::protocol::{CookieDomains, Opened, TabInfo};
use super::bridge::{Bridge, BridgeError, TabShared};
use super::guard::{allows, Policy};
use crate::engine::net::http::NotPublic;

/// How long a navigation call may take (Chrome answers `Page.navigate` once the page responds).
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long closing a tab may take.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest total size of the files given to one file input.
const FILES_MAX: usize = 20 * 1024 * 1024;

/// Which browser profile a tab opens in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Profile {
    /// The JobPilot Incognito window: no cookies of the user's (discovery, reading pages).
    Private,
    /// A JobPilot window in the user's normal profile, where they are signed in (applying).
    Signed,
}

/// How [`Bridge::open`] opens a tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenOptions {
    pub profile: Profile,
    /// Bring the window and the tab to the front (login, hand-off to the user).
    pub focus: bool,
    /// Keep the tab's requests to what `Policy` allows (discovery). `None` = no guard.
    pub guard: Option<Policy>,
}

/// A part of the page for [`Tab::screenshot`], in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clip {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

/// A tab JobPilot opened. Cheap to clone; every clone is the same tab.
#[derive(Clone)]
pub struct Tab {
    bridge: Bridge,
    id: i64,
    profile: Profile,
    shared: Arc<TabShared>,
}

impl fmt::Debug for Tab {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tab").field("id", &self.id).field("profile", &self.profile).finish()
    }
}

/// Fail when a guarded tab may not go to `url` (checked before the browser asks for it).
async fn check_guard(guard: Option<&Policy>, url: &str) -> Result<()> {
    if let (Some(policy), Ok(parsed)) = (guard, url::Url::parse(url)) {
        if !allows(policy, &parsed).await {
            return Err(NotPublic.into());
        }
    }
    Ok(())
}

impl Bridge {
    /// Open `url` (http(s) or `about:blank`) in a new tab of the JobPilot window of `opts.profile`.
    /// Returns once the navigation started; use [`Tab::wait_loaded`]. A guarded tab is refused an
    /// address its policy does not allow before anything is opened.
    pub async fn open(&self, url: &str, opts: OpenOptions) -> Result<Tab> {
        check_guard(opts.guard.as_ref(), url).await?;
        let id = self.shared.next_id();
        if let Some(policy) = &opts.guard {
            self.shared.state().guarded_opens.insert(id, policy.clone());
        }
        let params = json!({
            "url": url,
            "private": opts.profile == Profile::Private,
            "focus": opts.focus,
            "guard": opts.guard.is_some(),
        });
        let answer = self.call_id(id, "tab.open", params, NAVIGATE_TIMEOUT).await;
        let shared = {
            let mut state = self.shared.state();
            state.guarded_opens.remove(&id);
            let opened: Opened = serde_json::from_value(answer?).context("the extension's answer to tab.open")?;
            // The guard may already know the tab (it answered requests of the first navigation).
            let shared = state.tabs.entry(opened.tab).or_insert_with(|| TabShared::new(opts.guard.clone())).clone();
            *shared.guard.lock().unwrap_or_else(|e| e.into_inner()) = opts.guard.clone();
            tracing::info!("browser tab {} opened ({:?}, guard {})", opened.tab, opts.profile, opts.guard.is_some());
            (opened.tab, shared)
        };
        Ok(Tab { bridge: self.clone(), id: shared.0, profile: opts.profile, shared: shared.1 })
    }

    /// The domains the browser holds cookies for under `domain` (leading dots removed, sorted);
    /// `private` = the JobPilot Incognito window's store. Cookie names and values never leave the
    /// browser.
    pub async fn cookie_domains(&self, domain: &str, private: bool) -> Result<Vec<String>> {
        let answer = self.call("cookies.domains", json!({ "domain": domain, "private": private })).await?;
        let domains: CookieDomains = serde_json::from_value(answer).context("the extension's answer to cookies.domains")?;
        Ok(domains.domains)
    }
}

impl Tab {
    /// Chrome's id of the tab.
    pub fn id(&self) -> i64 {
        self.id
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Requests the guard refused so far.
    pub fn blocked(&self) -> usize {
        self.shared.blocked.load(Ordering::SeqCst)
    }

    /// Send a Chrome DevTools command to the tab (only the methods the extension allows).
    pub async fn cdp(&self, method: &str, params: Value) -> Result<Value> {
        self.cdp_timeout(method, params, super::bridge::CALL_TIMEOUT).await
    }

    async fn cdp_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let answer = self.bridge.call_timeout("cdp.send", json!({ "tab": self.id, "method": method, "params": params }), timeout).await;
        Ok(self.seen(answer)?)
    }

    /// A call about this tab: a "not owned" answer means it is closed.
    fn seen(&self, answer: Result<Value, BridgeError>) -> Result<Value, BridgeError> {
        if matches!(answer, Err(BridgeError::NotOwned)) {
            self.shared.mark_closed();
        }
        answer
    }

    async fn tab_call(&self, method: &str, timeout: Duration) -> Result<Value> {
        let answer = self.bridge.call_timeout(method, json!({ "tab": self.id }), timeout).await;
        Ok(self.seen(answer)?)
    }

    /// Navigate to `url` (http(s)). A guarded tab is refused an address its policy does not allow.
    pub async fn goto(&self, url: &str) -> Result<()> {
        let guard = self.shared.guard.lock().unwrap_or_else(|e| e.into_inner()).clone();
        check_guard(guard.as_ref(), url).await?;
        let answer = self.bridge.call_timeout("tab.navigate", json!({ "tab": self.id, "url": url }), NAVIGATE_TIMEOUT).await;
        self.seen(answer)?;
        Ok(())
    }

    /// Wait until `document.readyState` is `complete` (polled every 200 ms), at most `timeout`.
    /// Never fails: a page that keeps loading is read as it is.
    pub async fn wait_loaded(&self, timeout: Duration) {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            if left.is_zero() || !self.is_open() {
                return;
            }
            let step = left.min(Duration::from_secs(5));
            if let Ok(Value::String(state)) = self.eval_timeout("document.readyState", step).await {
                if state == "complete" {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(200).min(deadline.saturating_duration_since(tokio::time::Instant::now()))).await;
        }
    }

    async fn info(&self) -> Result<TabInfo> {
        let answer = self.tab_call("tab.info", super::bridge::CALL_TIMEOUT).await?;
        serde_json::from_value(answer).context("the extension's answer to tab.info")
    }

    /// The page's address (`location.href`; Chrome's idea of it while no document answers).
    pub async fn url(&self) -> Result<String> {
        match self.eval_as::<String>("location.href").await {
            Ok(url) => Ok(url),
            Err(_) => Ok(self.info().await?.url),
        }
    }

    pub async fn title(&self) -> Result<String> {
        match self.eval_as::<String>("document.title").await {
            Ok(title) => Ok(title),
            Err(_) => Ok(self.info().await?.title),
        }
    }

    /// The page's HTML, doctype included.
    pub async fn content(&self) -> Result<String> {
        self.eval_as(
            "(() => { let html = ''; if (document.doctype) html = new XMLSerializer().serializeToString(document.doctype); \
             if (document.documentElement) html += document.documentElement.outerHTML; return html; })()",
        )
        .await
    }

    /// Evaluate `expression` in the page (a promise is awaited) and return its value as JSON
    /// (`null` for `undefined`). A thrown exception is an error with its text.
    pub async fn eval(&self, expression: &str) -> Result<Value> {
        self.eval_timeout(expression, super::bridge::CALL_TIMEOUT).await
    }

    async fn eval_timeout(&self, expression: &str, timeout: Duration) -> Result<Value> {
        let params = json!({ "expression": expression, "returnByValue": true, "awaitPromise": true, "userGesture": true });
        let answer = self.cdp_timeout("Runtime.evaluate", params, timeout).await?;
        evaluated(answer)
    }

    /// [`eval`](Self::eval) into `T`.
    pub async fn eval_as<T: DeserializeOwned>(&self, expression: &str) -> Result<T> {
        let value = self.eval(expression).await?;
        serde_json::from_value(value).context("the page returned an unexpected value")
    }

    /// A PNG of the visible page, or of `clip` (which may lie beyond the viewport).
    pub async fn screenshot(&self, clip: Option<Clip>) -> Result<Vec<u8>> {
        let mut params = json!({ "format": "png" });
        if let (Some(clip), Value::Object(map)) = (clip, &mut params) {
            map.insert("clip".into(), json!({ "x": clip.x, "y": clip.y, "width": clip.width, "height": clip.height, "scale": clip.scale }));
            map.insert("captureBeyondViewport".into(), Value::Bool(true));
        }
        let answer = self.cdp_timeout("Page.captureScreenshot", params, Duration::from_secs(60)).await?;
        let data = answer.get("data").and_then(Value::as_str).ok_or_else(|| anyhow!("the browser returned no screenshot"))?;
        base64::engine::general_purpose::STANDARD.decode(data).context("the screenshot is not base64")
    }

    /// Click the centre of the first element matching `selector` (scrolled into view first), like
    /// a user's left click. Fails when nothing matches.
    pub async fn click(&self, selector: &str) -> Result<()> {
        let doc = self.cdp("DOM.getDocument", json!({ "depth": 0 })).await?;
        let root = doc.pointer("/root/nodeId").and_then(Value::as_i64).ok_or_else(|| anyhow!("the page has no document"))?;
        let found = self.cdp("DOM.querySelector", json!({ "nodeId": root, "selector": selector })).await?;
        let node = found.get("nodeId").and_then(Value::as_i64).filter(|n| *n != 0).ok_or_else(|| anyhow!("no element on the page matches `{selector}`"))?;
        self.cdp("DOM.scrollIntoViewIfNeeded", json!({ "nodeId": node })).await?;
        let model = self.cdp("DOM.getBoxModel", json!({ "nodeId": node })).await?;
        let (x, y) = quad_centre(model.pointer("/model/content")).ok_or_else(|| anyhow!("the element `{selector}` is not visible"))?;
        self.cdp("Input.dispatchMouseEvent", json!({ "type": "mouseMoved", "x": x, "y": y })).await?;
        for kind in ["mousePressed", "mouseReleased"] {
            self.cdp("Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 })).await?;
        }
        Ok(())
    }

    /// Type `text` into the focused element, one key per character.
    pub async fn type_text(&self, text: &str) -> Result<()> {
        for c in text.chars() {
            match c {
                '\n' => self.press_key("Enter").await?,
                '\t' => self.press_key("Tab").await?,
                c => {
                    let s = c.to_string();
                    self.cdp("Input.dispatchKeyEvent", json!({ "type": "keyDown", "key": s, "text": s, "unmodifiedText": s })).await?;
                    self.cdp("Input.dispatchKeyEvent", json!({ "type": "keyUp", "key": s })).await?;
                }
            }
        }
        Ok(())
    }

    /// Press and release a named key: `Escape`, `Enter`, `Tab`, `Backspace`, `Delete`, `Space`,
    /// `ArrowDown`/`Up`/`Left`/`Right`, `Home`, `End`, `PageUp`, `PageDown`.
    pub async fn press_key(&self, key: &str) -> Result<()> {
        let Some(k) = named_key(key) else { bail!("unknown key `{key}`") };
        let mut down = json!({
            "type": if k.text.is_empty() { "rawKeyDown" } else { "keyDown" },
            "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.key_code, "nativeVirtualKeyCode": k.key_code,
        });
        if let (false, Value::Object(map)) = (k.text.is_empty(), &mut down) {
            map.insert("text".into(), Value::String(k.text.into()));
            map.insert("unmodifiedText".into(), Value::String(k.text.into()));
        }
        self.cdp("Input.dispatchKeyEvent", down).await?;
        let up = json!({ "type": "keyUp", "key": k.key, "code": k.code, "windowsVirtualKeyCode": k.key_code, "nativeVirtualKeyCode": k.key_code });
        self.cdp("Input.dispatchKeyEvent", up).await?;
        Ok(())
    }

    /// Put `files` into the file input matching `selector` (as if the user picked them) and fire
    /// its `input` and `change` events.
    pub async fn set_files(&self, selector: &str, files: &[PathBuf]) -> Result<()> {
        let mut total = 0;
        let mut list = Vec::with_capacity(files.len());
        for path in files {
            let bytes = tokio::fs::read(path).await.with_context(|| format!("reading {}", path.display()))?;
            total += bytes.len();
            if total > FILES_MAX {
                bail!("the files are larger than {} MB", FILES_MAX / 1024 / 1024);
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
            let mime = mime_of(&name);
            list.push(json!({ "name": name, "type": mime, "data": base64::engine::general_purpose::STANDARD.encode(&bytes) }));
        }
        let selector_js = serde_json::to_string(selector).context("encoding the selector")?;
        let files_js = serde_json::to_string(&list).context("encoding the files")?;
        let script = format!(
            "(() => {{ const input = document.querySelector({selector_js}); \
             if (!input) throw new Error('no element on the page matches the selector'); \
             if (!(input instanceof HTMLInputElement) || input.type !== 'file') throw new Error('the element is not a file input'); \
             const dt = new DataTransfer(); \
             for (const f of {files_js}) {{ const bin = atob(f.data); const bytes = new Uint8Array(bin.length); \
               for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i); \
               dt.items.add(new File([bytes], f.name, {{ type: f.type }})); }} \
             input.files = dt.files; \
             input.dispatchEvent(new Event('input', {{ bubbles: true }})); \
             input.dispatchEvent(new Event('change', {{ bubbles: true }})); \
             return input.files.length; }})()"
        );
        let count: usize = self.eval_as(&script).await?;
        if count != files.len() {
            bail!("the file input took {count} of {} files", files.len());
        }
        Ok(())
    }

    /// Bring the tab and its window to the front.
    pub async fn focus(&self) -> Result<()> {
        self.tab_call("tab.focus", super::bridge::CALL_TIMEOUT).await?;
        Ok(())
    }

    /// False once the tab was closed (by the user or by [`close`](Self::close)).
    pub fn is_open(&self) -> bool {
        !*self.shared.closed.borrow()
    }

    /// Resolves when the tab is closed.
    pub async fn closed(&self) {
        let mut rx = self.shared.closed.subscribe();
        let _ = rx.wait_for(|closed| *closed).await;
    }

    /// Close the tab (best effort).
    pub async fn close(&self) {
        if !self.is_open() {
            return;
        }
        if let Err(e) = self.bridge.call_timeout("tab.close", json!({ "tab": self.id }), CLOSE_TIMEOUT).await {
            tracing::debug!("closing browser tab {}: {e}", self.id);
        }
        self.shared.mark_closed();
        self.bridge.shared.state().tabs.remove(&self.id);
    }
}

/// The value of a `Runtime.evaluate` answer, or the exception it threw.
fn evaluated(answer: Value) -> Result<Value> {
    if let Some(details) = answer.get("exceptionDetails") {
        let text = details
            .pointer("/exception/description")
            .and_then(Value::as_str)
            .or_else(|| details.get("text").and_then(Value::as_str))
            .unwrap_or("the script failed");
        bail!("the page script failed: {text}");
    }
    Ok(answer.pointer("/result/value").cloned().unwrap_or(Value::Null))
}

/// The centre of a CDP quad (`[x1, y1, ... x4, y4]`).
fn quad_centre(quad: Option<&Value>) -> Option<(f64, f64)> {
    let points: Vec<f64> = quad?.as_array()?.iter().filter_map(Value::as_f64).collect();
    if points.len() != 8 {
        return None;
    }
    let x = (points[0] + points[2] + points[4] + points[6]) / 4.0;
    let y = (points[1] + points[3] + points[5] + points[7]) / 4.0;
    Some((x, y))
}

struct NamedKey {
    key: &'static str,
    code: &'static str,
    key_code: u32,
    text: &'static str,
}

fn named_key(key: &str) -> Option<NamedKey> {
    let (key, code, key_code, text) = match key {
        "Enter" => ("Enter", "Enter", 13, "\r"),
        "Escape" => ("Escape", "Escape", 27, ""),
        "Tab" => ("Tab", "Tab", 9, ""),
        "Backspace" => ("Backspace", "Backspace", 8, ""),
        "Delete" => ("Delete", "Delete", 46, ""),
        "Space" | " " => (" ", "Space", 32, " "),
        "ArrowDown" => ("ArrowDown", "ArrowDown", 40, ""),
        "ArrowUp" => ("ArrowUp", "ArrowUp", 38, ""),
        "ArrowLeft" => ("ArrowLeft", "ArrowLeft", 37, ""),
        "ArrowRight" => ("ArrowRight", "ArrowRight", 39, ""),
        "Home" => ("Home", "Home", 36, ""),
        "End" => ("End", "End", 35, ""),
        "PageUp" => ("PageUp", "PageUp", 33, ""),
        "PageDown" => ("PageDown", "PageDown", 34, ""),
        _ => return None,
    };
    Some(NamedKey { key, code, key_code, text })
}

/// The MIME type of a file to upload, by its extension.
fn mime_of(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "doc" => "application/msword",
        "txt" => "text/plain",
        "md" => "text/markdown",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluation_answers_become_values_or_errors() {
        assert_eq!(evaluated(json!({"result": {"type": "string", "value": "complete"}})).unwrap(), json!("complete"));
        assert_eq!(evaluated(json!({"result": {"type": "undefined"}})).unwrap(), Value::Null);
        let err = evaluated(json!({"result": {"type": "object"}, "exceptionDetails": {"text": "Uncaught", "exception": {"description": "Error: boom\n    at <anonymous>:1:7"}}})).unwrap_err();
        assert!(err.to_string().contains("Error: boom"), "{err}");
        let err = evaluated(json!({"exceptionDetails": {"text": "Uncaught SyntaxError"}})).unwrap_err();
        assert!(err.to_string().contains("SyntaxError"), "{err}");
    }

    #[test]
    fn the_centre_of_a_quad() {
        assert_eq!(quad_centre(Some(&json!([10, 20, 30, 20, 30, 40, 10, 40]))), Some((20.0, 30.0)));
        assert_eq!(quad_centre(Some(&json!([1, 2]))), None);
        assert_eq!(quad_centre(None), None);
    }

    #[test]
    fn keys_and_mime_types() {
        let enter = named_key("Enter").unwrap();
        assert_eq!((enter.code, enter.key_code, enter.text), ("Enter", 13, "\r"));
        assert_eq!(named_key("ArrowDown").unwrap().key_code, 40);
        assert_eq!(named_key("Escape").unwrap().text, "");
        assert!(named_key("F13").is_none());
        assert_eq!(mime_of("cv.PDF"), "application/pdf");
        assert_eq!(mime_of("cv.docx"), "application/vnd.openxmlformats-officedocument.wordprocessingml.document");
        assert_eq!(mime_of("photo.jpeg"), "image/jpeg");
        assert_eq!(mime_of("README"), "application/octet-stream");
    }
}
