//! Challenge detection: is a login, captcha or block page standing between the app and a page?
//!
//! This only **detects**. The app never solves, evades or waits out a challenge: a source that meets
//! one pauses, the user is notified and deals with it in the visible managed browser (see
//! `discover::handoff`). No solver service, no fingerprint tricks.
//!
//! - [`detect`] / [`detect_at`]: look at a rendered page. One script reads what is on screen (title,
//!   the start of the text, visible captcha widgets, the navigation's HTTP status); the verdict is
//!   [`classify`], a plain function over [`PageFacts`] so it is testable without a browser. Login uses
//!   the pieces of the session probe (`probe`): a visible password input, or a redirect to a login URL.
//! - [`detect_text`]: the cheap version for the API path, with no browser: an HTTP status and the start
//!   of a body (text or HTML).
//!
//! Every rule that reads words is limited to **short pages** (an interstitial has a few sentences; a
//! job posting does not), so a posting that mentions "access denied" or carries an invisible captcha
//! badge for its apply form is not mistaken for a block page.

use std::sync::OnceLock;

use chromiumoxide::Page;
use regex::Regex;
use serde::Deserialize;

use super::probe::{looks_like_login_url, PASSWORD_JS};
use crate::graph::nodes::ChallengeKind;

/// Pages with at least this much text are real content for the word-based rules.
pub const INTERSTITIAL_MAX_CHARS: usize = 800;
/// A visible password input counts as a login wall on pages up to this long (a login page has a
/// form and a few lines; an article with a sign-in box in its sidebar is longer).
const LOGIN_PAGE_MAX_CHARS: usize = 2_500;
/// How much of the page text is kept in [`PageFacts::text`].
const TEXT_SNIPPET_CHARS: usize = 2_000;
/// How much of a body [`detect_text`] reads.
const BODY_SNIPPET_BYTES: usize = 64 * 1024;

/// A login, captcha or block page was found where a page should be. Raised by the code that renders
/// pages for discovery; the runner turns it into the `wait_human` interrupt instead of skipping the
/// page. The text names the kind only: the address stays out of notes and logs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChallengeFound {
    pub kind: ChallengeKind,
    /// The page that was requested (not where a redirect led), for "Open browser".
    pub url: String,
}

impl std::fmt::Display for ChallengeFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self.kind {
            ChallengeKind::Login => "the site asks for a login",
            ChallengeKind::Captcha => "the site shows a captcha",
            ChallengeKind::Blocked => "the site blocks automated access",
        })
    }
}

impl std::error::Error for ChallengeFound {}

/// Elements of the known captcha widgets (reCAPTCHA, hCaptcha, Cloudflare Turnstile, and the
/// interstitial widgets of DataDome, PerimeterX and Arkose).
const WIDGET_SELECTORS: &[&str] = &[
    ".g-recaptcha",
    "iframe[src*='recaptcha']",
    "iframe[title*='reCAPTCHA' i]",
    ".h-captcha",
    "iframe[src*='hcaptcha.com']",
    "iframe[title*='hCaptcha' i]",
    ".cf-turnstile",
    "iframe[src*='challenges.cloudflare.com']",
    "iframe[title*='Cloudflare security challenge' i]",
    "[id^='cf-chl-widget']",
    "iframe[src*='captcha-delivery.com']",
    "#px-captcha",
    "iframe[src*='arkoselabs.com']",
];

/// The same widgets as they appear in raw markup (lower case), for [`detect_text`].
const WIDGET_MARKERS: &[&str] = &[
    "g-recaptcha",
    "google.com/recaptcha",
    "h-captcha",
    "hcaptcha.com",
    "cf-turnstile",
    "challenges.cloudflare.com",
    "captcha-delivery.com",
    "px-captcha",
    "arkoselabs.com",
];

/// Interstitial titles that ask for a human (matched at the start of the page title).
const CAPTCHA_TITLES: &[&str] = &[
    "just a moment",
    "attention required",
    "checking your browser",
    "verify you are human",
    "verifying you are human",
    "human verification",
    "are you a robot",
    "are you human",
    "robot check",
    "security check",
    "one more step",
    "captcha",
];

/// Titles of block pages.
const BLOCK_TITLES: &[&str] = &[
    "access denied",
    "access to this page has been denied",
    "pardon our interruption",
    "request blocked",
    "you have been blocked",
    "403 forbidden",
    "429 too many requests",
    "error 403",
    "error 429",
    "error 1020",
];

