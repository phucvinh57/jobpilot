//! Managed browser sessions: the only place that launches Chrome.
//!
//! A session is an app-owned Chrome profile in `sessions/<id>/` (never the user's own profile). Every
//! launch goes through [`SessionManager::launch`], which takes the per-profile lock, writes the profile
//! preferences (no password manager) and starts Chrome with the managed flag set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::cdp::browser_protocol::network::Cookie;
use chromiumoxide::error::CdpError;
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::{Browser, BrowserConfig, Page};
use futures::StreamExt;
use serde_json::{json, Value};

use super::chrome::{self, Chrome};
use super::lock::{LockError, ProfileLock};
use super::probe;
use crate::engine::store::{NewSession, Session};
use crate::engine::{Paths, Store};

/// Set to anything non-empty to launch Chrome with `--no-sandbox` (containers, running as root).
pub const NO_SANDBOX_ENV: &str = "JOBPILOT_NO_SANDBOX";

/// How long page navigation may take in [`BrowserHandle::new_page`].
pub(crate) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(30);

/// Fixed launch flags of every managed browser (no sync, no extensions, no password manager, no
/// "controlled by automated software" banner / webdriver flag).
const LAUNCH_ARGS: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-extensions",
    "--password-store=basic",
    "--use-mock-keychain",
    "--disable-features=PasswordManagerOnboarding,PasswordLeakDetection,PasswordImport,AutofillServerCommunication,TranslateUI,MediaRouter,OptimizationHints",
    "--disable-save-password-bubble",
    "--disable-background-mode",
    "--disable-breakpad",
    "--disable-component-extensions-with-background-pages",
    "--disable-default-apps",
    "--disable-dev-shm-usage",
    "--disable-hang-monitor",
    "--disable-ipc-flooding-protection",
    "--disable-prompt-on-repost",
    "--disable-background-timer-throttling",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--metrics-recording-only",
    "--force-color-profile=srgb",
    "--lang=en-US",
];

/// Login state of a session as stored in the `sessions.status` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    /// Never logged in / signed out.
    New,
    Ok,
    Expired,
}

impl SessionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionStatus::New => "new",
            SessionStatus::Ok => "ok",
            SessionStatus::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "ok" => SessionStatus::Ok,
            "expired" => SessionStatus::Expired,
            _ => SessionStatus::New,
        }
    }
}

impl Session {
    /// Parsed [`Session::status`].
    pub fn session_status(&self) -> SessionStatus {
        SessionStatus::parse(&self.status)
    }

    /// Cookie domains saved by the last login/check.
    pub fn domains(&self) -> Vec<String> {
        serde_json::from_str(&self.domains_json).unwrap_or_default()
    }
}

// ---- handle ----

/// A running managed browser. Cheap to clone (all clones share one browser); the browser is killed and
/// the profile lock released when [`close`](Self::close) is called or the last clone is dropped.
#[derive(Clone)]
pub struct BrowserHandle {
    inner: Arc<HandleInner>,
}

struct HandleInner {
    session_id: i64,
    headless: bool,
    browser: tokio::sync::Mutex<Option<Browser>>,
    handler_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    lock: Mutex<Option<ProfileLock>>,
    closed: AtomicBool,
}

impl Drop for HandleInner {
    fn drop(&mut self) {
        if let Some(task) = self.handler_task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        // `browser` (kill-on-drop) and `lock` drop after this, in field order.
    }
}

impl BrowserHandle {
    pub fn session_id(&self) -> i64 {
        self.inner.session_id
    }

    pub fn is_headless(&self) -> bool {
        self.inner.headless
    }

