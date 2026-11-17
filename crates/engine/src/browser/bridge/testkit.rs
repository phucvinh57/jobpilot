//! A real Chromium with the built JobPilot extension, connected to a bridge on an ephemeral port
//! (tests only: the crate's own tests, and integration tests under `tests/` built with the cargo
//! feature `test-browser`; the tests using it are `#[ignore]`d).
//!
//! It builds nothing: the extension must already be built at `<repo>/extension/dist`
//! (`bun run ext:build`). The browser is `JOBPILOT_CHROME` or the first Playwright Chromium under
//! `/opt/pw-browsers` (branded Google Chrome ignores `--load-extension`). It runs headless, with
//! `--no-sandbox` when `JOBPILOT_NO_SANDBOX` is set, and maps every `*.test` name to `127.0.0.1`
//! (fixture servers under a name the extension's literal-address rule does not block).
//!
//! [`launch`] leaves the extension out of Incognito, as an unpacked extension is until the user says
//! otherwise; [`launch_incognito`] seeds the profile's preferences with "Allow in Incognito" (what the
//! toggle on chrome://extensions stores), so `Profile::Private` tabs work. [`TestBrowser::attach`]
//! makes `core.browser()` return the test browser's bridge, for code that takes a `Core`.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use super::{Bridge, BridgeConfig, EXTENSION_ID};
use crate::Secrets;
#[cfg(test)]
use {super::BRIDGES, crate::Core};

/// The built extension.
pub fn extension_dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extension/dist")
}

/// The Chromium to run, if any.
pub fn chromium() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("JOBPILOT_CHROME") {
        return Some(PathBuf::from(path));
    }
    let mut found: Vec<PathBuf> = std::fs::read_dir("/opt/pw-browsers")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("chromium-")))
        .flat_map(|dir| [dir.join("chrome-linux/chrome"), dir.join("chrome-linux64/chrome")])
        .filter(|p| p.is_file())
        .collect();
    found.sort();
    found.pop()
}

/// A browser with the extension, paired with [`bridge`](Self::bridge). Killed when dropped.
pub struct TestBrowser {
    pub bridge: Bridge,
    child: tokio::process::Child,
    _dirs: tempfile::TempDir,
}

impl TestBrowser {
    /// From now on `core.browser()` (for every `Core` on the same data dir) is this browser's bridge.
    #[cfg(test)]
    pub(crate) fn attach(&self, core: &Core) {
        let mut map = BRIDGES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.insert(core.paths.data_dir.clone(), self.bridge.clone());
    }
}

impl Drop for TestBrowser {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

/// [`launch_with`] on a new bridge, with the extension not allowed in Incognito (as after a plain
/// install).
pub async fn launch() -> Option<TestBrowser> {
    launch_with(Bridge::new(Secrets::memory()), false).await
}

/// [`launch`] with the extension allowed in Incognito (`Profile::Private` tabs work).
pub async fn launch_incognito() -> Option<TestBrowser> {
    launch_with(Bridge::new(Secrets::memory()), true).await
}

/// Start `bridge` (auto-approving, on a free port, with the extension consenting by itself; pass `core.browser()` to drive the browser through
/// a `Core`), launch Chromium with the extension and wait until it is connected. `incognito`: the
/// extension is allowed in Incognito, as if the user had turned that on (private tabs work).
/// `None` (with a message) when the extension is not built or no Chromium is found.
pub async fn launch_with(bridge: Bridge, incognito: bool) -> Option<TestBrowser> {
    let dist = extension_dist();
    if !dist.join("manifest.json").is_file() {
        eprintln!("skipped: the extension is not built ({} has no manifest.json; run `bun run ext:build`)", dist.display());
        return None;
    }
    let Some(chrome) = chromium() else {
        eprintln!("skipped: no Chromium (set JOBPILOT_CHROME; branded Google Chrome ignores --load-extension)");
        return None;
    };

    let config = BridgeConfig { port: 0, auto_approve: true, allowed_origin: format!("chrome-extension://{EXTENSION_ID}") };
    let port = bridge.start_with(config).await.expect("start the bridge");

    let dirs = tempfile::tempdir().expect("a temp dir");
    let ext = dirs.path().join("extension");
    copy_dir(&dist, &ext).expect("copy the extension");
    // `autoConsent` (tests only, honoured only from the packaged config.json): the extension says
    // Pair to the code itself, as the user would in its popup; the bridge approves on the app side.
    let config = serde_json::json!({ "port": port, "autoConsent": true });
    std::fs::write(ext.join("config.json"), config.to_string()).expect("write config.json");
    let profile = dirs.path().join("profile");
    if incognito {
        // What the "Allow in Incognito" switch on chrome://extensions stores.
        let prefs = serde_json::json!({ "extensions": { "settings": { EXTENSION_ID: { "incognito": true } } } });
        std::fs::create_dir_all(profile.join("Default")).expect("create the profile");
        std::fs::write(profile.join("Default/Preferences"), prefs.to_string()).expect("write the preferences");
    }

    let mut cmd = tokio::process::Command::new(&chrome);
    cmd.arg("--headless=new")
        .arg(format!("--load-extension={}", ext.display()))
        .arg(format!("--disable-extensions-except={}", ext.display()))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args(["--no-first-run", "--no-default-browser-check", "--disable-gpu", "--host-resolver-rules=MAP *.test 127.0.0.1"])
        .arg("about:blank")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if std::env::var_os("JOBPILOT_TEST_CHROME_LOG").is_some() { Stdio::inherit() } else { Stdio::null() })
        .kill_on_drop(true);
    if std::env::var_os("JOBPILOT_NO_SANDBOX").is_some() {
        cmd.arg("--no-sandbox");
    }
    let child = cmd.spawn().unwrap_or_else(|e| panic!("start {}: {e}", chrome.display()));
    let browser = TestBrowser { bridge, child, _dirs: dirs };

    for _ in 0..300 {
        let status = browser.bridge.status();
        if status.connected {
            assert_eq!(status.incognito_allowed, incognito, "the extension's Incognito access: {status:?}");
            return Some(browser);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the extension did not connect within 30 s (status: {:?})", browser.bridge.status());
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