/// Sentences of a captcha or "checking your browser" page.
const CAPTCHA_PHRASES: &[&str] = &[
    "verify you are human",
    "verifying you are human",
    "verify that you are human",
    "confirm you are human",
    "prove you are human",
    "i'm not a robot",
    "i am not a robot",
    "checking your browser",
    "checking if the site connection is secure",
    "complete the security check",
    "needs to review the security of your connection",
    "press & hold",
    "press and hold to confirm",
    "solve this captcha",
    "complete the captcha",
    "enable javascript and cookies to continue",
    "are you a robot",
    "are you a human",
];

/// Sentences of a block page.
const BLOCK_PHRASES: &[&str] = &[
    "access denied",
    "access to this page has been denied",
    "you have been blocked",
    "you've been blocked",
    "request blocked",
    "your request has been blocked",
    "automated access",
    "automated requests",
    "automated traffic",
    "unusual traffic",
    "suspicious activity",
    "bot activity",
    "403 forbidden",
    "429 too many requests",
    "too many requests",
    "pardon our interruption",
    "error 1020",
];

/// Sentences of a login wall.
const LOGIN_PHRASES: &[&str] = &[
    "sign in to continue",
    "log in to continue",
    "login to continue",
    "sign in to view",
    "log in to view",
    "sign in to see",
    "join to view",
    "you must be logged in",
    "you must be signed in",
    "please sign in to",
    "please log in to",
];

/// What a page showed: the input of [`classify`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct PageFacts {
    /// The address the browser ended up at.
    pub url: String,
    pub title: String,
    /// HTTP status of the navigation (`0` when unknown).
    pub status: u16,
    /// Characters of visible text.
    pub text_len: usize,
    /// The start of the visible text.
    pub text: String,
    /// A captcha widget is visible (the invisible reCAPTCHA badge does not count).
    pub widget: bool,
    /// A password input is visible.
    #[serde(skip)]
    pub password: bool,
}

/// The verdict on a page. Precedence: a captcha, then a login, then a block; whatever the answer, a
/// page with plenty of text is content unless the server said 401, 403 or 429 or a password field
/// (up to [`LOGIN_PAGE_MAX_CHARS`]) or a redirect to a login address says otherwise.
///
/// `requested` is the address that was asked for: a final address that looks like a login page counts
/// only when it is not the one that was asked for.
pub fn classify(facts: &PageFacts, requested: Option<&str>) -> Option<ChallengeKind> {
    let short = facts.text_len < INTERSTITIAL_MAX_CHARS;
    let title = facts.title.trim().to_lowercase();
    let text = facts.text.to_lowercase();
    let starts = |list: &[&str]| list.iter().any(|t| title.starts_with(t));
    let says = |list: &[&str]| list.iter().any(|p| text.contains(p));

    if short && (facts.widget || starts(CAPTCHA_TITLES) || says(CAPTCHA_PHRASES)) {
        return Some(ChallengeKind::Captcha);
    }
    let login_url = looks_like_login_url(&facts.url) && requested.is_none_or(|r| !same_page(r, &facts.url) && !looks_like_login_url(r));
    if facts.status == 401 || login_url || (facts.password && facts.text_len < LOGIN_PAGE_MAX_CHARS) {
        return Some(ChallengeKind::Login);
    }
    if matches!(facts.status, 403 | 429) || (short && (starts(BLOCK_TITLES) || says(BLOCK_PHRASES))) {
        return Some(ChallengeKind::Blocked);
    }
    if short && says(LOGIN_PHRASES) {
        return Some(ChallengeKind::Login);
    }
    None
}

/// Same address, ignoring the fragment and a trailing slash.
fn same_page(a: &str, b: &str) -> bool {
    let norm = |u: &str| u.split('#').next().unwrap_or_default().trim_end_matches('/').to_string();
    norm(a) == norm(b)
}

