//! Sessions: the sites the user is signed in to in their own Chrome (LinkedIn, an ATS account).
//!
//! JobPilot keeps no browser profile and never touches the user's cookies: a session is a row (name,
//! start URL, the cookie domains the last check found, `new`/`ok`/`expired`). Signing in happens in a
//! tab of the user's browser that JobPilot opens ([`SessionManager::open_login`]); the check
//! ([`SessionManager::check`]) opens the start page in a background tab and looks whether the browser
//! holds cookies for the site and the page is not a login page ([`probe`]). Apply picks the session
//! that covers an ATS (`apply::pick_session`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};

use super::bridge::Bridge;
use super::probe;
use super::tab::{OpenOptions, Profile, Tab};
use crate::store::{NewSession, Session};
use crate::{Core, Paths, Secrets, Store};

/// Login state of a session as stored in the `sessions.status` column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionStatus {
    /// Never checked (or checked before the browser was the user's own).
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

    /// Cookie domains saved by the last check.
    pub fn domains(&self) -> Vec<String> {
        serde_json::from_str(&self.domains_json).unwrap_or_default()
    }
}

/// The sessions and their login tabs. Cheap to clone; get it with `core.sessions()`.
#[derive(Clone)]
pub struct SessionManager {
    inner: Arc<ManagerInner>,
}

struct ManagerInner {
    store: Store,
    /// Which bridge (`Core::browser`) to use: looked up on every call, so a test browser attached
    /// later is the one used.
    data_dir: PathBuf,
    secrets: Secrets,
    /// The login tab of each session, until `finish_login`.
    logins: Mutex<HashMap<i64, Tab>>,
}

impl SessionManager {
    pub(crate) fn new(paths: &Paths, store: Store, secrets: Secrets) -> Self {
        let inner = ManagerInner { store, data_dir: paths.data_dir.clone(), secrets, logins: Mutex::default() };
        Self { inner: Arc::new(inner) }
    }

    fn bridge(&self) -> Bridge {
        Bridge::for_data_dir(&self.inner.data_dir, &self.inner.secrets)
    }

    /// All sessions, oldest first.
    pub fn list(&self) -> Result<Vec<Session>> {
        self.inner.store.list_sessions()
    }

    pub fn get(&self, id: i64) -> Result<Session> {
        self.inner.store.get_session(id)?.with_context(|| format!("session {id} not found"))
    }

    /// Is a login tab open for this session (until `finish_login`, or the user closed it)?
    pub fn login_in_progress(&self, id: i64) -> bool {
        self.logins().get(&id).is_some_and(Tab::is_open)
    }

