//! The default posting fetch: a page's readable text through the shared safe HTTP client
//! ([`crate::net::http`]), so a posting URL taken from third-party data (an HN comment, a board's
//! `absolute_url`) gets the same checks as every other request: public hosts only on every hop,
//! the per-host delay and a size cap while streaming. Text extraction is the one of
//! `pipeline::ingest`.

use anyhow::{Context, Result};
use futures::FutureExt;

use super::deps::PostingFetcher;
use crate::net::http::{checked_url, FetchError, Http};
use crate::pipeline::ingest::{page_text, NeedsBrowser};

/// Largest page read (the adapters' JSON answers have their own, larger cap).
pub(crate) const MAX_POSTING_BYTES: usize = 10 * 1024 * 1024;

/// `Accept` only: any other header would make the client refuse a redirect to another origin
/// (a posting link often goes through the company's own domain first).
const ACCEPT: &str = "text/html,application/xhtml+xml,application/pdf;q=0.9,*/*;q=0.5";

/// A [`PostingFetcher`] reading through `http` (its cap should be [`MAX_POSTING_BYTES`]).
pub(crate) fn fetcher(http: Http) -> PostingFetcher {
    std::sync::Arc::new(move |req| {
        let http = http.clone();
        async move { posting_text(&http, &req.candidate.url).await }.boxed()
    })
}

/// The readable text of the page at `url`. The answers `pipeline::ingest` gives: [`NeedsBrowser`]
/// for a bot check or login wall (HTTP 401, 403, 429, 503) and for a page with too little text
/// (rendered by JavaScript); any other failure is an ordinary error (its text names the status or
/// the network problem, never the URL).
pub(crate) async fn posting_text(http: &Http, url: &str) -> Result<String> {
    let url = checked_url(url)?.to_string();
    let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
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
    page_text(&url, &resp.url, &resp.content_type, resp.body).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::net::fixtures::Fixtures;
    use crate::net::http::HttpLimits;

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
        assert_eq!(crate::health::classify(&err), crate::health::Problem::Network, "{err}");
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

    #[tokio::test]
    async fn only_http_links_are_fetched() {
        let http = client(&limits());
        for bad in ["file:///etc/passwd", "ftp://x.test/job", "not a url", ""] {
            assert!(posting_text(&http, bad).await.is_err(), "{bad}");
        }
    }
}