/// The cheap check for the API path: the HTTP `status` of an answer (if known) and the start of its
/// `body`, plain text or HTML. HTML is reduced to its title and visible text first; its widget markup
/// (a Turnstile or reCAPTCHA container) counts like a visible widget.
pub fn detect_text(status: Option<u16>, body: &str) -> Option<ChallengeKind> {
    let body = if body.len() > BODY_SNIPPET_BYTES {
        let mut end = BODY_SNIPPET_BYTES;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        &body[..end]
    } else {
        body
    };
    let is_html = {
        let head: String = body.chars().take(300).collect::<String>().to_lowercase();
        head.contains("<html") || head.contains("<!doctype html") || head.contains("<head") || head.contains("<body")
    };
    let (title, text, widget) = if is_html {
        let lower = body.to_lowercase();
        let text = crate::pipeline::ingest::clean(&html2text::from_read(body.as_bytes(), 120).unwrap_or_default());
        (html_title(body), text, WIDGET_MARKERS.iter().any(|m| lower.contains(m)))
    } else {
        (String::new(), body.to_string(), false)
    };
    let facts = PageFacts {
        status: status.unwrap_or(0),
        title,
        text_len: text.chars().count(),
        text: text.chars().take(TEXT_SNIPPET_CHARS).collect(),
        widget,
        ..PageFacts::default()
    };
    classify(&facts, None)
}

fn html_title(html: &str) -> String {
    static TITLE: OnceLock<Regex> = OnceLock::new();
    let re = TITLE.get_or_init(|| Regex::new(r"(?is)<title[^>]*>(.*?)</title>").expect("static regex"));
    re.captures(html).and_then(|c| c.get(1)).map(|m| m.as_str().split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default()
}

/// One script: title, text start, visible widgets and the HTTP status of the navigation.
fn facts_script() -> String {
    const TEMPLATE: &str = r#"(() => {
  const shown = e => {
    const r = e.getBoundingClientRect(), s = getComputedStyle(e);
    return r.width >= 40 && r.height >= 20 && s.visibility !== 'hidden' && s.display !== 'none' && s.opacity !== '0';
  };
  const nav = (performance.getEntriesByType('navigation') || [])[0];
  const text = document.body ? document.body.innerText : '';
  const widget = Array.from(document.querySelectorAll("__WIDGETS__")).some(e =>
    !e.closest('.grecaptcha-badge') && !(e.getAttribute('src') || '').includes('size=invisible') && shown(e));
  return JSON.stringify({
    url: location.href, title: document.title || '', status: (nav && nav.responseStatus) || 0,
    text_len: text.length, text: text.slice(0, __TEXT__), widget
  });
})()"#;
    TEMPLATE.replace("__WIDGETS__", &WIDGET_SELECTORS.join(", ")).replace("__TEXT__", &TEXT_SNIPPET_CHARS.to_string())
}

/// What `page` shows right now, or `None` when it cannot be read (the page is gone or navigating).
pub async fn facts(page: &Page) -> Option<PageFacts> {
    let json: String = page.evaluate_expression(facts_script()).await.ok()?.into_value().ok()?;
    let mut facts: PageFacts = serde_json::from_str(&json).ok()?;
    facts.password = page.evaluate_expression(PASSWORD_JS).await.ok().and_then(|v| v.into_value::<bool>().ok()).unwrap_or(false);
    Some(facts)
}

/// Is a login, captcha or block page showing in `page`? A page that cannot be read is not reported.
pub async fn detect(page: &Page) -> Option<ChallengeKind> {
    detect_at(page, None).await
}

