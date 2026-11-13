//! Rendering pages for job discovery, and the visible window for a challenge hand-off.
//!
//! - [`Renderer::render`]: a page in a headless browser, either with a throwaway profile or in the
//!   profile of a managed session (the source's bound session: its cookies are what a login or a
//!   solved check left behind). The result says whether a login, captcha or block page was shown
//!   instead ([`challenge`](super::challenge)); nothing here tries to get past one.
//! - [`open_visible`]: a **visible** browser on a page, with a throwaway profile, for a source
//!   without a session. The user looks at the page, or deals with it, themselves.
//!
//! These browsers **do not disguise themselves**: they are started without the switch that hides
//! automation and without changing the user agent (`HeadlessChrome` stays what it is), unlike the
//! managed browsers of logins and the apply flow (see [`Flavor`](super::session::Flavor)).
//!
//! **Where they may go.** Only http(s) addresses are opened, and every request the browser makes,
//! for the page, its redirects and everything the page loads, goes through the request guard
//! ([`guard`](super::guard)), which refuses machine-local and private-network hosts by the rules of
//! `net::http` and resolves names itself. A refused request makes the render fail (or, for a
//! subresource, is counted and logged) and never reaches the private host.
//!
//! **One use of a session at a time.** Reads of one session, and the window opened for the user, are
//! ordered by [`gate`](super::gate), shared by the whole app: a read that finds the window open
//! fails with [`SessionInUse`](super::gate::SessionInUse) instead of an opaque "profile in use".

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::Browser;
use futures::StreamExt;
use tokio::task::JoinHandle;

use super::chrome;
use super::fetch::{finish_page, Rendered, TempProfile, LOAD_TIMEOUT, RENDER_TIMEOUT};
use super::gate::{self, SessionInUse, SessionUse};
use super::guard::{Guard, Policy};
use super::session::{browser_config_for, prepare_profile, wait_loaded, Flavor, SessionManager};
use crate::net::http::public_http_url;
use crate::{Core, Paths};

/// What a render says when the guard refused the page.
const BLOCKED: &str = "the page is on this machine or a private network (the request was refused)";

const NO_CHROME: &str = "Chrome not found: install Chrome/Chromium or download the browser in Settings > Browser sessions";

/// Renders pages for discovery. Cheap to clone.
#[derive(Clone)]
pub struct Renderer {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    sessions: SessionManager,
    policy: Policy,
}

impl Renderer {
    /// A renderer over the app's browser sessions. `allow_private_hosts` is for tests (fixture pages
    /// are on `127.0.0.1`).
    pub fn new(core: &Core, allow_private_hosts: bool) -> Self {
        Self::with_policy(core, Policy::new(allow_private_hosts))
    }

    pub(crate) fn with_policy(core: &Core, policy: Policy) -> Self {
        Self { inner: Arc::new(Inner { paths: core.paths.clone(), sessions: core.sessions(), policy }) }
    }

    /// Render `url` headless: in session `session_id`'s profile when there is one, else with a
    /// throwaway profile. `Err` carrying [`SessionInUse`] when the session's window is open or its
    /// profile stays busy. The error text never contains `url`.
    pub async fn render(&self, session_id: Option<i64>, url: &str) -> Result<Rendered> {
        let url = public_http_url(url, self.inner.policy.allows_private_addresses())?.to_string();
        let guard = Guard::start(self.inner.policy.clone()).await?;
        let rendered = match session_id {
            None => self.render_plain(&guard, &url).await,
            Some(id) => {
                let dir = self.inner.paths.session_dir(id);
                let in_use = gate::read(&dir).await.map_err(anyhow::Error::new)?;
                self.render_in_session(&guard, id, &url, in_use).await
            }
        };
        let blocked = guard.blocked();
        if blocked > 0 {
            tracing::warn!(blocked, "the request guard refused requests while a page was rendered");
        }
        let rendered = match rendered {
            Ok(rendered) => check_final(rendered, self.inner.policy.allows_private_addresses(), blocked),
            // The browser reports a refused page as a tunnel or connection error: say what happened.
            Err(_) if blocked > 0 => Err(anyhow!(BLOCKED)),
            Err(e) => Err(e),
        };
        rendered.map_err(|e| scrub(e, &url))
    }

