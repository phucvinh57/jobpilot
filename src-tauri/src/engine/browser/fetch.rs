//! Session-less page rendering for JD ingest.
//!
//! A JS-rendered job page comes back (almost) empty from a plain HTTP fetch
//! ([`NeedsBrowser`](crate::engine::pipeline::NeedsBrowser)). [`rendered_html`] opens it in a headless Chrome
//! with a **throwaway** profile (no cookies, no login, deleted afterwards) and returns the DOM;
//! [`generate_with_browser`] wires that into the generate pipeline.
//!
//! Logged-in browsing belongs to [`SessionManager`](super::SessionManager); this never touches a session.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chromiumoxide::handler::viewport::Viewport;
use chromiumoxide::{Browser, BrowserConfig};
use futures::StreamExt;

use super::challenge;
use super::chrome;
use super::session::NO_SANDBOX_ENV;
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::pipeline::ingest::MIN_PAGE_CHARS;
use crate::engine::pipeline::generate::shared_llm;
use crate::engine::pipeline::{ingest, ingest_html, JdInput, JdText, NeedsBrowser, Pipeline, Progress, Stage};
use crate::engine::store::Application;
use crate::engine::{Core, Paths};

/// Longest the whole render may take (launch + load + settle).
pub(super) const RENDER_TIMEOUT: Duration = Duration::from_secs(60);
/// How long to wait for the page to reach `readyState == "complete"`.
pub(super) const LOAD_TIMEOUT: Duration = Duration::from_secs(25);
/// How long a client-side app may take to put readable text on the page after load.
const TEXT_TIMEOUT: Duration = Duration::from_secs(8);

/// Flags for the throwaway browser (a subset of the managed-session flags: nothing is persisted).
const FLAGS: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-sync",
    "--disable-extensions",
    "--password-store=basic",
    "--use-mock-keychain",
    "--disable-background-mode",
    "--disable-breakpad",
    "--disable-default-apps",
    "--disable-dev-shm-usage",
    "--force-color-profile=srgb",
    "--lang=en-US",
];

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

/// A temp profile dir that is deleted when dropped.
pub(super) struct TempProfile(PathBuf);

impl TempProfile {
    pub(super) fn create(paths: &Paths) -> Result<Self> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = paths.data_dir.join("tmp").join(format!("fetch-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(Self(dir))
    }

    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempProfile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Accept only http(s) URLs (this also keeps the value from ever being parsed as a Chrome flag).
fn checked_url(raw: &str) -> Result<String> {
    let url = url::Url::parse(raw.trim()).with_context(|| format!("`{}` is not a valid URL", raw.trim()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("only http(s) links can be opened in the browser, got `{}`", raw.trim());
    }
    Ok(url.to_string())
}

/// Open `url` in a headless Chrome with a throwaway profile and return the rendered DOM as HTML.
///
/// This is the browser fallback of **JD ingest** (a link the user pasted), with the managed browser
/// flags. Job discovery reads pages with [`render::Renderer`](super::render::Renderer) instead, which
/// adds the request guard. No browser hides that it is automated.
pub async fn rendered_html(paths: &Paths, url: &str) -> Result<String> {
    let url = checked_url(url)?;
    let chrome = chrome::resolve(paths)
        .context("Chrome not found: install Chrome/Chromium or download the browser in Settings > Browser sessions")?;
    let profile = TempProfile::create(paths)?;
    let rendered = tokio::time::timeout(RENDER_TIMEOUT, render(&chrome.path, profile.path(), &url))
        .await
        .map_err(|_| anyhow!("the browser did not render {url} within {} seconds", RENDER_TIMEOUT.as_secs()))??;
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

async fn render(chrome: &Path, profile: &Path, url: &str) -> Result<Rendered> {
    let mut builder = BrowserConfig::builder()
        .chrome_executable(chrome)
        .user_data_dir(profile)
        .disable_default_args()
        .args(FLAGS.iter().copied())
        .new_headless_mode()
        .window_size(1280, 800)
        .viewport(Viewport { width: 1280, height: 800, ..Default::default() })
        .launch_timeout(Duration::from_secs(30));
    if std::env::var_os(NO_SANDBOX_ENV).is_some_and(|v| !v.is_empty()) {
        builder = builder.no_sandbox();
    }
    let config = builder.build().map_err(|e| anyhow!(e))?;
    let (mut browser, mut handler) =
        Browser::launch(config).await.with_context(|| format!("launching {}", chrome.display()))?;
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });

    let result = read_page(&browser, url).await;

    let _ = browser.close().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), browser.wait()).await;
    let _ = browser.kill().await;
    driver.abort();
    result
}