/// [`detect`] knowing which address was asked for (see [`classify`]).
pub async fn detect_at(page: &Page, requested: Option<&str>) -> Option<ChallengeKind> {
    classify(&facts(page).await?, requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(title: &str, text: &str) -> PageFacts {
        PageFacts { title: title.into(), text: text.into(), text_len: text.chars().count(), url: "https://x.test/jobs/1".into(), ..PageFacts::default() }
    }

    #[test]
    fn interstitial_titles_and_phrases_are_captchas() {
        for (title, text) in [
            ("Just a moment...", "Enable JavaScript and cookies to continue"),
            ("Attention Required! | Cloudflare", "Please complete the security check to access x.test"),
            ("", "Verifying you are human. This may take a few seconds."),
            ("Security Check", ""),
            ("Are you a robot?", "Please confirm below"),
        ] {
            assert_eq!(classify(&seen(title, text), None), Some(ChallengeKind::Captcha), "{title} / {text}");
        }
        assert_eq!(classify(&PageFacts { widget: true, ..seen("Sign up", "Welcome") }, None), Some(ChallengeKind::Captcha));
    }

    #[test]
    fn statuses_and_block_text_are_blocks() {
        for status in [403, 429] {
            assert_eq!(classify(&PageFacts { status, ..seen("", "") }, None), Some(ChallengeKind::Blocked), "{status}");
        }
        // Even a long page with that status is a block: the server said so.
        let long = "word ".repeat(500);
        assert_eq!(classify(&PageFacts { status: 403, ..seen("Jobs", &long) }, None), Some(ChallengeKind::Blocked));
        for (title, text) in [
            ("Access Denied", ""),
            ("", "You have been blocked. Your IP address has been flagged for unusual traffic."),
            ("", "Sorry, we cannot serve automated requests from your network."),
            ("429 Too Many Requests", ""),
        ] {
            assert_eq!(classify(&seen(title, text), None), Some(ChallengeKind::Blocked), "{title} / {text}");
        }
    }

    #[test]
    fn logins_are_a_status_a_password_field_a_redirect_or_a_sentence() {
        assert_eq!(classify(&PageFacts { status: 401, ..seen("", "") }, None), Some(ChallengeKind::Login));
        assert_eq!(classify(&PageFacts { password: true, ..seen("Sign in", "Email Password") }, None), Some(ChallengeKind::Login));
        let redirected = PageFacts { url: "https://x.test/login?next=/jobs".into(), ..seen("", "Welcome back") };
        assert_eq!(classify(&redirected, Some("https://x.test/jobs/1")), Some(ChallengeKind::Login));
        assert_eq!(classify(&redirected, None), Some(ChallengeKind::Login));
        // Asked for the login page itself: not a redirect, nothing to report.
        assert_eq!(classify(&redirected, Some("https://x.test/login?next=/jobs")), None);
        assert_eq!(classify(&seen("", "Please sign in to continue to the jobs board."), None), Some(ChallengeKind::Login));
    }

    #[test]
    fn a_page_with_plenty_of_text_is_content() {
        let posting = "You will build backend services in Rust. Access denied errors are logged and we see unusual traffic patterns. ".repeat(10);
        let page = PageFacts { widget: true, ..seen("Security Check Analyst - Acme", &posting) };
        assert_eq!(classify(&page, None), None, "an apply-form captcha, a title and phrases on a long page");
        let with_form = PageFacts { password: true, ..seen("Jobs", &"text ".repeat(800)) };
        assert_eq!(classify(&with_form, None), None, "a sign-in box next to a long article is not a login wall");
        assert_eq!(classify(&seen("Backend Engineer", "Short but fine"), None), None);
        assert_eq!(classify(&PageFacts { status: 404, ..seen("Not found", "Nothing here") }, None), None);
        assert_eq!(classify(&PageFacts { url: "https://x.test/jobs/login-engineer".into(), ..seen("", "Hello") }, Some("https://x.test/jobs/login-engineer")), None);
    }

    #[test]
    fn the_text_detector_reads_a_status_and_a_body_snippet() {
        assert_eq!(detect_text(Some(403), "Forbidden"), Some(ChallengeKind::Blocked));
        assert_eq!(detect_text(Some(429), ""), Some(ChallengeKind::Blocked));
        assert_eq!(detect_text(Some(401), "{}"), Some(ChallengeKind::Login));
        assert_eq!(detect_text(Some(200), "Access denied. Reference #18.abc"), Some(ChallengeKind::Blocked));
        assert_eq!(detect_text(None, "Just a moment... Verifying you are human."), Some(ChallengeKind::Captcha));
        assert_eq!(detect_text(Some(200), "Please sign in to continue."), Some(ChallengeKind::Login));
        assert_eq!(detect_text(Some(200), "{\"jobs\": []}"), None);
        assert_eq!(detect_text(Some(200), ""), None);
        assert_eq!(detect_text(Some(503), "Service Unavailable"), None, "a busy server is not a challenge");
        let posting = format!("We are hiring a backend engineer. {}", "You will build services in Rust and own their quality. ".repeat(30));
        assert_eq!(detect_text(Some(200), &posting), None);
    }

    #[test]
    fn the_text_detector_reduces_html_to_its_title_text_and_widgets() {
        let cloudflare = "<!DOCTYPE html><html><head><title>Just a moment...</title><style>body{}</style><script>var a = 1;</script></head><body><h1>x.test</h1><p>Verify you are human by completing the action below.</p><div class=\"cf-turnstile\" data-sitekey=\"k\"></div></body></html>";
        assert_eq!(detect_text(Some(403), cloudflare), Some(ChallengeKind::Captcha));
        let widget_only = "<html><head><title>Sign up</title></head><body><form><div class=\"g-recaptcha\" data-sitekey=\"k\"></div></form></body></html>";
        assert_eq!(detect_text(Some(200), widget_only), Some(ChallengeKind::Captcha));
        let blocked = "<html><head><title>403 Forbidden</title></head><body><center><h1>403 Forbidden</h1></center><hr><center>nginx</center></body></html>";
        assert_eq!(detect_text(Some(403), blocked), Some(ChallengeKind::Blocked));
        assert_eq!(detect_text(Some(200), blocked), Some(ChallengeKind::Blocked), "the title alone says it");
        let posting = format!("<html><head><title>Engineer</title><script src=\"https://www.google.com/recaptcha/api.js\"></script></head><body><main><p>{}</p><div class=\"g-recaptcha\"></div></main></body></html>", "You will build services in Rust and own their quality. ".repeat(30));
        assert_eq!(detect_text(Some(200), &posting), None, "a long posting with a captcha in its apply form is content");
    }

    #[test]
    fn a_huge_body_is_cut_on_a_character_boundary() {
        let body = "\u{e9}".repeat(BODY_SNIPPET_BYTES);
        assert_eq!(detect_text(Some(200), &body), None);
    }

    #[test]
    fn the_found_challenge_names_the_kind_but_not_the_page() {
        let found = ChallengeFound { kind: ChallengeKind::Captcha, url: "https://secret.test/jobs/1".into() };
        let text = format!("{:#}", anyhow::Error::new(found));
        assert_eq!(text, "the site shows a captcha");
        assert_eq!(crate::health::classify(&text), crate::health::Problem::Other, "it is not a network error to retry");
    }

    #[test]
    fn the_facts_script_lists_every_widget_selector() {
        let js = facts_script();
        for sel in WIDGET_SELECTORS {
            assert!(js.contains(sel), "{sel}");
        }
        assert!(!js.contains("__"), "no placeholder is left: {js}");
    }

    // ---- real pages in a real browser: `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored challenge`

    use crate::net::fixtures::Fixtures;

    const WIDGET_STYLE: &str = "width:304px;height:78px;border:1px solid #888;display:block";

    fn page(title: &str, body: &str) -> String {
        format!("<!doctype html><html><head><title>{title}</title></head><body>{body}</body></html>")
    }

    /// A job page with plenty of text: what a real posting looks like to the detector.
    fn posting(extra: &str) -> String {
        let text = "You will build and run backend services in Rust, own their quality, and work with a small team. ".repeat(12);
        page("Backend Engineer - Acme", &format!("<main><h1>Backend Engineer</h1><p>{text}</p>{extra}</main>"))
    }

    fn fixtures(fx: &Fixtures) {
        let html = "text/html; charset=utf-8";
        fx.route("/login", 200, html, page("Sign in", "<form><input name=u><input type=password name=p><button>Sign in</button></form>"))
            .redirect("/jobs/1", 302, &format!("{}/account/login?next=/jobs/1", fx.base()))
            .route("/account/login", 200, html, page("Welcome", "<p>Welcome back. Your session ended.</p>"))
            .route("/recaptcha", 200, html, page("Sign up", &format!("<form><div class=\"g-recaptcha\" data-sitekey=\"k\" style=\"{WIDGET_STYLE}\"></div></form>")))
            .route("/recaptcha-frame", 200, html, page("Sign up", "<iframe title=\"reCAPTCHA\" src=\"about:blank\" width=\"304\" height=\"78\"></iframe>"))
            .route("/hcaptcha", 200, html, page("Verify", &format!("<div class=\"h-captcha\" data-sitekey=\"k\" style=\"{WIDGET_STYLE}\"></div>")))
            .route("/turnstile", 200, html, page("Verify", &format!("<div class=\"cf-turnstile\" data-sitekey=\"k\" style=\"{WIDGET_STYLE}\"></div>")))
            .route("/moment", 200, html, page("Just a moment...", "<h1>x.test</h1><p>Enable JavaScript and cookies to continue</p>"))
            .route("/403", 403, html, page("Error", "<h1>Not for you</h1>"))
            .route("/429", 429, html, page("Error", "<h1>Slow down</h1>"))
            .route("/blocktext", 200, html, page("Access Denied", "<p>You have been blocked. Reference #18.2d351ab8.</p>"))
            .route("/hello", 200, html, page("Hello", "<h1>Hello</h1><p>Nothing to see.</p>"))
            .route("/posting", 200, html, posting(""))
            // An apply form with a visible captcha and an invisible-badge, on a long page: content.
            .route(
                "/posting-with-captcha",
                200,
                html,
                posting(&format!(
                    "<form><div class=\"g-recaptcha\" data-sitekey=\"k\" style=\"{WIDGET_STYLE}\"></div></form><div class=\"grecaptcha-badge\" style=\"width:256px;height:60px;position:fixed;bottom:0;right:0\"><iframe title=\"reCAPTCHA\" src=\"about:blank\" width=\"256\" height=\"60\"></iframe></div>"
                )),
            );
    }

    #[test]
    #[ignore = "launches Chrome"]
    fn challenge_detect_tells_login_captcha_block_and_content_apart_in_a_real_browser() {
        use crate::{runtime, Core, Paths, Secrets};
        runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
            let renderer = crate::browser::render::Renderer::new(&core, true);
            let fx = Fixtures::start().await;
            fixtures(&fx);

            let expected = [
                ("/login", Some(ChallengeKind::Login)),
                ("/jobs/1", Some(ChallengeKind::Login)),
                ("/recaptcha", Some(ChallengeKind::Captcha)),
                ("/recaptcha-frame", Some(ChallengeKind::Captcha)),
                ("/hcaptcha", Some(ChallengeKind::Captcha)),
                ("/turnstile", Some(ChallengeKind::Captcha)),
                ("/moment", Some(ChallengeKind::Captcha)),
                ("/403", Some(ChallengeKind::Blocked)),
                ("/429", Some(ChallengeKind::Blocked)),
                ("/blocktext", Some(ChallengeKind::Blocked)),
                ("/hello", None),
                ("/posting", None),
                ("/posting-with-captcha", None),
            ];
            // A few browsers at a time (each page waits a few seconds for text that never comes).
            for chunk in expected.chunks(5) {
                let renders = chunk.iter().map(|(path, _)| {
                    let (renderer, url) = (renderer.clone(), format!("{}{path}", fx.base()));
                    async move { renderer.render(None, &url).await }
                });
                for ((path, want), got) in chunk.iter().zip(futures::future::join_all(renders).await) {
                    let got = got.unwrap_or_else(|e| panic!("{path}: {e:#}"));
                    assert_eq!(got.challenge, *want, "{path}");
                    assert!(!got.html.is_empty(), "{path}: the DOM is returned either way");
                }
            }
        });
    }

    #[test]
    #[ignore = "launches Chrome"]
    fn detect_reads_a_page_that_is_open_in_a_browser_and_the_facts_carry_the_status() {
        use crate::{runtime, Core, Paths, Secrets};
        runtime::block_on(async {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
            let mgr = core.sessions();
            let session = mgr.create("fixture", "https://fixture.test/").await.unwrap();
            let fx = Fixtures::start().await;
            fixtures(&fx);
            // The same detection through a managed session's profile: `detect` over a page we opened.
            let handle = mgr.launch(session.id, true).await.unwrap();
            let blocked = handle.new_page(&format!("{}/403", fx.base())).await.unwrap();
            let page_facts = super::facts(&blocked).await.expect("facts");
            assert_eq!((page_facts.status, detect(&blocked).await), (403, Some(ChallengeKind::Blocked)), "{page_facts:?}");
            let hello = handle.new_page(&format!("{}/hello", fx.base())).await.unwrap();
            assert_eq!((super::facts(&hello).await.map(|f| f.status), detect(&hello).await), (Some(200), None));
            let login = handle.new_page(&format!("{}/login", fx.base())).await.unwrap();
            let page_facts = super::facts(&login).await.expect("facts");
            assert!(page_facts.password, "{page_facts:?}");
            assert_eq!(detect(&login).await, Some(ChallengeKind::Login));
            handle.close().await.unwrap();

            // And `Renderer` in the session: the same verdict, and the profile is free afterwards.
            let renderer = crate::browser::render::Renderer::new(&core, true);
            let got = renderer.render(Some(session.id), &format!("{}/turnstile", fx.base())).await.unwrap();
            assert_eq!(got.challenge, Some(ChallengeKind::Captcha));
            assert!(!mgr.is_open(session.id));
            // The address stays out of a failure's text.
            let err = renderer.render(Some(session.id), "http://127.0.0.1:1/never").await.err();
            if let Some(e) = err {
                assert!(!format!("{e:#}").contains("127.0.0.1:1/never"), "{e:#}");
            }
        });
    }
}