    async fn render_plain(&self, guard: &Guard, url: &str) -> Result<Rendered> {
        let chrome = chrome::resolve(&self.inner.paths).context(NO_CHROME)?;
        let profile = TempProfile::create(&self.inner.paths)?;
        prepare_profile(profile.path())?;
        let args = guard.chrome_args();
        let config = browser_config_for(&chrome.path, profile.path(), true, None, Flavor::Reading(&args))?;
        tokio::time::timeout(RENDER_TIMEOUT, async {
            let (mut browser, mut handler) =
                Browser::launch(config).await.with_context(|| format!("launching {}", chrome.path.display()))?;
            let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
            let result = async {
                let page = browser.new_page("about:blank").await.context("opening a tab")?;
                page.goto(url).await.context("navigating to the page")?;
                wait_loaded(&page, LOAD_TIMEOUT).await;
                finish_page(&page, url).await
            }
            .await;
            let _ = browser.close().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), browser.wait()).await;
            let _ = browser.kill().await;
            driver.abort();
            result
        })
        .await
        .unwrap_or_else(|_| Err(timed_out()))
    }

    /// `_in_use` is held until the browser is closed.
    async fn render_in_session(&self, guard: &Guard, id: i64, url: &str, _in_use: SessionUse) -> Result<Rendered> {
        let handle = self.inner.sessions.launch_reading(id, true, &guard.chrome_args()).await?;
        let result = tokio::time::timeout(RENDER_TIMEOUT, async {
            let page = handle.new_page_with_timeout(url, LOAD_TIMEOUT).await?;
            let rendered = finish_page(&page, url).await;
            let _ = page.close().await;
            rendered
        })
        .await
        .unwrap_or_else(|_| Err(timed_out()));
        let _ = handle.close().await;
        result
    }
}

fn timed_out() -> anyhow::Error {
    anyhow!("the browser did not render the page within {} seconds (timed out)", RENDER_TIMEOUT.as_secs())
}

/// The browser follows redirects itself: whatever address it ended on must be a public web page
/// too. The guard refuses the hops to private hosts, so this is the second line.
fn check_final(rendered: Rendered, allow_private: bool, blocked: usize) -> Result<Rendered> {
    if public_http_url(&rendered.url, allow_private).is_ok() {
        return Ok(rendered);
    }
    if blocked > 0 {
        bail!(BLOCKED);
    }
    bail!("the page ended on an address that is not a public web page")
}

/// `e` as one message without the page's address, and with Chrome's network errors worded so the
/// app classifies them as network problems (Retry). A [`SessionInUse`] is kept as it is.
fn scrub(e: anyhow::Error, url: &str) -> anyhow::Error {
    if e.downcast_ref::<SessionInUse>().is_some() {
        return e;
    }
    let mut text = format!("{e:#}").replace(url, "<page>");
    if let Some(parsed) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)) {
        text = text.replace(&parsed, "<host>");
    }
    // Names and connections are the guard's now: a name that does not resolve or a host that does not
    // answer reaches the browser as a tunnel / proxy error.
    const NETWORK: [&str; 8] = [
        "CONNECTION_",
        "NAME_NOT_RESOLVED",
        "TIMED_OUT",
        "INTERNET_DISCONNECTED",
        "NETWORK_",
        "ADDRESS_UNREACHABLE",
        "TUNNEL_CONNECTION_FAILED",
        "PROXY_CONNECTION_FAILED",
    ];
    if text.contains("net::ERR_") && NETWORK.iter().any(|c| text.contains(&format!("net::ERR_{c}"))) {
        text = format!("network error: {text}");
    }
    anyhow!("{text}")
}

/// A visible browser window on a throwaway profile, for the user to look at a page. Closing it (or
/// dropping it) ends the browser and deletes the profile.
pub struct Visible {
    browser: Browser,
    driver: JoinHandle<()>,
    _profile: TempProfile,
    /// The request guard the window's requests go through; it outlives the browser.
    _guard: Guard,
}

/// Open a visible browser on `url` (a public http(s) address; with `allow_private` any, for tests).
/// Its requests go through the request guard, and it is not disguised (see the module docs).
pub async fn open_visible(paths: &Paths, url: &str, allow_private: bool) -> Result<Visible> {
    let policy = Policy::new(allow_private);
    let url = public_http_url(url, policy.allows_private_addresses())?.to_string();
    let chrome = chrome::resolve(paths).context(NO_CHROME)?;
    let guard = Guard::start(policy).await?;
    let profile = TempProfile::create(paths)?;
    prepare_profile(profile.path())?;
    let args = guard.chrome_args();
    let config = browser_config_for(&chrome.path, profile.path(), false, Some(&url), Flavor::Reading(&args))?;
    let (browser, mut handler) =
        Browser::launch(config).await.with_context(|| format!("launching {}", chrome.path.display()))?;
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok(Visible { browser, driver, _profile: profile, _guard: guard })
}

