//! Rendering pages for job discovery, and the tab opened for a challenge hand-off. Both are tabs in
//! the user's own Chrome, driven through the JobPilot extension ([`Bridge`]).
//!
//! - [`Renderer::render`]: a page in a background tab. A source without a session is read in a
//!   **private** tab (the JobPilot Incognito window: none of the user's cookies); a source with a
//!   session is read in the user's **normal profile**, where they are signed in (a source's session
//!   means "use my signed-in browser for this site"). The result says whether a login, captcha or
//!   block page was shown instead ([`challenge`](super::challenge)); nothing here tries to get past one.
//! - [`open_visible`]: a focused tab on a page, for the user to look at or deal with themselves.
//!
//! These tabs **do not disguise themselves**: the extension leaves `navigator.webdriver` true and the
//! user agent is the browser's own. No JobPilot tab hides that it is automated.
//!
//! **Where they may go.** Only http(s) addresses are opened, and every request of the tab, for the
//! page, its redirects and everything the page loads, is checked by the guard
//! ([`guard::allows`](super::guard), answered by the app; the extension also blocks machine-local and
//! private address literals itself), which refuses machine-local and private-network hosts by the
//! rules of `net::http`. A refused page makes the render fail; a refused subresource is counted and
//! logged, and never reaches the private host.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};

use super::bridge::{Bridge, BridgeError};
use super::fetch::{render_tab, Rendered};
use super::guard::Policy;
use super::tab::{OpenOptions, Profile, Tab};
use crate::engine::net::http::public_http_url;
use crate::engine::store::Store;
use crate::engine::Core;

/// What a render says when the guard refused the page.
const BLOCKED: &str = "the page is on this machine or a private network (the request was refused)";

/// Renders pages for discovery. Cheap to clone.
#[derive(Clone)]
pub struct Renderer {
    inner: Arc<Inner>,
}

struct Inner {
    bridge: Bridge,
    store: Store,
    policy: Policy,
}

impl Renderer {
    /// A renderer over the app's browser ([`Core::browser`]). `allow_private_hosts` is for tests
    /// (fixture pages are on this machine).
    pub fn new(core: &Core, allow_private_hosts: bool) -> Self {
        Self::with_policy(core, Policy::new(allow_private_hosts))
    }

    pub(crate) fn with_policy(core: &Core, policy: Policy) -> Self {
        Self { inner: Arc::new(Inner { bridge: core.browser(), store: core.store.clone(), policy }) }
    }

    /// Render `url` in a background tab: in the user's signed-in profile when the source has a
    /// session (`session_id`, which must still exist), else in a private tab. The tab is closed
    /// afterwards. The error text never contains `url`; a browser the user has to fix (the extension
    /// is not connected, or not allowed in Incognito) is a [`BridgeError`] inside the error.
    pub async fn render(&self, session_id: Option<i64>, url: &str) -> Result<Rendered> {
        let allow_private = self.inner.policy.allows_private_addresses();
        let url = public_http_url(url, allow_private)?.to_string();
        let profile = self.profile_for(session_id)?;
        let (rendered, blocked) = render_tab(&self.inner.bridge, &url, profile, self.inner.policy.clone()).await;
        if blocked > 0 {
            tracing::warn!(blocked, "the browser guard refused requests while a page was rendered");
        }
        let rendered = match rendered {
            Ok(rendered) => check_final(rendered, allow_private, blocked),
            // The browser reports a refused page as a blocked or failed navigation: say what happened.
            Err(e) if blocked > 0 || refused_by_extension(&e) => Err(anyhow!(BLOCKED)),
            Err(e) => Err(e),
        };
        rendered.map_err(|e| scrub(e, &url))
    }

    /// The profile a source's reads use: a session means the user's signed-in profile.
    fn profile_for(&self, session_id: Option<i64>) -> Result<Profile> {
        let Some(id) = session_id else { return Ok(Profile::Private) };
        self.inner.store.get_session(id)?.with_context(|| format!("session {id} not found"))?;
        Ok(Profile::Signed)
    }
}

/// The extension's own rule refused the address (a redirect to an address literal on this machine
/// or a private network; the app's guard never sees those requests).
fn refused_by_extension(e: &anyhow::Error) -> bool {
    matches!(e.downcast_ref::<BridgeError>(), Some(BridgeError::Cdp(text)) if text.contains("ERR_BLOCKED_BY_CLIENT"))
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
/// app classifies them as network problems (Retry). A browser the user has to fix (a
/// [`BridgeError`] that is not about the page) is kept as it is, so it can be told apart.
fn scrub(e: anyhow::Error, url: &str) -> anyhow::Error {
    if let Some(kind @ (BridgeError::NotConnected | BridgeError::IncognitoNotAllowed | BridgeError::NotAllowed)) =
        e.downcast_ref::<BridgeError>()
    {
        // Its text never names a page.
        return anyhow::Error::new(kind.clone());
    }
    let mut text = format!("{e:#}").replace(url, "<page>");
    if let Some(parsed) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)) {
        text = text.replace(&parsed, "<host>");
    }
    // A name that does not resolve or a host that does not answer.
    const NETWORK: [&str; 6] = ["CONNECTION_", "NAME_NOT_RESOLVED", "TIMED_OUT", "INTERNET_DISCONNECTED", "NETWORK_", "ADDRESS_UNREACHABLE"];
    if text.contains("net::ERR_") && NETWORK.iter().any(|c| text.contains(&format!("net::ERR_{c}"))) {
        text = format!("network error: {text}");
    }
    anyhow!("{text}")
}