    /// False once [`close`](Self::close) ran (or the browser exited and was noticed).
    pub fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::SeqCst)
    }

    /// Is the Chrome process still running? `false` after the user closed the window themselves.
    pub async fn is_alive(&self) -> bool {
        let mut guard = self.inner.browser.lock().await;
        match guard.as_mut() {
            None => false,
            Some(b) => matches!(b.try_wait(), Ok(None)),
        }
    }

    /// Open `url` in a new tab and wait (up to 30 s; a slow page is returned as-is) for it to load.
    pub async fn new_page(&self, url: &str) -> Result<Page> {
        self.new_page_with_timeout(url, NAVIGATE_TIMEOUT).await
    }

    /// Like [`new_page`](Self::new_page) with an explicit load timeout.
    pub async fn new_page_with_timeout(&self, url: &str, load_timeout: Duration) -> Result<Page> {
        let guard = self.inner.browser.lock().await;
        let browser = guard.as_ref().context("the browser is closed")?;
        let page = browser.new_page("about:blank").await.context("opening a tab")?;
        drop(guard);
        page.goto(url).await.with_context(|| format!("navigating to {url}"))?;
        wait_loaded(&page, load_timeout).await;
        Ok(page)
    }

    /// The tabs currently open (e.g. the login page of a headed window).
    pub async fn pages(&self) -> Result<Vec<Page>> {
        let guard = self.inner.browser.lock().await;
        let browser = guard.as_ref().context("the browser is closed")?;
        browser.pages().await.context("listing tabs")
    }

    /// Every cookie in the profile (CDP `Storage.getCookies`).
    pub async fn cookies(&self) -> Result<Vec<Cookie>> {
        let guard = self.inner.browser.lock().await;
        let browser = guard.as_ref().context("the browser is closed")?;
        browser.get_cookies().await.context("reading cookies")
    }

    /// Sorted cookie domains of the profile.
    pub async fn domains(&self) -> Result<Vec<String>> {
        Ok(probe::cookie_domains(&self.cookies().await?))
    }

    /// Close the browser (gracefully, then forcibly after 5 s) and release the profile lock. Idempotent.
    pub async fn close(&self) -> Result<()> {
        let browser = self.inner.browser.lock().await.take();
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Some(mut browser) = browser {
            // Already gone when the user closed the window; `close` then just errors, which is fine.
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                let _ = browser.close().await;
                let _ = browser.wait().await;
            })
            .await;
            if matches!(browser.try_wait(), Ok(None)) {
                let _ = browser.kill().await;
            }
        }
        if let Some(task) = self.inner.handler_task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        self.inner.lock.lock().unwrap_or_else(|e| e.into_inner()).take();
        Ok(())
    }
}