impl Visible {
    /// Is the window still open? `false` once the user closed it.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.browser.try_wait(), Ok(None))
    }

    /// Close the window.
    pub async fn close(mut self) {
        let _ = self.browser.close().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.browser.wait()).await;
        let _ = self.browser.kill().await;
        self.driver.abort();
    }
}

impl Drop for Visible {
    fn drop(&mut self) {
        self.driver.abort();
        // The browser (kill on drop) goes with it, then the guard.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::nodes::ChallengeKind;

    fn rendered(url: &str) -> Rendered {
        Rendered { url: url.into(), html: "<html></html>".into(), challenge: None::<ChallengeKind> }
    }

    #[test]
    fn errors_lose_the_page_address_and_chrome_network_errors_become_network_problems() {
        let e = anyhow!("navigating to https://jobs.acme.test/careers").context("opening the page");
        let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
        assert!(!text.contains("acme"), "{text}");
        for net in ["net::ERR_CONNECTION_REFUSED", "net::ERR_TUNNEL_CONNECTION_FAILED", "net::ERR_NAME_NOT_RESOLVED"] {
            let e = anyhow!("{net} at https://jobs.acme.test/careers");
            let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
            assert_eq!(crate::health::classify(&text), crate::health::Problem::Network, "{text}");
        }
        let e = anyhow!("Chrome not found: install Chrome/Chromium");
        assert_eq!(crate::health::classify(&format!("{:#}", scrub(e, "https://a.test/"))), crate::health::Problem::NoChrome);
        let e = anyhow!("net::ERR_CERT_AUTHORITY_INVALID");
        assert_eq!(crate::health::classify(&format!("{:#}", scrub(e, "https://a.test/"))), crate::health::Problem::Other);
    }

    #[test]
    fn a_session_in_use_keeps_its_type_through_the_scrub() {
        let busy = scrub(anyhow::Error::new(SessionInUse::Window), "https://a.test/");
        assert_eq!(busy.downcast_ref::<SessionInUse>(), Some(&SessionInUse::Window));
        assert!(format!("{busy:#}").contains("in use"));
    }

    #[test]
    fn the_address_the_browser_ended_on_must_be_a_public_web_page() {
        // The page itself, or a redirect to another public site.
        assert!(check_final(rendered("https://acme.test/careers"), false, 0).is_ok());
        assert!(check_final(rendered("https://careers.other.test/x"), false, 0).is_ok());
        // A redirect that ended on this machine or the local network.
        for bad in ["http://127.0.0.1:8080/admin", "http://localhost/", "http://192.168.1.1/", "http://169.254.169.254/latest/meta-data", "http://[::1]/"] {
            let err = check_final(rendered(bad), false, 0).unwrap_err().to_string();
            assert!(err.contains("not a public web page"), "{bad}: {err}");
        }
        // The browser's own error page: with a refusal counted, the render says why.
        let err = check_final(rendered("chrome-error://chromewebdata/"), false, 1).unwrap_err().to_string();
        assert!(err.contains("private network"), "{err}");
        assert!(check_final(rendered("chrome-error://chromewebdata/"), false, 0).is_err());
        // Tests with fixture pages on 127.0.0.1 may allow it.
        assert!(check_final(rendered("http://127.0.0.1:8080/x"), true, 0).is_ok());
    }

    #[tokio::test]
    async fn a_private_address_is_refused_before_any_browser_starts() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
        let renderer = Renderer::new(&core, false);
        for url in ["http://127.0.0.1:9/x", "http://localhost/", "http://10.0.0.1/", "file:///etc/passwd", "ftp://a.test/"] {
            let err = renderer.render(None, url).await.unwrap_err().to_string();
            assert!(!err.contains("Chrome"), "{url}: refused before launching a browser: {err}");
        }
        let err = open_visible(&core.paths, "http://127.0.0.1:9/x", false).await.err().expect("refused").to_string();
        assert!(err.contains("private network"), "{err}");
        assert!(open_visible(&core.paths, "javascript:alert(1)", false).await.is_err());
    }