    fn logins(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Tab>> {
        self.inner.logins.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Add a session (a row only). `start_url` must be http(s).
    pub fn create(&self, name: &str, start_url: &str) -> Result<Session> {
        let name = name.trim();
        if name.is_empty() {
            bail!("give the site a name");
        }
        let start_url = normalize_start_url(start_url)?;
        let session = self.inner.store.add_session(&NewSession { name: name.to_string(), start_url })?;
        tracing::info!("session {} added", session.id);
        Ok(session)
    }

    /// Open the session's start URL in a focused tab of the user's browser so they can sign in. A
    /// login tab that is still open is brought to the front instead.
    pub async fn open_login(&self, id: i64) -> Result<()> {
        let open = self.logins().get(&id).filter(|t| t.is_open()).cloned();
        if let Some(tab) = open {
            if tab.focus().await.is_ok() {
                return Ok(());
            }
        }
        let session = self.get(id)?;
        let tab = self
            .bridge()
            .open(&session.start_url, OpenOptions { profile: Profile::Signed, focus: true, guard: None })
            .await?;
        tracing::info!("session {id}: login tab {} opened", tab.id());
        self.logins().insert(id, tab);
        Ok(())
    }

    /// The user is done signing in: close the login tab (unless they already did), then check.
    pub async fn finish_login(&self, id: i64) -> Result<SessionStatus> {
        let tab = self.logins().remove(&id);
        if let Some(tab) = tab {
            tab.close().await;
        }
        self.check(id).await
    }

    /// Open the start page in a background tab of the user's browser and judge whether they are
    /// signed in; stores and returns `Ok` or `Expired` (and the cookie domains). Fails, leaving the
    /// stored status alone, when the browser or the page cannot be reached.
    pub async fn check(&self, id: i64) -> Result<SessionStatus> {
        let session = self.get(id)?;
        let result = probe::probe(&self.bridge(), &session.start_url).await?;
        let status = if result.logged_in() { SessionStatus::Ok } else { SessionStatus::Expired };
        tracing::info!(
            "session {id} check: {} (cookie domains={}, login_url={}, password_input={})",
            status.as_str(),
            result.domains.len(),
            result.login_url,
            result.password_input
        );
        self.inner.store.update_session_domains(id, &serde_json::to_string(&result.domains)?)?;
        self.inner.store.update_session_status(id, status.as_str(), Some(chrono::Utc::now()))?;
        Ok(status)
    }

    /// Remove the session's row. The user stays signed in to the site in their browser, and a login
    /// tab that is still open stays open (it is the user's now).
    pub fn delete(&self, id: i64) -> Result<()> {
        self.logins().remove(&id);
        self.inner.store.delete_session(id)?;
        tracing::info!("session {id} deleted");
        Ok(())
    }
}

/// Accept only http(s) URLs.
fn normalize_start_url(raw: &str) -> Result<String> {
    let raw = raw.trim();
    let candidate = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = url::Url::parse(&candidate).with_context(|| format!("{raw:?} is not a valid URL"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("the start URL must be http(s)");
    }
    Ok(url.to_string())
}

impl Core {
    /// Delete `<data_dir>/sessions/`, the Chrome profiles of releases that ran their own browser
    /// (cookies of old logins). Best effort; returns how many profiles were removed.
    pub fn remove_old_browser_profiles(&self) -> usize {
        let dir = self.paths.old_sessions_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else { return 0 };
        let count = entries.count();
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => {
                tracing::info!(count, "removed the old browser profiles");
                count
            }
            Err(e) => {
                tracing::warn!("removing the old browser profiles failed: {}", e.kind());
                0
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_url_is_validated() {
        assert_eq!(normalize_start_url("https://www.linkedin.com/login").unwrap(), "https://www.linkedin.com/login");
        assert_eq!(normalize_start_url(" linkedin.com/login ").unwrap(), "https://linkedin.com/login");
        assert!(normalize_start_url("file:///etc/passwd").is_err());
        assert!(normalize_start_url("javascript:alert(1)").is_err());
        assert!(normalize_start_url("").is_err());
    }

    #[test]
    fn status_round_trips() {
        for s in [SessionStatus::New, SessionStatus::Ok, SessionStatus::Expired] {
            assert_eq!(SessionStatus::parse(s.as_str()), s);
        }
        assert_eq!(SessionStatus::parse("garbage"), SessionStatus::New);
    }

    #[test]
    fn create_validates_and_delete_removes_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        let mgr = core.sessions();
        assert!(mgr.create("  ", "https://example.com").is_err());
        assert!(mgr.create("x", "ftp://example.com").is_err());
        let s = mgr.create(" LinkedIn ", "linkedin.com/login").unwrap();
        assert_eq!((s.name.as_str(), s.start_url.as_str(), s.session_status()), ("LinkedIn", "https://linkedin.com/login", SessionStatus::New));
        assert!(!mgr.login_in_progress(s.id));
        mgr.delete(s.id).unwrap();
        assert!(mgr.get(s.id).is_err());
    }

    #[test]
    fn the_old_browser_profiles_are_removed() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        assert_eq!(core.remove_old_browser_profiles(), 0, "nothing to remove");
        let old = core.paths.old_sessions_dir();
        std::fs::create_dir_all(old.join("1/Default")).unwrap();
        std::fs::write(old.join("1/Default/Cookies"), "x").unwrap();
        std::fs::create_dir_all(old.join("2")).unwrap();
        assert_eq!(core.remove_old_browser_profiles(), 2);
        assert!(!old.exists());
    }

    /// Sessions in a real Chromium with the extension (`testkit`): the check, the login flow and
    /// delete. `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine -- --ignored sessions_` after
    /// `bun run ext:build`. `*.test` names resolve to the local fixture server.
    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn sessions_check_login_and_delete_in_the_users_browser() {
        use crate::browser::bridge::testkit;
        use crate::net::fixtures::Fixtures;

        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        let Some(browser) = testkit::launch().await else { return };
        browser.attach(&core);
        let mgr = core.sessions();

        let fx = Fixtures::start().await;
        let port = url::Url::parse(fx.base()).unwrap().port().unwrap();
        let signed_in = "<title>Feed</title><script>document.cookie = 'sid=abc; path=/; max-age=3600'</script><h1>welcome back</h1>";
        fx.route("/", 200, "text/html", signed_in);
        fx.route("/login", 200, "text/html", "<script>document.cookie = 'sid=abc; path=/; max-age=3600'</script><form><input name=u><input type=password name=p></form>");
        fx.route("/plain", 200, "text/html", "<h1>no cookie here</h1>");
        fx.redirect("/go", 302, "/login");

        // Cookies for the site and no login page: ok, and the cookie domains are saved.
        let ok = mgr.create("home", &format!("http://home.test:{port}/")).unwrap();
        assert_eq!(mgr.check(ok.id).await.unwrap(), SessionStatus::Ok);
        let row = mgr.get(ok.id).unwrap();
        assert_eq!((row.session_status(), row.domains()), (SessionStatus::Ok, vec!["home.test".to_string()]));
        assert!(row.last_verified_at.is_some());

        // A login form (cookies or not), a redirect to a login URL, or no cookies: expired.
        let login = mgr.create("login", &format!("http://login.test:{port}/login")).unwrap();
        assert_eq!(mgr.check(login.id).await.unwrap(), SessionStatus::Expired);
        let redirected = mgr.create("go", &format!("http://go.test:{port}/go")).unwrap();
        assert_eq!(mgr.check(redirected.id).await.unwrap(), SessionStatus::Expired);
        let plain = mgr.create("plain", &format!("http://plain.test:{port}/plain")).unwrap();
        assert_eq!(mgr.check(plain.id).await.unwrap(), SessionStatus::Expired);
        assert!(mgr.get(plain.id).unwrap().domains().is_empty());

        // Login flow: a focused tab, opened once; Done closes it and checks.
        let site = mgr.create("site", &format!("http://site.test:{port}/")).unwrap();
        mgr.open_login(site.id).await.unwrap();
        assert!(mgr.login_in_progress(site.id));
        let tab = mgr.logins().get(&site.id).cloned().unwrap();
        mgr.open_login(site.id).await.unwrap();
        assert_eq!(mgr.logins().get(&site.id).map(Tab::id), Some(tab.id()), "the open login tab is reused");
        tab.wait_loaded(std::time::Duration::from_secs(20)).await;
        assert_eq!(mgr.finish_login(site.id).await.unwrap(), SessionStatus::Ok);
        assert!(!mgr.login_in_progress(site.id) && !tab.is_open());

        // The user closed the login tab before pressing Done.
        mgr.open_login(ok.id).await.unwrap();
        let tab = mgr.logins().get(&ok.id).cloned().unwrap();
        tab.close().await;
        assert!(!mgr.login_in_progress(ok.id));
        assert_eq!(mgr.finish_login(ok.id).await.unwrap(), SessionStatus::Ok);

        // Delete removes the row only.
        mgr.delete(login.id).unwrap();
        assert!(mgr.get(login.id).is_err());
        assert_eq!(mgr.list().unwrap().len(), 4);
    }
}
