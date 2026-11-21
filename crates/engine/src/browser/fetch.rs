//! Rendering a page in a tab of the user's own Chrome (through the JobPilot extension, [`Bridge`]).
//!
//! A JS-rendered job page comes back (almost) empty from a plain HTTP fetch
//! ([`NeedsBrowser`](crate::pipeline::NeedsBrowser)). [`rendered_html`] opens it in a
//! **private** tab (the JobPilot Incognito window: none of the user's cookies or logins), guarded so
//! neither the page nor anything it loads reaches this machine or the private network, and returns
//! the DOM; [`generate_with_browser`] wires that into the generate pipeline.
//!
//! [`render_tab`] is the one render this and discovery ([`Renderer`](super::render::Renderer)) share:
//! open a guarded tab, load, wait for text, look for a challenge, read, and close the tab, whatever
//! happened. The tab is not disguised: the extension leaves `navigator.webdriver` true.

use std::future::Future;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};

use super::bridge::Bridge;
use super::challenge;
use super::guard::{allows, Policy};
use super::tab::{OpenOptions, Profile, Tab};
use crate::graph::nodes::ChallengeKind;
use crate::net::http::NotPublic;
use crate::pipeline::generate::shared_llm;
use crate::pipeline::ingest::MIN_PAGE_CHARS;
use crate::pipeline::{ingest, ingest_html, JdInput, JdText, NeedsBrowser, Pipeline, Progress, Stage};
use crate::store::Application;
use crate::{runtime, Core};

/// Longest a render may take once its tab is open (load + settle + read).
pub(super) const RENDER_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for the page to reach `readyState == "complete"`.
pub(super) const LOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// How long a client-side app may take to put readable text on the page after load.
const TEXT_TIMEOUT: Duration = Duration::from_secs(8);

/// Visible text length of the page, to know when a client-side app has rendered.
const TEXT_LEN_JS: &str = "(document.body ? document.body.innerText.length : 0)";

/// A rendered page: its address after redirects, its DOM, and the login, captcha or block page that
/// took its place, if any (see [`challenge`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rendered {
    pub url: String,
    pub html: String,
    pub challenge: Option<ChallengeKind>,
}

/// Accept only http(s) URLs.
fn checked_url(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw.trim()).with_context(|| format!("`{}` is not a valid URL", raw.trim()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("only http(s) links can be opened in the browser, got `{}`", raw.trim());
    }
    Ok(url.to_string())
}

/// Open `url` in a private tab of the user's Chrome and return the rendered DOM as HTML.
///
/// This is the browser fallback of **JD ingest** (a link the user pasted). The tab is guarded with
/// [`Policy::Public`]: a page on this machine or a private network is not opened. `Err` holding a
/// [`BridgeError`](super::BridgeError) when the extension is not connected or may not open
/// Incognito tabs.
pub async fn rendered_html(bridge: &Bridge, url: &str) -> Result<String> {
    rendered_html_with(bridge, url, Policy::Public).await
}

/// [`rendered_html`] under `policy` (tests read fixture pages).
pub(crate) async fn rendered_html_with(bridge: &Bridge, url: &str, policy: Policy) -> Result<String> {
    let url = checked_url(url)?;
    let (rendered, _blocked) = render_tab(bridge, &url, Profile::Private, policy).await;
    let rendered = rendered?;
    if let Some(kind) = rendered.challenge {
        bail!("{} instead of the job description: open the link in your browser and paste the text or upload the file", challenge_text(kind));
    }
    Ok(rendered.html)
}

/// What the user is told when a login, captcha or block page took the job description's place.
fn challenge_text(kind: ChallengeKind) -> &'static str {
    match kind {
        ChallengeKind::Login => "The site asked for a login",
        ChallengeKind::Captcha => "The site showed a captcha",
        ChallengeKind::Blocked => "The site blocked the request",
    }
}