async fn read_page(browser: &Browser, url: &str) -> Result<Rendered> {
    let page = browser.new_page("about:blank").await.context("opening a tab")?;
    page.goto(url).await.with_context(|| format!("navigating to {url}"))?;
    wait_until(LOAD_TIMEOUT, || async {
        page.evaluate_expression("document.readyState")
            .await
            .ok()
            .and_then(|v| v.into_value::<String>().ok())
            .is_some_and(|s| s == "complete")
    })
    .await;
    finish_page(&page, url).await
}

/// The rest of a render once the page has loaded: wait (a little, never failing) for a client-side
/// app to put readable text on it, look for a challenge, and read the DOM. `requested` is the address
/// that was asked for.
pub(super) async fn finish_page(page: &chromiumoxide::Page, requested: &str) -> Result<Rendered> {
    wait_until(TEXT_TIMEOUT, || async {
        page.evaluate_expression(TEXT_LEN_JS)
            .await
            .ok()
            .and_then(|v| v.into_value::<u64>().ok())
            .is_some_and(|n| n as usize >= MIN_PAGE_CHARS)
    })
    .await;
    let challenge = challenge::detect_at(page, Some(requested)).await;
    let url = page.url().await.ok().flatten().unwrap_or_else(|| requested.to_string());
    let html = page.content().await.context("reading the page")?;
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

/// [`ingest_with_fallback`] with headless Chrome (a throwaway profile) as the browser.
pub async fn ingest_with_browser(paths: &Paths, input: JdInput) -> Result<JdText> {
    ingest_with_fallback(input, |url| async move { rendered_html(paths, &url).await }).await
}

/// The app's generate entry point: default AI provider, headless Chrome as the browser fallback.
pub async fn generate_with_browser(
    core: &Core,
    existing: Option<i64>,
    input: JdInput,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<Application> {
    if let Some(app_id) = existing {
        crate::engine::pipeline::require_open(core, app_id)?;
    }
    let llm = shared_llm(core)?;
    let paths = core.paths.clone();
    let pipeline = Pipeline::new(core, &*llm).with_insight(llm.clone());
    generate_with_fallback(&pipeline, existing, input, progress, move |url| async move {
        rendered_html(&paths, &url).await
    })
    .await
}

#[cfg(test)]
mod flag_tests {
    use super::FLAGS;

    #[test]
    fn the_jd_browser_is_not_disguised() {
        // No fingerprint spoofing.
        assert!(!FLAGS.iter().any(|a| a.contains("AutomationControlled") || a.contains("user-agent")), "{FLAGS:?}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use super::*;
    use crate::engine::llm::fake::FakeLlm;
    use crate::engine::pipeline::ingest::tests::{serve, JOB_HTML};
    use crate::engine::store::Status;
    use crate::engine::Paths;

    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");
    const SECTIONS: &str = "sections: [summary, skills, projects, experience, education, languages]";

    fn core() -> (tempfile::TempDir, Core) {
        crate::engine::profile::test_core_with_profile(crate::engine::profile::TEST_PROFILE)
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

    #[test]
    fn temp_profile_is_removed_on_drop() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let profile = TempProfile::create(&paths).unwrap();
        let path = profile.path().to_path_buf();
        assert!(path.is_dir());
        drop(profile);
        assert!(!path.exists());
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

    #[test]
    #[ignore = "launches Chrome"]
    fn headless_chrome_renders_a_client_side_page() {
        crate::engine::runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::new(dir.path().join("jp"));
            paths.ensure().unwrap();
            let body = format!(
                "<html><body><div id=root></div><script>setTimeout(() => {{ document.getElementById('root').innerText = '{}'; }}, 300)</script></body></html>",
                "Senior Rust Engineer at Acme. ".repeat(20)
            );
            let url = serve(200, "text/html", body).await;
            let html = rendered_html(&paths, &url).await.unwrap();
            assert!(html.contains("Senior Rust Engineer at Acme."), "{html}");
            assert!(!paths.data_dir.join("tmp").read_dir().unwrap().any(|_| true), "temp profile removed");
        });
    }
}