    #[tokio::test]
    async fn a_session_whose_window_is_open_is_not_read_it_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
        let session = core.sessions().create("Acme", "https://acme.test/login").await.unwrap();
        let renderer = Renderer::new(&core, true);
        let window = gate::window(&core.paths.session_dir(session.id)).await.unwrap();
        let err = renderer.render(Some(session.id), "https://acme.test/careers").await.unwrap_err();
        assert_eq!(err.downcast_ref::<SessionInUse>(), Some(&SessionInUse::Window), "{err:#}");
        drop(window);
    }

    // ------------------------------------------------------------ Chrome (ignored: they start a real browser)

    use crate::net::fixtures::Fixtures;

    /// A page that writes what a script can tell about the browser into its text.
    fn probe_page() -> String {
        "<html><head><title>Probe</title></head><body><div id=out>starting</div><script>
        document.getElementById('out').innerText = 'webdriver=' + navigator.webdriver + ' ua=' + navigator.userAgent +
          ' ' + 'This probe page prints what it sees so that the render has enough readable text to settle. '.repeat(8);
        </script></body></html>"
            .to_string()
    }

    #[test]
    #[ignore = "launches Chrome"]
    fn discovery_browsers_do_not_hide_that_they_are_automated() {
        crate::runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
            let fx = Fixtures::start().await;
            fx.route("/probe", 200, "text/html", probe_page());
            let url = format!("{}/probe", fx.base());
            let renderer = Renderer::new(&core, true);

            // A throwaway profile.
            let plain = renderer.render(None, &url).await.unwrap();
            // And a session's profile (headless).
            let session = core.sessions().create("Probe", &url).await.unwrap();
            let in_session = renderer.render(Some(session.id), &url).await.unwrap();
            for (name, page) in [("throwaway", plain), ("session", in_session)] {
                assert!(page.html.contains("webdriver=true"), "{name}: the automation switch is not hidden: {}", page.html);
                assert!(page.html.contains("HeadlessChrome"), "{name}: the user agent is not rewritten: {}", page.html);
                assert_eq!(page.challenge, None, "{name}");
            }
            // The session's profile is free again afterwards.
            core.sessions().launch(session.id, true).await.unwrap().close().await.unwrap();
        });
    }

    #[test]
    #[ignore = "launches Chrome"]
    fn the_guard_stops_a_page_from_reaching_the_local_network_by_any_route() {
        crate::runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
            // `public` stands for a public site; `private` for a service on the local network.
            let (public, private) = (Fixtures::start().await, Fixtures::start().await);
            let port = |fx: &Fixtures| url::Url::parse(fx.base()).unwrap().port().unwrap();
            private.route("/secret", 200, "text/plain", "the admin console");
            private.route("/frame", 200, "text/html", "<p>private frame</p>");
            private.route("/pixel", 200, "image/gif", "GIF89a");
            private.route("/script.js", 200, "text/javascript", "window.stolen = 'yes'");
            public.redirect("/go", 302, &format!("{}/secret", private.base()));
            let page = format!(
                "<html><head><title>Jobs</title></head><body><div id=out>waiting</div>
                 <img src='{p}/pixel'><iframe src='{p}/frame'></iframe><script src='{p}/script.js'></script>
                 <script>
                   fetch('{p}/secret').then(r => r.text()).then(t => out('fetched ' + t)).catch(() => out('blocked'));
                   function out(s) {{ document.getElementById('out').innerText = s + ' ' + 'Careers at Acme: we are hiring engineers for many teams. '.repeat(10); }}
                 </script></body></html>",
                p = private.base()
            );
            public.route("/jobs", 200, "text/html", page);
            let renderer = Renderer::with_policy(&core, Policy::OnlyPorts(vec![port(&public)]));

            // The page itself is served, and nothing it asks for on the "private" server gets there.
            let rendered = renderer.render(None, &format!("{}/jobs", public.base())).await.unwrap();
            assert!(rendered.html.contains("blocked"), "{}", rendered.html);
            assert!(!rendered.html.contains("admin console") && !rendered.html.contains("yes"), "{}", rendered.html);
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(private.hits().is_empty(), "the private server saw {:?}", private.hits());

            // A redirect to it fails the render and is not followed.
            let err = renderer.render(None, &format!("{}/go", public.base())).await.unwrap_err().to_string();
            assert!(err.contains("private network"), "{err}");
            assert!(private.hits().is_empty(), "the redirect was followed: {:?}", private.hits());

            // The same through a session's profile.
            let session = core.sessions().create("Guarded", &format!("{}/jobs", public.base())).await.unwrap();
            let in_session = renderer.render(Some(session.id), &format!("{}/jobs", public.base())).await.unwrap();
            assert!(in_session.html.contains("blocked"));
            assert!(renderer.render(Some(session.id), &format!("{}/go", public.base())).await.is_err());
            assert!(private.hits().is_empty(), "{:?}", private.hits());
        });
    }
}