/// Open `url` (a public http(s) address; any address under a `policy` that allows private ones, for
/// tests) in a **focused** tab of `profile`, guarded by `policy`, for the user to look at or deal
/// with. JobPilot does nothing in it. [`Tab::closed`] resolves when the user closes it;
/// [`Tab::close`] closes it.
pub async fn open_visible(bridge: &Bridge, url: &str, profile: Profile, policy: Policy) -> Result<Tab> {
    let url = public_http_url(url, policy.allows_private_addresses())?.to_string();
    let tab = bridge.open(&url, OpenOptions { profile, focus: true, guard: Some(policy) }).await;
    tab.map_err(|e| scrub(e, &url))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::engine::graph::nodes::ChallengeKind;
    use crate::engine::store::NewSession;
    use crate::engine::{Paths, Secrets};

    fn rendered(url: &str) -> Rendered {
        Rendered { url: url.into(), html: "<html></html>".into(), challenge: None::<ChallengeKind> }
    }

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn session(core: &Core, name: &str, url: &str) -> i64 {
        core.store.add_session(&NewSession { name: name.into(), start_url: url.into() }).unwrap().id
    }

    #[test]
    fn errors_lose_the_page_address_and_chrome_network_errors_become_network_problems() {
        let e = anyhow!("navigating to https://jobs.acme.test/careers").context("opening the page");
        let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
        assert!(!text.contains("acme"), "{text}");
        for net in ["net::ERR_CONNECTION_REFUSED", "net::ERR_NAME_NOT_RESOLVED", "net::ERR_ADDRESS_UNREACHABLE"] {
            let e = anyhow::Error::new(BridgeError::Cdp(net.into())).context("navigating to the page");
            let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
            assert_eq!(crate::engine::health::classify(&text), crate::engine::health::Problem::Network, "{text}");
        }
        let e = anyhow!("net::ERR_CERT_AUTHORITY_INVALID");
        assert_eq!(crate::engine::health::classify(&format!("{:#}", scrub(e, "https://a.test/"))), crate::engine::health::Problem::Other);
    }

    #[test]
    fn a_browser_the_user_has_to_fix_keeps_its_kind_through_the_scrub() {
        for kind in [BridgeError::NotConnected, BridgeError::IncognitoNotAllowed] {
            let e = anyhow::Error::new(kind.clone()).context("opening a browser tab at https://a.test/x");
            let kept = scrub(e, "https://a.test/x");
            assert_eq!(kept.downcast_ref::<BridgeError>(), Some(&kind));
            assert!(!format!("{kept:#}").contains("a.test"), "{kept:#}");
        }
        // Chrome's refusal of the page is about the page: scrubbed like any other error.
        let e = anyhow::Error::new(BridgeError::Cdp("net::ERR_BLOCKED_BY_CLIENT at https://a.test/x".into()));
        assert!(refused_by_extension(&e));
        assert!(!format!("{:#}", scrub(e, "https://a.test/x")).contains("a.test"));
        assert!(!refused_by_extension(&anyhow::Error::new(BridgeError::Cdp("net::ERR_NAME_NOT_RESOLVED".into()))));
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
    async fn a_private_address_is_refused_before_the_browser_is_asked() {
        let (_dir, core) = core();
        let renderer = Renderer::new(&core, false);
        for url in ["http://127.0.0.1:9/x", "http://localhost/", "http://10.0.0.1/", "file:///etc/passwd", "ftp://a.test/"] {
            let err = renderer.render(None, url).await.unwrap_err();
            assert!(err.downcast_ref::<BridgeError>().is_none(), "{url}: refused before asking the browser: {err:#}");
        }
        let err = open_visible(&core.browser(), "http://127.0.0.1:9/x", Profile::Private, Policy::Public).await.unwrap_err();
        assert!(err.to_string().contains("private network"), "{err}");
        assert!(open_visible(&core.browser(), "javascript:alert(1)", Profile::Private, Policy::Public).await.is_err());
    }

    #[tokio::test]
    async fn a_source_whose_session_is_gone_is_not_read_and_a_missing_extension_says_so() {
        let (_dir, core) = core();
        let renderer = Renderer::new(&core, true);
        let err = renderer.render(Some(999), "https://acme.test/careers").await.unwrap_err();
        assert!(format!("{err:#}").contains("session 999 not found"), "{err:#}");
        // No extension is connected: the error is the bridge's, for the UI to say what to do.
        let id = session(&core, "Acme", "https://acme.test/login");
        for session_id in [None, Some(id)] {
            let err = renderer.render(session_id, "https://acme.test/careers").await.unwrap_err();
            assert_eq!(err.downcast_ref::<BridgeError>(), Some(&BridgeError::NotConnected), "{err:#}");
        }
    }

    // ------------------------------------------------------------ a real browser (ignored)
    // `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot -- --ignored render::` after `bun run ext:build`.

    use crate::engine::browser::bridge::testkit;
    use crate::engine::net::fixtures::Fixtures;

    fn port(fx: &Fixtures) -> u16 {
        url::Url::parse(fx.base()).unwrap().port().unwrap()
    }

    /// A page that writes what a script can tell about the browser into its text.
    fn probe_page() -> String {
        "<html><head><title>Probe</title></head><body><div id=out>starting</div><script>
        document.getElementById('out').innerText = 'webdriver=' + navigator.webdriver + ' ua=' + navigator.userAgent +
          ' ' + 'This probe page prints what it sees so that the render has enough readable text to settle. '.repeat(8);
        </script></body></html>"
            .to_string()
    }

    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn discovery_browsers_do_not_hide_that_they_are_automated() {
        let Some(browser) = testkit::launch_incognito().await else { return };
        let (_dir, core) = core();
        browser.attach(&core);
        let fx = Fixtures::start().await;
        fx.route("/probe", 200, "text/html", probe_page());
        // `*.test` is this machine in the test browser (the extension blocks 127.0.0.1 itself).
        let url = format!("http://jobs.test:{}/probe", port(&fx));
        let renderer = Renderer::new(&core, true);

        // A private tab, and the user's signed-in profile (a source with a session).
        let private = renderer.render(None, &url).await.unwrap();
        let id = session(&core, "Probe", &url);
        let signed = renderer.render(Some(id), &url).await.unwrap();
        for (name, page) in [("private", private), ("signed", signed)] {
            assert!(page.html.contains("webdriver=true"), "{name}: the automation switch is not hidden: {}", page.html);
            assert!(page.html.contains("HeadlessChrome"), "{name}: the user agent is not rewritten: {}", page.html);
            assert_eq!(page.challenge, None, "{name}");
        }
    }

    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn the_guard_stops_a_page_from_reaching_the_local_network_by_any_route() {
        let Some(browser) = testkit::launch_incognito().await else { return };
        let (_dir, core) = core();
        browser.attach(&core);
        // `public` stands for a public site; `private` for a service on the local network. Both are
        // on this machine, under `*.test` names (the policy tells them apart by port).
        let (public, private) = (Fixtures::start().await, Fixtures::start().await);
        let (a, b) = (port(&public), port(&private));
        let (site, lan) = (format!("http://jobs.test:{a}"), format!("http://intranet.test:{b}"));
        private.route("/secret", 200, "text/plain", "the admin console");
        private.route("/frame", 200, "text/html", "<p>private frame</p>");
        private.route("/pixel", 200, "image/gif", "GIF89a");
        private.route("/script.js", 200, "text/javascript", "window.stolen = 'yes'");
        public.redirect("/go", 302, &format!("{lan}/secret"));
        public.redirect("/go-literal", 302, &format!("http://127.0.0.1:{b}/secret"));
        let page = format!(
            "<html><head><title>Jobs</title></head><body><div id=out>waiting</div>
             <img src='{lan}/pixel'><iframe src='{lan}/frame'></iframe><script src='{lan}/script.js'></script>
             <script>
               fetch('{lan}/secret').then(r => r.text()).then(t => out('fetched ' + t)).catch(() => out('blocked'));
               function out(s) {{ document.getElementById('out').innerText = s + ' ' + 'Careers at Acme: we are hiring engineers for many teams. '.repeat(10); }}
             </script></body></html>"
        );
        public.route("/jobs", 200, "text/html", page);
        let renderer = Renderer::with_policy(&core, Policy::OnlyPorts(vec![a]));
        let id = session(&core, "Guarded", &format!("{site}/jobs"));

        // In a private tab and in the signed-in profile alike.
        for session_id in [None, Some(id)] {
            // The page itself is served, and nothing it asks for on the "private" server gets there.
            let rendered = renderer.render(session_id, &format!("{site}/jobs")).await.unwrap();
            assert!(rendered.html.contains("blocked"), "{session_id:?}: {}", rendered.html);
            assert!(!rendered.html.contains("admin console") && !rendered.html.contains("yes"), "{}", rendered.html);
            tokio::time::sleep(Duration::from_millis(300)).await;
            assert!(private.hits().is_empty(), "{session_id:?}: the private server saw {:?}", private.hits());

            // A redirect to it fails the render and is not followed, by name or by address.
            for go in ["/go", "/go-literal"] {
                let err = renderer.render(session_id, &format!("{site}{go}")).await.unwrap_err().to_string();
                assert!(err.contains("private network"), "{session_id:?} {go}: {err}");
                assert!(!err.contains("jobs.test"), "{err}");
            }
            assert!(private.hits().is_empty(), "{session_id:?}: the redirect was followed: {:?}", private.hits());
            // And it is refused before the browser is asked.
            assert!(renderer.render(session_id, &format!("{lan}/secret")).await.is_err());
            assert!(private.hits().is_empty());
        }
    }
}