/// Closes the tab when dropped (a render whose future is dropped half way leaves no tab behind).
pub(super) struct CloseOnDrop(pub(super) Tab);

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if self.0.is_open() {
            let tab = self.0.clone();
            runtime::handle().spawn(async move { tab.close().await });
        }
    }
}

/// Render `url` in a new background tab of `profile`, guarded by `policy`: load it (at most
/// [`LOAD_TIMEOUT`]), [`finish_page`], and close the tab, also on an error or a timeout. Also returns
/// how many requests the guard refused (the page itself, a redirect or a subresource).
///
/// The tab opens on `about:blank` and then navigates, so a refused page still counts as blocked.
/// `Err` holding a [`BridgeError`](super::BridgeError) when the browser cannot be used (extension
/// not connected, not allowed in Incognito).
pub(super) async fn render_tab(bridge: &Bridge, url: &str, profile: Profile, policy: Policy) -> (Result<Rendered>, usize) {
    // Refused before any tab is opened (the navigation checks it again).
    match url::Url::parse(url) {
        Ok(parsed) if allows(&policy, &parsed).await => {}
        Ok(_) => return (Err(NotPublic.into()), 0),
        Err(e) => return (Err(anyhow::Error::new(e).context("the page address is not valid")), 0),
    }
    let opened = bridge.open("about:blank", OpenOptions { profile, focus: false, guard: Some(policy) }).await;
    let tab = match opened {
        Ok(tab) => CloseOnDrop(tab),
        Err(e) => return (Err(e.context("opening a browser tab")), 0),
    };
    let result = tokio::time::timeout(RENDER_TIMEOUT, async {
        tab.0.goto(url).await.context("navigating to the page")?;
        tab.0.wait_loaded(LOAD_TIMEOUT).await;
        finish_page(&tab.0, url).await
    })
    .await
    .unwrap_or_else(|_| Err(timed_out()));
    let blocked = tab.0.blocked();
    tab.0.close().await;
    (result, blocked)
}

pub(super) fn timed_out() -> anyhow::Error {
    anyhow!("the browser did not render the page within {} seconds (timed out)", RENDER_TIMEOUT.as_secs())
}

/// The rest of a render once the page has loaded: wait (a little, never failing) for a client-side
/// app to put readable text on it, look for a challenge, and read the DOM. `requested` is the address
/// that was asked for.
pub(super) async fn finish_page(tab: &Tab, requested: &str) -> Result<Rendered> {
    wait_until(TEXT_TIMEOUT, || async { tab.eval_as::<u64>(TEXT_LEN_JS).await.is_ok_and(|n| n as usize >= MIN_PAGE_CHARS) }).await;
    let challenge = challenge::detect_at(tab, Some(requested)).await;
    let url = tab.url().await.unwrap_or_else(|_| requested.to_string());
    let html = tab.content().await.context("reading the page")?;
    Ok(Rendered { url, html, challenge })
}

