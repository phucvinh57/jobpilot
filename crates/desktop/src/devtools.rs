//! Developer tools for checking the UI without a person at the screen (cargo feature `dev-tools`,
//! off by default and never in a release package).
//!
//! A window capture cannot see the webview on Wayland, and a screen capture needs the user's
//! consent, so the webview takes its own picture and runs a script that drives the page:
//!
//! ```text
//! JOBPILOT_UI_SCRIPT=check.js JOBPILOT_UI_SHOTS=/tmp/shots \
//!   cargo run -p jobpilot --features dev-tools
//! ```
//!
//! The script is the body of an async function with these helpers in scope: `shot(name)` (PNG in
//! `JOBPILOT_UI_SHOTS`), `log(text)`, `sleep(ms)`, `waitFor(fn, ms)`, `$(selector)`,
//! `byText(text, selector)`, `click(target)`, `type(target, text)`, `invoke(command, args)`. The app
//! exits when the script ends.

use std::path::PathBuf;

use tauri::ipc::Invoke;
use tauri::{AppHandle, WebviewWindow};
use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};

/// The script to run once the page has loaded.
pub const SCRIPT_ENV: &str = "JOBPILOT_UI_SCRIPT";
/// Where `shot(name)` writes its PNGs (default: the system temp dir).
pub const SHOTS_ENV: &str = "JOBPILOT_UI_SHOTS";

pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![dev_snapshot, dev_log, dev_exit]
}

/// A file name from a script: letters, digits, `-` and `_` only, so it stays inside the shots dir.
fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    if cleaned.is_empty() {
        "shot".into()
    } else {
        cleaned
    }
}

fn shot_path(name: &str) -> PathBuf {
    let dir = std::env::var_os(SHOTS_ENV).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    dir.join(format!("{}.png", safe_name(name)))
}

/// Save what the webview shows as `<shots dir>/<name>.png`.
#[tauri::command]
async fn dev_snapshot(window: WebviewWindow, name: String) -> Result<String, String> {
    let path = shot_path(&name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let (tx, rx) = tokio::sync::oneshot::channel::<Result<(), String>>();
    let target = path.clone();
    window
        .with_webview(move |webview| {
            webview.inner().snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gtk::gio::Cancellable>, move |result| {
                let saved = result.map_err(|e| e.to_string()).and_then(|surface| {
                    let mut file = std::fs::File::create(&target).map_err(|e| e.to_string())?;
                    surface.write_to_png(&mut file).map_err(|e| e.to_string())
                });
                let _ = tx.send(saved);
            });
        })
        .map_err(|e| e.to_string())?;
    rx.await.map_err(|_| "the snapshot was dropped".to_string())??;
    Ok(path.display().to_string())
}

/// A line from the script, in the app log.
#[tauri::command]
fn dev_log(msg: String) {
    tracing::info!("ui script: {msg}");
}

#[tauri::command]
fn dev_exit(app: AppHandle) {
    app.exit(0);
}

/// The helpers every script gets, then the script itself, then exit.
const PRELUDE: &str = r#"
const invoke = window.__TAURI_INTERNALS__.invoke;
const log = (m) => invoke('dev_log', { msg: String(m) });
const shot = (name) => invoke('dev_snapshot', { name: String(name) });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const $ = (selector) => document.querySelector(selector);
const byText = (text, selector = 'button, a, [role=tab], [role=button]') =>
  [...document.querySelectorAll(selector)].find((el) => (el.getAttribute('aria-label') || el.textContent || '').trim().includes(text));
const waitFor = async (find, ms = 5000) => {
  const end = Date.now() + ms;
  for (;;) {
    const found = find();
    if (found) return found;
    if (Date.now() > end) throw new Error('waitFor timed out: ' + find);
    await sleep(50);
  }
};
const resolve = (target) => (typeof target === 'string' ? waitFor(() => $(target) || byText(target)) : target);
const click = async (target) => { (await resolve(target)).click(); await sleep(150); };
const type = async (target, text) => {
  const el = await resolve(target);
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, text);
  el.dispatchEvent(new Event('input', { bubbles: true }));
  await sleep(100);
};
window.addEventListener('error', (e) => log('page error: ' + e.message));
window.addEventListener('unhandledrejection', (e) => log('unhandled rejection: ' + ((e.reason && e.reason.message) || e.reason)));
document.addEventListener('securitypolicyviolation', (e) => log('CSP: ' + e.violatedDirective + ' ' + e.blockedURI));
"#;

/// Run the script named by `JOBPILOT_UI_SCRIPT` in `window` (call once, when the page has loaded).
pub fn run_script(window: &WebviewWindow) {
    let Some(path) = std::env::var_os(SCRIPT_ENV) else { return };
    let script = match std::fs::read_to_string(&path) {
        Ok(script) => script,
        Err(e) => {
            tracing::error!("reading the UI script failed: {e}");
            return;
        }
    };
    let wrapped = format!(
        "(async () => {{ {PRELUDE}\n try {{ await sleep(300);\n{script}\n }} catch (e) {{ await log('SCRIPT ERROR: ' + (e && e.stack || e)); }} finally {{ await invoke('dev_exit'); }} }})();"
    );
    if let Err(e) = window.eval(wrapped) {
        tracing::error!("starting the UI script failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::safe_name;

    #[test]
    fn a_shot_name_cannot_leave_the_shots_dir() {
        assert_eq!(safe_name("today-dark"), "today-dark");
        assert_eq!(safe_name("../../etc/passwd"), "______etc_passwd");
        assert_eq!(safe_name(""), "shot");
    }
}
