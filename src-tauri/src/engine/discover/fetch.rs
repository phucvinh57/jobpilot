//! The default posting fetch: a page's readable text through the shared safe HTTP client
//! ([`crate::engine::net::http`]), so a posting URL taken from third-party data (an HN comment, a board's
//! `absolute_url`) gets the same checks as every other request: public hosts only on every hop,
//! the per-host delay and a size cap while streaming. Text extraction is the one of
//! `pipeline::ingest`, except for a page that carries exactly one schema.org `JobPosting` (JSON-LD):
//! with a full description its text is built from that item, which is cleaner than reading the page;
//! with a short one (a teaser) the page is read as usual and the item's header (location, salary,
//! dates) goes first ([`structured`]).

use anyhow::{Context, Result};
use futures::FutureExt;

use super::deps::PostingFetcher;
use super::source::jsonld;
use crate::engine::browser::challenge::ChallengeFound;
use crate::engine::browser::render::Renderer;
use crate::engine::net::http::{checked_url, FetchError, Http};
use crate::engine::pipeline::ingest::{extract_html, page_text, NeedsBrowser};

/// Largest page read (the adapters' JSON answers have their own, larger cap).
pub(crate) const MAX_POSTING_BYTES: usize = 10 * 1024 * 1024;

/// `Accept` only: any other header would make the client refuse a redirect to another origin
/// (a posting link often goes through the company's own domain first).
const ACCEPT: &str = "text/html,application/xhtml+xml,application/pdf;q=0.9,*/*;q=0.5";

/// Shortest `JobPosting` description (as text) that stands for the whole posting. Many sites put a
/// one-line teaser there; below this the page itself is read.
const MIN_STRUCTURED_CHARS: usize = 400;

/// A [`PostingFetcher`] reading through `http` (its cap should be [`MAX_POSTING_BYTES`]).
pub(crate) fn fetcher(http: Http) -> PostingFetcher {
    std::sync::Arc::new(move |req| {
        let http = http.clone();
        async move { posting_text(&http, &req.candidate.url).await }.boxed()
    })
}

/// [`fetcher`] with the browser as the fallback: a page the plain read cannot use ([`NeedsBrowser`]:
/// a wall, or a page JavaScript draws) is rendered in a background tab of the user's Chrome (their
/// signed-in profile when the source has a session, else a private tab). A login, captcha or block
/// page shown there is **not** skipped: it is returned as [`ChallengeFound`], so the run pauses and
/// the user deals with it in a tab opened for them (`Core::discover_open_browser`). Any other
/// browser failure is an ordinary error.
pub(crate) fn fetcher_with_browser(http: Http, renderer: Renderer) -> PostingFetcher {
    std::sync::Arc::new(move |req| {
        let (http, renderer) = (http.clone(), renderer.clone());
        async move {
            match posting_text(&http, &req.candidate.url).await {
                Err(e) if e.downcast_ref::<NeedsBrowser>().is_some() => browsed_text(&renderer, req.source.session_id, &req.candidate.url).await,
                other => other,
            }
        }
        .boxed()
    })
}

/// The readable text of `url` rendered in the browser; [`ChallengeFound`] when a challenge is shown.
async fn browsed_text(renderer: &Renderer, session_id: Option<i64>, url: &str) -> Result<String> {
    let rendered = renderer.render(session_id, url).await?;
    if let Some(kind) = rendered.challenge {
        return Err(anyhow::Error::new(ChallengeFound { kind, url: url.to_string() }));
    }
    Ok(rendered_text(&rendered.html, &rendered.url))
}

/// The text of a page the browser rendered: [`structured`] first, else the page's own text.
fn rendered_text(html: &str, page_url: &str) -> String {
    match structured(html, page_url) {
        Structured::Full(text) => text,
        Structured::Header(header) => jsonld::with_header(&header, &extract_html(html, Some(page_url))),
        Structured::None => extract_html(html, Some(page_url)),
    }
}