/// Poll `ready` every 200 ms until it is true or `timeout` passes.
async fn wait_until<F, Fut>(timeout: Duration, mut ready: F)
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let poll = async {
        while !ready().await {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let _ = tokio::time::timeout(timeout, poll).await;
}

// ---------------------------------------------------------------- generate with a browser fallback

/// [`Pipeline::generate`] (new Draft when `existing` is `None`) or [`Pipeline::generate_into`]
/// (re-run an existing application), and when the page turns out to need a browser, render it with
/// `render_page` and continue with `JdInput::Html` on the same application.
///
/// `progress` sees every event, plus a `Fetching` event with a detail line for the browser step.
pub async fn generate_with_fallback<F, Fut>(
    pipeline: &Pipeline<'_>,
    existing: Option<i64>,
    input: JdInput,
    progress: impl Fn(Progress) + Send + Sync,
    render_page: F,
) -> Result<Application>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    // The first progress event of a new application carries its id; keep it for the retry.
    let app_id = AtomicI64::new(existing.unwrap_or(0));
    let seen = |p: Progress| {
        app_id.store(p.app_id, Ordering::SeqCst);
        progress(p);
    };
    let first = match existing {
        None => pipeline.generate(input, &seen).await,
        Some(id) => pipeline.generate_into(id, input, &seen).await,
    };
    let err = match first {
        Ok(app) => return Ok(app),
        Err(e) => e,
    };
    let Some(needs) = err.downcast_ref::<NeedsBrowser>().cloned() else { return Err(err) };
    let id = app_id.load(Ordering::SeqCst);
    if id == 0 {
        return Err(err);
    }

    progress(Progress { app_id: id, stage: Stage::Fetching, detail: "rendering the page in a browser".into() });
    let html = match render_page(needs.url.clone()).await {
        Ok(html) => html,
        Err(e) => {
            let e = e.context(format!("could not open {} in a browser", needs.url));
            let _ = pipeline.core.store.add_event(id, "error", &format!("{e:#}"));
            return Err(e);
        }
    };
    pipeline.generate_into(id, JdInput::Html { html, url: needs.url }, progress).await
}

/// [`ingest`] with the same browser fallback as [`generate_with_fallback`], for callers that only
/// need the JD text (adding a lead): when the plain fetch says the page needs a browser, render it
/// with `render_page` and read its HTML.
pub async fn ingest_with_fallback<F, Fut>(input: JdInput, render_page: F) -> Result<JdText>
where
    F: FnOnce(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let err = match ingest(input).await {
        Ok(jd) => return Ok(jd),
        Err(e) => e,
    };
    let Some(needs) = err.downcast_ref::<NeedsBrowser>().cloned() else { return Err(err) };
    let html = render_page(needs.url.clone()).await.with_context(|| format!("could not open {} in a browser", needs.url))?;
    ingest_html(&html, &needs.url)
}

/// [`ingest_with_fallback`] with a private tab of the user's Chrome ([`rendered_html`]) as the browser.
pub async fn ingest_with_browser(bridge: &Bridge, input: JdInput) -> Result<JdText> {
    ingest_with_fallback(input, |url| async move { rendered_html(bridge, &url).await }).await
}