/// Wait until `document.readyState == "complete"` (errors while the page navigates are ignored).
pub(crate) async fn wait_loaded(page: &Page, timeout: Duration) {
    let poll = async {
        loop {
            if let Ok(res) = page.evaluate_expression("document.readyState").await {
                if res.into_value::<String>().is_ok_and(|s| s == "complete") {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let _ = tokio::time::timeout(timeout, poll).await;
}

// ---- manager ----

/// Owner of every managed browser launch. Cheap to clone; get it with `core.sessions()`.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    paths: Paths,
    store: Store,
    /// Every browser launched through the manager (weak: callers own the handle they get).
    live: Mutex<HashMap<i64, Weak<HandleInner>>>,
    /// Headed login browsers the manager keeps alive until `finish_login`.
    logins: Mutex<HashMap<i64, BrowserHandle>>,
}

impl SessionManager {
    pub(crate) fn new(paths: Paths, store: Store) -> Self {
        Self { inner: Arc::new(ManagerInner { paths, store, live: Default::default(), logins: Default::default() }) }
    }

    /// The Chrome that [`launch`](Self::launch) would use (`None` = not installed).
    pub fn chrome(&self) -> Option<Chrome> {
        chrome::resolve(&self.inner.paths)
    }

    /// All sessions, oldest first.
    pub fn list(&self) -> Result<Vec<Session>> {
        self.inner.store.list_sessions()
    }

    pub fn get(&self, id: i64) -> Result<Session> {
        self.inner.store.get_session(id)?.with_context(|| format!("session {id} not found"))
    }

    /// Is a browser for this session currently running (launched through the manager)?
    pub fn is_open(&self, id: i64) -> bool {
        self.live_handle(id).is_some()
    }

    /// Is a headed login window pending `finish_login`?
    pub fn login_in_progress(&self, id: i64) -> bool {
        self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&id)
    }

    /// Add a session (row + empty private profile dir). `start_url` must be http(s).
    pub async fn create(&self, name: &str, start_url: &str) -> Result<Session> {
        let name = name.trim();
        if name.is_empty() {
            bail!("give the session a name");
        }
        let start_url = normalize_start_url(start_url)?;
        let session = self.inner.store.add_session(&NewSession { name: name.to_string(), start_url })?;
        if let Err(e) = prepare_profile(&self.inner.paths.session_dir(session.id)) {
            let _ = self.inner.store.delete_session(session.id);
            return Err(e);
        }
        Ok(session)
    }

    /// Start a **headed** browser at the session's start URL so the user can log in. The manager keeps the
    /// browser until [`finish_login`](Self::finish_login); the returned clone is optional to hold.
    pub async fn open_login(&self, id: i64) -> Result<BrowserHandle> {
        let previous = self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(previous) = previous {
            if previous.is_alive().await {
                self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).insert(id, previous.clone());
                return Ok(previous);
            }
            // The user closed the previous login window themselves: release it and start over.
            previous.close().await?;
        }
        let session = self.get(id)?;
        let handle = self.launch_inner(id, false, Some(&session.start_url), Flavor::Managed).await?;
        self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).insert(id, handle.clone());
        Ok(handle)
    }

    /// The user is done logging in: save the cookie domains, close the window (it may already have been
    /// closed by the user), then run the health check. Returns the resulting status.
    pub async fn finish_login(&self, id: i64) -> Result<SessionStatus> {
        let handle = self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(handle) = handle {
            if handle.is_alive().await {
                match handle.domains().await {
                    Ok(domains) => self.save_domains(id, &domains)?,
                    Err(e) => tracing::warn!("session {id}: could not read cookies before closing: {e:#}"),
                }
            }
            handle.close().await?;
        }
        self.check(id).await
    }

    /// Headless health check of a session; stores and returns `Ok` or `Expired`. Fails (leaving the stored
    /// status alone) when the browser or the page cannot be reached.
    pub async fn check(&self, id: i64) -> Result<SessionStatus> {
        let session = self.get(id)?;
        let handle = self.launch(id, true).await?;
        let result = probe::probe(&handle, &session.start_url).await;
        let _ = handle.close().await;
        let result = result?;

        let status = if result.logged_in() { SessionStatus::Ok } else { SessionStatus::Expired };
        tracing::info!(
            "session {id} check: {} (url={}, cookies={}, login_url={}, password_input={})",
            status.as_str(),
            result.final_url,
            result.has_cookies,
            result.login_url,
            result.password_input
        );
        self.save_domains(id, &result.domains)?;
        self.inner.store.update_session_status(id, status.as_str(), Some(chrono::Utc::now()))?;
        Ok(status)
    }

    /// Close any live browser of the session and delete its profile contents (cookies, logins); the row
    /// stays, reset to `new`.
    pub async fn sign_out(&self, id: i64) -> Result<()> {
        self.get(id)?;
        self.close_live(id).await?;
        let dir = self.inner.paths.session_dir(id);
        if dir.exists() {
            let lock = ProfileLock::acquire(&dir).map_err(lock_error)?;
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.file_name().is_some_and(|n| n == super::lock::LOCK_FILE) {
                    continue;
                }
                remove_path(&path)?;
            }
            drop(lock);
        }
        prepare_profile(&dir)?;
        self.inner.store.reset_session(id)
    }

    /// Remove the session: close its browser, delete the profile dir and the row.
    pub async fn delete(&self, id: i64) -> Result<()> {
        self.close_live(id).await?;
        let dir = self.inner.paths.session_dir(id);
        if dir.exists() {
            let _lock = ProfileLock::acquire(&dir).map_err(lock_error)?;
            remove_dir_retry(&dir)?;
        }
        self.inner.store.delete_session(id)
    }

    /// Launch Chrome on the session's profile (the single entry point for automation, e.g. auto-apply).
    /// Fails when the profile is already open, so there is at most one live browser per profile.
    /// Close it with [`BrowserHandle::close`]; dropping every clone also kills it.
    pub async fn launch(&self, id: i64, headless: bool) -> Result<BrowserHandle> {
        self.launch_inner(id, headless, None, Flavor::Managed).await
    }

    /// Like [`launch`](Self::launch) for **reading third-party pages** (job discovery): with
    /// `guard_args` (a [`Guard`](super::guard::Guard)'s flags) every request goes through the
    /// request guard. The profile and its lock are the session's, so the
    /// cookies of a login the user made are what the page is read with.
    pub(crate) async fn launch_reading(&self, id: i64, headless: bool, guard_args: &[String]) -> Result<BrowserHandle> {
        self.launch_inner(id, headless, None, Flavor::Reading(guard_args)).await
    }

    async fn launch_inner(&self, id: i64, headless: bool, start_url: Option<&str>, flavor: Flavor<'_>) -> Result<BrowserHandle> {
        self.get(id)?;
        let chrome = self.chrome().context("Chrome not found: install Chrome/Chromium or download the browser in Settings")?;
        let dir = self.inner.paths.session_dir(id);
        prepare_profile(&dir)?;
        let dir = std::fs::canonicalize(&dir).with_context(|| format!("resolving {}", dir.display()))?;
        let lock = ProfileLock::acquire(&dir).map_err(lock_error)?;

        // A headed window without a URL would open the new-tab page; keep it blank instead.
        let start_url = start_url.or((!headless).then_some("about:blank"));
        let config = browser_config_for(&chrome.path, &dir, headless, start_url, flavor)?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .with_context(|| format!("launching {}", chrome.path.display()))?;
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                match event {
                    Ok(()) => {}
                    // The socket is gone (browser exited): stop driving it.
                    Err(CdpError::Ws(_)) | Err(CdpError::Io(_)) => break,
                    // CDP messages newer than this chromiumoxide knows: ignore.
                    Err(e) => tracing::debug!("chromiumoxide: {e}"),
                }
            }
        });

        let handle = BrowserHandle {
            inner: Arc::new(HandleInner {
                session_id: id,
                headless,
                browser: tokio::sync::Mutex::new(Some(browser)),
                handler_task: Mutex::new(Some(handler_task)),
                lock: Mutex::new(Some(lock)),
                closed: AtomicBool::new(false),
            }),
        };
        self.inner.live.lock().unwrap_or_else(|e| e.into_inner()).insert(id, Arc::downgrade(&handle.inner));
        Ok(handle)
    }

    fn live_handle(&self, id: i64) -> Option<BrowserHandle> {
        let mut live = self.inner.live.lock().unwrap_or_else(|e| e.into_inner());
        match live.get(&id).and_then(Weak::upgrade) {
            Some(inner) if !inner.closed.load(Ordering::SeqCst) => Some(BrowserHandle { inner }),
            Some(_) => None,
            None => {
                live.remove(&id);
                None
            }
        }
    }

    async fn close_live(&self, id: i64) -> Result<()> {
        let login = self.inner.logins.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        if let Some(handle) = login {
            handle.close().await?;
        }
        if let Some(handle) = self.live_handle(id) {
            handle.close().await?;
        }
        Ok(())
    }

    fn save_domains(&self, id: i64, domains: &[String]) -> Result<()> {
        self.inner.store.update_session_domains(id, &serde_json::to_string(domains)?)
    }
}