/// What a posting page's `JobPosting` JSON-LD gives.
#[derive(Debug, PartialEq, Eq)]
enum Structured {
    /// The posting's whole text: header and a description of at least [`MIN_STRUCTURED_CHARS`].
    Full(String),
    /// Only the header lines, to put before the page's own text (the description is a teaser).
    Header(String),
    /// No help: no posting, several (a listing, or related jobs), or a header with nothing in it.
    None,
}

/// The page's `JobPosting` JSON-LD, when it has exactly one. `page_url` (after redirects) resolves
/// the item's relative addresses. A posting whose `validThrough` has passed is still read (the
/// header shows the date): the lead was found while it was open, and scoring and the user decide.
fn structured(html: &str, page_url: &str) -> Structured {
    let Ok(base) = url::Url::parse(page_url) else { return Structured::None };
    let postings = jsonld::job_postings(html, &base);
    let [one] = postings.as_slice() else { return Structured::None };
    if one.description_text().trim().chars().count() >= MIN_STRUCTURED_CHARS {
        return Structured::Full(jsonld::posting_text(one));
    }
    match jsonld::header_text(one) {
        header if header.is_empty() => Structured::None,
        header => Structured::Header(header),
    }
}

/// The readable text of the page at `url`. The answers `pipeline::ingest` gives: [`NeedsBrowser`]
/// for a bot check or login wall (HTTP 401, 403, 429, 503) and for a page with too little text
/// (rendered by JavaScript); any other failure is an ordinary error (its text names the status or
/// the network problem, never the URL).
pub(crate) async fn posting_text(http: &Http, url: &str) -> Result<String> {
    let parsed = checked_url(url)?;
    let host = parsed.host_str().unwrap_or_default().to_string();
    let url = parsed.to_string();
    let resp = match http.get(&url, &[("Accept", ACCEPT)]).await {
        Ok(resp) => resp,
        Err(FetchError::Status(code @ (401 | 403 | 429 | 503))) => {
            return Err(anyhow::Error::new(NeedsBrowser {
                url,
                reason: format!("the server answered HTTP {code} (bot check or login wall)"),
            }))
        }
        Err(e) => return Err(anyhow::Error::new(e)).with_context(|| format!("fetching {host}")),
    };
    let content_type = resp.content_type.to_ascii_lowercase();
    let mut header = String::new();
    if content_type.contains("html") || content_type.is_empty() {
        match structured(&String::from_utf8_lossy(&resp.body), &resp.url) {
            Structured::Full(text) => return Ok(text),
            Structured::Header(h) => header = h,
            Structured::None => {}
        }
    }
    // The header does not count toward the page's own length: a page that needs a browser still does.
    let text = page_text(&url, &resp.url, &resp.content_type, resp.body).await?;
    Ok(jsonld::with_header(&header, &text))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::engine::net::fixtures::Fixtures;
    use crate::engine::net::http::HttpLimits;

    fn limits() -> HttpLimits {
        HttpLimits { host_delay: Duration::ZERO, max_bytes: MAX_POSTING_BYTES, allow_private_hosts: true, ..HttpLimits::default() }
    }

    fn client(limits: &HttpLimits) -> Http {
        Http::new(limits).unwrap()
    }

    fn job_page() -> String {
        let body = "We build a sync engine in Rust and need an engineer who likes databases, networks and clear writing. ".repeat(8);
        format!("<!doctype html><html><head><title>Backend Engineer</title></head><body><main><h1>Backend Engineer</h1><p>{body}</p></main></body></html>")
    }

    #[tokio::test]
    async fn a_page_becomes_text_and_a_js_page_needs_a_browser() {
        let fx = Fixtures::start().await;
        fx.route("/job", 200, "text/html; charset=utf-8", job_page())
            .route("/spa", 200, "text/html", "<html><body><div id=root></div><script src=app.js></script></body></html>")
            .route("/plain", 200, "text/plain", "Senior engineer wanted. ".repeat(40));
        let http = client(&limits());
        let text = posting_text(&http, &format!("{}/job", fx.base())).await.unwrap();
        assert!(text.contains("sync engine in Rust"), "{text}");
        assert!(fx.hits_on("/job")[0].header("accept").is_some_and(|a| a.starts_with("text/html")));

        let err = posting_text(&http, &format!("{}/spa", fx.base())).await.unwrap_err();
        assert!(err.downcast_ref::<NeedsBrowser>().is_some_and(|nb| nb.reason.contains("JavaScript")), "{err:#}");

        assert!(posting_text(&http, &format!("{}/plain", fx.base())).await.unwrap().starts_with("Senior engineer wanted."));
    }

    #[tokio::test]
    async fn walls_need_a_browser_and_other_statuses_are_plain_errors() {
        let fx = Fixtures::start().await;
        for code in [401u16, 403, 429, 503] {
            fx.route(&format!("/s{code}"), code, "text/html", "blocked");
        }
        fx.route("/gone", 404, "text/html", "nope");
        let http = client(&limits());
        for code in [401, 403, 429, 503] {
            let err = posting_text(&http, &format!("{}/s{code}", fx.base())).await.unwrap_err();
            assert!(err.downcast_ref::<NeedsBrowser>().is_some(), "{code}: {err:#}");
        }
        let err = posting_text(&http, &format!("{}/gone", fx.base())).await.unwrap_err();
        assert!(err.downcast_ref::<NeedsBrowser>().is_none() && format!("{err:#}").contains("HTTP 404"), "{err:#}");
        // A dead host reads as a network problem (the thread fails and offers Retry).
        let err = format!("{:#}", posting_text(&http, "http://127.0.0.1:1/x").await.unwrap_err());
        assert_eq!(crate::engine::health::classify(&err), crate::engine::health::Problem::Network, "{err}");
        assert!(!err.contains("127.0.0.1:1/x"), "the URL is not echoed: {err}");
    }

    #[tokio::test]
    async fn the_fetch_refuses_private_hosts_on_every_hop_and_caps_the_body() {
        let (public, private) = (Fixtures::start().await, Fixtures::start().await);
        public.route("/job", 200, "text/html", job_page());
        public.redirect("/to-private", 302, &format!("{}/secret", private.base()));
        private.route("/secret", 200, "text/html", job_page());
        // Only `public` may be asked: `private` stands for an address on the local network.
        let port = url::Url::parse(public.base()).unwrap().port();
        let strict_limits = HttpLimits { allow_private_hosts: false, ..limits() };
        let strict = client(&strict_limits);
        let err = posting_text(&strict, &format!("{}/job", public.base())).await.unwrap_err();
        assert!(format!("{err:#}").contains("private network"), "the real check refuses 127.0.0.1: {err:#}");
        assert!(public.hits().is_empty());

        let gated = Http::with_host_check(&limits(), move |u| u.port() == port).unwrap();
        let err = posting_text(&gated, &format!("{}/to-private", public.base())).await.unwrap_err();
        assert!(format!("{err:#}").contains("private network"), "{err:#}");
        assert!(private.hits().is_empty(), "the redirect target was never requested");

        let tiny = client(&HttpLimits { max_bytes: 300, ..limits() });
        let err = posting_text(&tiny, &format!("{}/job", public.base())).await.unwrap_err();
        assert!(format!("{err:#}").contains("larger than"), "{err:#}");
    }

    #[tokio::test]
    async fn a_redirect_to_another_origin_is_followed_for_a_page() {
        let (a, b) = (Fixtures::start().await, Fixtures::start().await);
        a.redirect("/apply", 301, &format!("{}/job", b.base()));
        b.route("/job", 200, "text/html", job_page());
        let text = posting_text(&client(&limits()), &format!("{}/apply", a.base())).await.unwrap();
        assert!(text.contains("sync engine in Rust"));
    }

    /// [`job_page`] with a `JobPosting` whose description is `description`.
    fn job_page_with_teaser(description: &str) -> String {
        let ld = format!(
            r#"<script type="application/ld+json">{{"@type":"JobPosting","title":"Backend Engineer","jobLocation":{{"address":{{"addressLocality":"Hanoi","addressCountry":"VN"}}}},"baseSalary":{{"currency":"USD","value":{{"minValue":50000,"maxValue":70000,"unitText":"YEAR"}}}},"validThrough":"2020-01-31","description":"{description}"}}</script></head>"#
        );
        job_page().replace("</head>", &ld)
    }

    #[tokio::test]
    async fn a_page_with_one_job_posting_is_read_from_its_json_ld() {
        let fx = Fixtures::start().await;
        let posting = include_str!("fixtures/posting_jsonld.html");
        let two = posting.replace("</head>", r#"<script type="application/ld+json">{"@type":"JobPosting","title":"Another","description":"<p>Another job with a long enough description to count as one.</p>"}</script></head>"#);
        let teaser = posting.replacen(r#""description": "<p>Acme"#, r#""description": "<p>Join our Rust team in Saigon.</p>", "x": "<p>Acme"#, 1);
        fx.route("/one", 200, "text/html; charset=utf-8", posting)
            .route("/two", 200, "text/html", two)
            .route("/teaser-empty-page", 200, "text/html", teaser)
            .route("/teaser", 200, "text/html", job_page_with_teaser("<p>Join our sync team.</p>"))
            .route("/none", 200, "text/html", job_page());
        let http = client(&limits());
        let text = posting_text(&http, &format!("{}/one", fx.base())).await.unwrap();
        assert!(
            text.starts_with("Title: Senior Rust Engineer\nCompany: Acme\nLocation: Ho Chi Minh City, HCMC, VN\nRemote: yes\nEmployment type: Full time\nSalary: USD 60000-90000 per year\nPosted: 2026-10-20\nValid through: 2099-12-31\n\n"),
            "{text}"
        );
        assert!(text.contains("replication protocol") && text.contains("Profile and tune SQLite") && !text.contains("Cookie settings"), "{text}");

        // A teaser description: the page itself is read, with the header first (an expired
        // posting is still read; its header says so).
        let text = posting_text(&http, &format!("{}/teaser", fx.base())).await.unwrap();
        assert!(text.starts_with("Title: Backend Engineer\nLocation: Hanoi, VN\nSalary: USD 50000-70000 per year\nValid through: 2020-01-31\n\n"), "{text}");
        assert!(text.contains("sync engine in Rust") && !text.contains("Join our sync team"), "{text}");

        // Two postings, or a teaser on a page with no text of its own: the page itself is read,
        // and that page is mostly empty without its JSON-LD, so it needs the browser.
        for page in ["/two", "/teaser-empty-page"] {
            let err = posting_text(&http, &format!("{}{page}", fx.base())).await.unwrap_err();
            assert!(err.downcast_ref::<NeedsBrowser>().is_some(), "{page}: {err:#}");
        }
        let text = posting_text(&http, &format!("{}/none", fx.base())).await.unwrap();
        assert!(text.contains("sync engine in Rust") && !text.starts_with("Title:"), "{text}");
    }

    #[test]
    fn the_rendered_page_takes_the_same_json_ld_path() {
        let posting = include_str!("fixtures/posting_jsonld.html");
        let url = "https://acme.test/careers/senior-rust-engineer";
        let text = rendered_text(posting, url);
        assert!(text.starts_with("Title: Senior Rust Engineer\n") && text.contains("replication protocol"), "{text}");
        let text = rendered_text(&job_page_with_teaser("Short."), url);
        assert!(text.starts_with("Title: Backend Engineer\n") && text.contains("sync engine in Rust"), "{text}");
        let text = rendered_text(&job_page(), url);
        assert!(!text.starts_with("Title:") && text.contains("sync engine in Rust"), "{text}");
        assert_eq!(structured(posting, "not a url"), Structured::None);
        assert_eq!(structured("<html><body>No data</body></html>", url), Structured::None);
        let empty = r#"<script type="application/ld+json">{"@type":"JobPosting","description":"Hi"}</script>"#;
        assert_eq!(structured(empty, url), Structured::None, "nothing to add");
    }

    #[tokio::test]
    async fn only_http_links_are_fetched() {
        let http = client(&limits());
        for bad in ["file:///etc/passwd", "ftp://x.test/job", "not a url", ""] {
            assert!(posting_text(&http, bad).await.is_err(), "{bad}");
        }
    }
}