/// The app's generate entry point: default AI provider, a private tab of the user's Chrome
/// ([`rendered_html`]) as the browser fallback.
pub async fn generate_with_browser(
    core: &Core,
    existing: Option<i64>,
    input: JdInput,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<Application> {
    if let Some(app_id) = existing {
        crate::pipeline::require_open(core, app_id)?;
    }
    let llm = shared_llm(core)?;
    let bridge = core.browser();
    let pipeline = Pipeline::new(core, &*llm).with_insight(llm.clone());
    generate_with_fallback(&pipeline, existing, input, progress, move |url| async move {
        rendered_html(&bridge, &url).await
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use super::*;
    use crate::llm::fake::FakeLlm;
    use crate::pipeline::ingest::tests::{serve, JOB_HTML};
    use crate::store::Status;

    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");
    const SECTIONS: &str = "sections: [summary, skills, projects, experience, education, languages]";

    fn core() -> (tempfile::TempDir, Core) {
        crate::profile::test_core_with_profile(crate::profile::TEST_PROFILE)
    }

    fn analysis() -> Value {
        json!({
            "company": "Acme", "role": "Senior Rust Engineer", "slug": "acme-senior-rust-engineer",
            "location": "remote", "apply_url": "", "must_have": ["Rust"], "nice_to_have": [],
            "key_terms": ["Rust"], "seniority": "senior"
        })
    }

    fn reply() -> Value {
        let one_page = EXAMPLE.replace(SECTIONS, "sections: [summary, skills, projects, experience, education]");
        json!({"yaml": one_page, "emphasised": [], "omitted": [], "gaps": ["Rust in production"]})
    }

    const SPA: &str = "<html><body><div id=root></div><script src=app.js></script></body></html>";

    #[tokio::test]
    async fn ingest_renders_a_js_page_in_the_browser_and_reads_plain_ones_directly() {
        let spa = serve(200, "text/html", SPA.to_string()).await;
        let asked = Mutex::new(Vec::new());
        let jd = ingest_with_fallback(JdInput::Url(spa.clone()), |u| {
            asked.lock().unwrap().push(u);
            async { Ok(JOB_HTML.to_string()) }
        })
        .await
        .unwrap();
        assert_eq!(asked.into_inner().unwrap(), vec![spa.clone()]);
        assert!(jd.text.contains("Senior Rust Engineer"), "{}", jd.text);
        assert_eq!(jd.url.as_deref(), Some(spa.as_str()));

        let plain = serve(200, "text/html", JOB_HTML.to_string()).await;
        let jd = ingest_with_fallback(JdInput::Url(plain), |_| async { panic!("no browser for a readable page") }).await.unwrap();
        assert!(jd.text.contains("Kubernetes"));

        let err = ingest_with_fallback(JdInput::Url(spa), |_| async { Err(anyhow!("Chrome not found")) }).await.unwrap_err();
        assert!(format!("{err:#}").contains("Chrome not found") && format!("{err:#}").contains("in a browser"), "{err:#}");
        let err = ingest_with_fallback(JdInput::Url("not a url".into()), |_| async { Ok(String::new()) }).await.unwrap_err();
        assert!(format!("{err:#}").contains("not a valid URL"), "{err:#}");
    }

    #[test]
    fn only_http_links_are_opened() {
        assert_eq!(checked_url(" https://a.test/x ").unwrap(), "https://a.test/x");
        for bad in ["file:///etc/passwd", "--remote-debugging-port=1", "javascript:alert(1)", "", "a.test"] {
            assert!(checked_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn the_browser_is_not_asked_for_a_private_page_and_its_errors_keep_their_kind() {
        use crate::browser::bridge::{Bridge, BridgeError};
        let bridge = Bridge::new(crate::Secrets::memory());
        // Refused by the guard before any tab is asked for (no extension is connected here).
        for url in ["http://127.0.0.1:9/x", "http://10.0.0.1/jobs", "http://localhost/"] {
            let err = rendered_html(&bridge, url).await.unwrap_err();
            assert!(err.downcast_ref::<BridgeError>().is_none(), "{url}: {err:#}");
            assert!(format!("{err:#}").contains("private network"), "{url}: {err:#}");
        }
        // No extension: the error says so and stays a `BridgeError` (the UI shows what to do).
        let err = rendered_html_with(&bridge, "http://jobs.test/1", Policy::Anything).await.unwrap_err();
        assert_eq!(err.downcast_ref::<BridgeError>(), Some(&BridgeError::NotConnected), "{err:#}");
    }

    #[tokio::test]
    async fn a_js_page_is_rendered_by_the_browser_and_generation_continues_on_the_same_application() {
        let (_dir, core) = core();
        let url = serve(200, "text/html", SPA.to_string()).await;
        let llm = FakeLlm::with_replies([analysis(), reply()]);
        let log = Mutex::new(Vec::new());
        let asked = Mutex::new(Vec::new());

        let app = generate_with_fallback(
            &Pipeline::new(&core, &llm),
            None,
            JdInput::Url(url.clone()),
            |p| log.lock().unwrap().push(p),
            |u| {
                asked.lock().unwrap().push(u);
                async { Ok(JOB_HTML.to_string()) }
            },
        )
        .await
        .unwrap();

        assert_eq!(asked.into_inner().unwrap(), vec![url.clone()]);
        assert_eq!(app.status, Status::Draft);
        assert_eq!((app.company.as_str(), app.role.as_str()), ("Acme", "Senior Rust Engineer"));
        assert_eq!(core.store.list_applications().unwrap().len(), 1, "no second Draft");
        assert!(app.cv_yaml_path().is_file());

        let log = log.into_inner().unwrap();
        assert!(log.iter().all(|p| p.app_id == app.id));
        assert!(log.iter().any(|p| p.stage == Stage::Fetching && p.detail.contains("browser")));
        assert_eq!(log.last().map(|p| p.stage), Some(Stage::Done));
    }

    #[tokio::test]
    async fn a_failing_browser_step_is_reported_on_the_timeline() {
        let (_dir, core) = core();
        let url = serve(200, "text/html", SPA.to_string()).await;
        let llm = FakeLlm::with_replies([analysis(), reply()]);
        let err = generate_with_fallback(
            &Pipeline::new(&core, &llm),
            None,
            JdInput::Url(url),
            |_| {},
            |_| async { Err(anyhow!("Chrome not found")) },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("Chrome not found"), "{err:#}");

        let apps = core.store.list_applications().unwrap();
        assert_eq!(apps.len(), 1);
        let events = core.store.list_events(apps[0].id).unwrap();
        assert!(events.iter().any(|e| e.kind == "error" && e.message.contains("Chrome not found")));
        assert!(llm.calls().is_empty(), "the model is not called without a JD");
    }

    #[tokio::test]
    async fn other_errors_and_plain_pages_do_not_touch_the_browser() {
        let (_dir, core) = core();
        let llm = FakeLlm::with_replies([analysis(), reply()]);
        let url = serve(200, "text/html", JOB_HTML.to_string()).await;
        let browser_used = Mutex::new(false);
        let app = generate_with_fallback(
            &Pipeline::new(&core, &llm),
            None,
            JdInput::Url(url),
            |_| {},
            |_| {
                *browser_used.lock().unwrap() = true;
                async { Ok(String::new()) }
            },
        )
        .await
        .unwrap();
        assert!(!*browser_used.lock().unwrap());
        assert_eq!(app.company, "Acme");

        // A bad link fails before any Draft exists.
        let err = generate_with_fallback(
            &Pipeline::new(&core, &llm),
            None,
            JdInput::Url("not a url".into()),
            |_| {},
            |_| async { Ok(String::new()) },
        )
        .await
        .unwrap_err();
        assert!(format!("{err:#}").contains("not a valid URL"), "{err:#}");
        assert_eq!(core.store.list_applications().unwrap().len(), 1);
    }

    /// `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine -- --ignored a_client_side_page` (after `bun run ext:build`).
    #[tokio::test]
    #[ignore = "launches Chromium with the extension"]
    async fn a_client_side_page_is_rendered_in_a_private_tab_that_is_closed_afterwards() {
        let Some(browser) = crate::browser::bridge::testkit::launch_incognito().await else { return };
        let body = format!(
            "<html><body><div id=root></div><script>setTimeout(() => {{ document.getElementById('root').innerText = '{}'; }}, 300)</script></body></html>",
            "Senior Rust Engineer at Acme. ".repeat(20)
        );
        // `*.test` is this machine in the test browser (the extension blocks 127.0.0.1 itself).
        let url = serve(200, "text/html", body).await.replace("127.0.0.1", "jobs.test");
        let html = rendered_html_with(&browser.bridge, &url, Policy::Anything).await.unwrap();
        assert!(html.contains("Senior Rust Engineer at Acme."), "{html}");
        assert!(browser.bridge.shared.state().tabs.is_empty(), "the tab was closed");

        // A login wall instead of the posting is an error that says what to do.
        let wall = serve(200, "text/html", "<html><head><title>Sign in</title></head><body><form><input name=u><input type=password name=p></form></body></html>".into()).await;
        let err = rendered_html_with(&browser.bridge, &wall.replace("127.0.0.1", "jobs.test"), Policy::Anything).await.unwrap_err();
        assert!(err.to_string().contains("asked for a login"), "{err:#}");
    }
}