fn lock_error(e: LockError) -> anyhow::Error {
    anyhow!(e)
}

/// Accept only http(s) URLs (this also keeps the value from ever being parsed as a Chrome flag).
fn normalize_start_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let candidate = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&candidate).with_context(|| format!("{raw:?} is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("the start URL must be http(s)");
    }
    Ok(url.to_string())
}

/// How a browser is started.
#[derive(Clone, Copy)]
pub(crate) enum Flavor<'a> {
    /// The managed flag set: logins and the apply flow (`LAUNCH_ARGS`).
    Managed,
    /// For reading third-party pages (job discovery): the managed flags plus these extra flags (the
    /// request guard's).
    Reading(&'a [String]),
}

pub(crate) fn browser_config(chrome: &Path, dir: &Path, headless: bool, start_url: Option<&str>) -> Result<BrowserConfig> {
    browser_config_for(chrome, dir, headless, start_url, Flavor::Managed)
}

/// The launch flags of `flavor`.
fn launch_args(flavor: Flavor<'_>) -> Vec<String> {
    let mut args: Vec<String> = LAUNCH_ARGS.iter().map(|a| a.to_string()).collect();
    if let Flavor::Reading(extra) = flavor {
        args.extend(extra.iter().cloned());
    }
    args
}

pub(crate) fn browser_config_for(chrome: &Path, dir: &Path, headless: bool, start_url: Option<&str>, flavor: Flavor<'_>) -> Result<BrowserConfig> {
    let args = launch_args(flavor);
    let mut builder = BrowserConfig::builder()
        .chrome_executable(chrome)
        .user_data_dir(dir)
        .disable_default_args()
        .args(args)
        .launch_timeout(Duration::from_secs(30));
    builder = if headless {
        builder.new_headless_mode().window_size(1280, 800).viewport(Viewport {
            width: 1280,
            height: 800,
            ..Default::default()
        })
    } else {
        // A real window: let it size itself instead of emulating an 800x600 viewport.
        builder.with_head().window_size(1280, 900).viewport(None)
    };
    if std::env::var_os(NO_SANDBOX_ENV).is_some_and(|v| !v.is_empty()) {
        builder = builder.no_sandbox();
    }
    if let Some(url) = start_url {
        builder = builder.arg(url);
    }
    builder.build().map_err(|e| anyhow!(e))
}

/// Create the profile dir (mode 0700) and make sure the password manager and autofill are off.
pub(crate) fn prepare_profile(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir.join("Default")).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    write_preferences(&dir.join("Default").join("Preferences"))
}

/// Merge our privacy preferences into `Default/Preferences` (Chrome must not be running on the profile).
fn write_preferences(path: &Path) -> Result<()> {
    let mut prefs: Value = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };
    if !prefs.is_object() {
        prefs = json!({});
    }
    let before = prefs.clone();
    for (key, value) in [
        ("credentials_enable_service", json!(false)),
        ("credentials_enable_autosignin", json!(false)),
        ("profile.password_manager_enabled", json!(false)),
        ("profile.password_manager_leak_detection", json!(false)),
        ("autofill.profile_enabled", json!(false)),
        ("autofill.credit_card_enabled", json!(false)),
        ("signin.allowed", json!(false)),
    ] {
        set_pref(&mut prefs, key, value);
    }
    if prefs != before || !path.exists() {
        std::fs::write(path, serde_json::to_vec(&prefs)?).with_context(|| format!("writing {}", path.display()))?;
    }
    Ok(())
}

/// Set a dotted key (`a.b.c`) inside nested JSON objects, creating them as needed.
fn set_pref(root: &mut Value, dotted: &str, value: Value) {
    let mut node = root;
    let mut parts = dotted.split('.').peekable();
    while let Some(part) = parts.next() {
        if !node.is_object() {
            *node = json!({});
        }
        let Some(map) = node.as_object_mut() else { return };
        if parts.peek().is_none() {
            map.insert(part.to_string(), value);
            return;
        }
        node = map.entry(part.to_string()).or_insert_with(|| json!({}));
    }
}

fn remove_path(path: &PathBuf) -> Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() {
        remove_dir_retry(path)
    } else {
        std::fs::remove_file(path).with_context(|| format!("removing {}", path.display()))
    }
}

/// `remove_dir_all` with a few retries: a just-closed Chrome may still be flushing files for a moment.
fn remove_dir_retry(dir: &Path) -> Result<()> {
    let mut last: Option<std::io::Error> = None;
    for _ in 0..5 {
        match std::fs::remove_dir_all(dir) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    match last {
        Some(e) => Err(anyhow!(e).context(format!("removing {}", dir.display()))),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_browser_is_disguised() {
        // No browser hides that it is automated (no fingerprint spoofing).
        assert!(!launch_args(Flavor::Managed).iter().any(|a| a.contains("AutomationControlled") || a.contains("user-agent")));
        // Discovery adds the request guard's flags.
        let guard = vec!["--proxy-server=http://127.0.0.1:1".to_string()];
        let reading = launch_args(Flavor::Reading(&guard));
        assert!(!reading.iter().any(|a| a.contains("AutomationControlled") || a.contains("user-agent")), "{reading:?}");
        assert!(reading.contains(&guard[0]));
        // Everything else the managed browsers have (no password manager, no sync) stays.
        for kept in ["--disable-sync", "--password-store=basic", "--no-first-run"] {
            assert!(reading.iter().any(|a| a == kept), "{kept}");
        }
        assert_eq!(reading.len(), launch_args(Flavor::Managed).len() + 1);
    }

    #[test]
    fn start_url_is_validated() {
        assert_eq!(normalize_start_url("https://www.linkedin.com/login").unwrap(), "https://www.linkedin.com/login");
        assert_eq!(normalize_start_url(" linkedin.com/login ").unwrap(), "https://linkedin.com/login");
        assert!(normalize_start_url("file:///etc/passwd").is_err());
        assert!(normalize_start_url("javascript:alert(1)").is_err());
        // Whatever the input, the result starts with a scheme, so it can never be read as a Chrome flag.
        assert!(normalize_start_url("--remote-debugging-port=1").map_or(true, |u| u.starts_with("https://")));
        assert!(normalize_start_url("").is_err());
    }

    #[test]
    fn preferences_disable_the_password_manager_and_keep_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        prepare_profile(dir.path()).unwrap();
        let path = dir.path().join("Default/Preferences");
        let prefs: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(prefs["credentials_enable_service"], json!(false));
        assert_eq!(prefs["profile"]["password_manager_enabled"], json!(false));

        // Chrome rewrote the file with its own keys and re-enabled a pref: ours win, the rest survives.
        std::fs::write(&path, r#"{"profile":{"password_manager_enabled":true,"name":"x"},"other":1}"#).unwrap();
        prepare_profile(dir.path()).unwrap();
        let prefs: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(prefs["profile"]["password_manager_enabled"], json!(false));
        assert_eq!(prefs["profile"]["name"], json!("x"));
        assert_eq!(prefs["other"], json!(1));

        // A corrupt file is replaced instead of failing the launch.
        std::fs::write(&path, "{ not json").unwrap();
        prepare_profile(dir.path()).unwrap();
        let prefs: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(prefs["credentials_enable_service"], json!(false));
    }

    #[cfg(unix)]
    #[test]
    fn profile_dir_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("7");
        prepare_profile(&profile).unwrap();
        assert_eq!(profile.metadata().unwrap().permissions().mode() & 0o777, 0o700);
    }

    #[test]
    fn status_round_trips() {
        for s in [SessionStatus::New, SessionStatus::Ok, SessionStatus::Expired] {
            assert_eq!(SessionStatus::parse(s.as_str()), s);
        }
        assert_eq!(SessionStatus::parse("garbage"), SessionStatus::New);
    }

    #[test]
    fn launch_args_never_enable_automation_or_password_manager() {
        assert!(!LAUNCH_ARGS.iter().any(|a| a.contains("enable-automation")));
        for needed in ["--no-first-run", "--no-default-browser-check", "--disable-sync", "--disable-extensions", "--password-store=basic"] {
            assert!(LAUNCH_ARGS.contains(&needed), "missing {needed}");
        }
        assert_eq!(LAUNCH_ARGS.iter().filter(|a| a.starts_with("--disable-features=")).count(), 1);
    }
}
