//! A page or PDF the user (or a search result, or a feed entry) points at, fetched with plain HTTP
//! and cleaned like a job description: Readability for HTML, `pdf-extract` for PDFs.
//!
//! There is no browser fallback here: a page that needs JavaScript yields too little text and is
//! reported as such; a login or bot wall (HTTP 401/403/429/503) is a challenge (see `collect.rs`).

use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};

use super::{normalize_url, Ctx, Item};
use crate::pipeline::ingest::{clean, extract_html, pdf_text};
use crate::trends::http::Response;
use crate::trends::text::{cap, first_line};

/// A page with less readable text than this is a shell (JavaScript app, paywall, error page).
const MIN_TEXT_CHARS: usize = 200;
const ACCEPT: &str = "text/html,application/xhtml+xml,application/pdf;q=0.9,text/plain;q=0.8,*/*;q=0.5";

/// What was read at an address.
#[derive(Debug)]
pub struct Page {
    /// The address after redirects.
    pub url: String,
    pub text: String,
}

/// The readable text at `url`.
pub async fn fetch_text(ctx: &Ctx, url: &str) -> Result<Page> {
    let Response { url, content_type, body } = ctx.http.get(url, &[("Accept", ACCEPT)]).await?;
    let text = readable(&content_type, &url, body, ctx.limits().max_text_chars).await?;
    ensure!(
        text.chars().count() >= MIN_TEXT_CHARS,
        "the page has too little readable text ({} characters; it may need a browser to render)",
        text.chars().count()
    );
    Ok(Page { url, text })
}

/// The text of a response body (moved in: it is not copied).
async fn readable(content_type: &str, url: &str, body: Vec<u8>, max_chars: usize) -> Result<String> {
    let kind = content_type.split(';').next().unwrap_or_default().trim();
    let text = if kind == "application/pdf" || body.starts_with(b"%PDF-") {
        let raw = tokio::task::spawn_blocking(move || pdf_text(&body)).await.context("the PDF reader stopped")??;
        clean(&raw)
    } else if kind.is_empty() || kind.contains("html") || kind.contains("xml") {
        let html = String::from_utf8(body).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned());
        let url = url.to_string();
        tokio::task::spawn_blocking(move || extract_html(&html, Some(&url))).await.context("the page reader stopped")?
    } else if kind.starts_with("text/") {
        clean(&String::from_utf8_lossy(&body))
    } else {
        bail!("unsupported content type `{kind}`");
    };
    Ok(cap(&text, max_chars))
}

/// An item for `url`. The text starts with a `# title` line: the hint when there is one, else the
/// heading the page already has, else its first line.
pub(crate) fn make_item(url: String, title_hint: Option<&str>, published_at: Option<DateTime<Utc>>, text: String) -> Item {
    let hint = title_hint.map(str::trim).filter(|t| !t.is_empty());
    let (title, text) = if text.starts_with("# ") {
        (hint.map(str::to_string).or_else(|| first_line(&text, 140)), text)
    } else if let Some(hint) = hint {
        let text = format!("# {hint}\n\n{text}");
        (Some(hint.to_string()), text)
    } else {
        (first_line(&text, 140), text)
    };
    Item { title: title.unwrap_or_else(|| url.clone()), url, published_at, text }
}

/// The user's page source: one item for its address (a page already collected is not read again).
pub async fn collect(ctx: &Ctx, source_url: &str) -> Result<Vec<Item>> {
    let url = normalize_url(source_url)?;
    if ctx.is_known(&url) {
        return Ok(Vec::new());
    }
    let page = fetch_text(ctx, &url).await?;
    Ok(vec![make_item(url, None, None, page.text)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::ingest::tests::JOB_HTML;
    use crate::trends::http::FetchError;
    use crate::trends::testutil::{ctx, typst_pdf, Fixtures};

    #[tokio::test]
    async fn html_text_and_pdf_pages_are_read() {
        let report = "= Annual report 2026\n".to_string()
            + &"Cloud spending rose again in 2026 and platform engineering teams grew faster than any other group. ".repeat(6);
        let fx = Fixtures::start().await;
        fx.route("/job", 200, "text/html; charset=utf-8", JOB_HTML);
        fx.route("/plain", 200, "text/plain", format!("Notes on hiring.\r\n\r\n\r\n{}", "Rust demand keeps growing. ".repeat(20)));
        fx.route("/report.pdf", 200, "application/pdf", typst_pdf(&report));
        fx.route("/blob", 200, "application/octet-stream", "xx");
        let ctx = ctx(&fx, &[]);

        let job = fetch_text(&ctx, &format!("{}/job", fx.base())).await.unwrap();
        assert!(job.text.contains("Senior Rust Engineer") && job.text.contains("Kubernetes"), "{}", job.text);
        assert!(!job.text.contains("do not show this"), "scripts are dropped");

        let plain = fetch_text(&ctx, &format!("{}/plain", fx.base())).await.unwrap();
        assert!(plain.text.starts_with("Notes on hiring.\n\nRust demand"), "{}", plain.text);

        let pdf = fetch_text(&ctx, &format!("{}/report.pdf", fx.base())).await.unwrap();
        let flat = pdf.text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("Annual report 2026") && flat.contains("platform engineering teams grew"), "{flat}");

        let err = fetch_text(&ctx, &format!("{}/blob", fx.base())).await.unwrap_err().to_string();
        assert!(err.contains("unsupported content type"), "{err}");
    }

    #[tokio::test]
    async fn the_host_guard_cannot_be_skipped_by_a_caller() {
        let fx = Fixtures::start().await;
        fx.route("/job", 200, "text/html", JOB_HTML);
        let mut strict = ctx(&fx, &[]);
        let mut config = (*strict.config).clone();
        config.limits.allow_private_hosts = false;
        strict.http = crate::trends::http::Http::new(&config.limits).unwrap();
        strict.config = std::sync::Arc::new(config);

        let err = fetch_text(&strict, &format!("{}/job", fx.base())).await.unwrap_err();
        assert_eq!(err.downcast_ref::<FetchError>(), Some(&FetchError::Private));
        assert!(collect(&strict, &format!("{}/job", fx.base())).await.is_err());
        assert!(fx.hits().is_empty());
    }

    #[tokio::test]
    async fn shells_and_walls_are_told_apart() {
        let fx = Fixtures::start().await;
        fx.route("/spa", 200, "text/html", "<html><body><div id=root></div><script src=app.js></script></body></html>");
        fx.route("/wall", 403, "text/html", "blocked");
        fx.route("/login", 401, "text/html", "sign in");
        fx.route("/bad.pdf", 200, "application/pdf", "not a pdf");
        let ctx = ctx(&fx, &[]);

        let err = fetch_text(&ctx, &format!("{}/spa", fx.base())).await.unwrap_err().to_string();
        assert!(err.contains("too little readable text"), "{err}");
        let err = fetch_text(&ctx, &format!("{}/wall", fx.base())).await.unwrap_err();
        assert_eq!(err.downcast_ref::<FetchError>(), Some(&FetchError::Status(403)));
        let err = fetch_text(&ctx, &format!("{}/login", fx.base())).await.unwrap_err();
        assert_eq!(err.downcast_ref::<FetchError>(), Some(&FetchError::Status(401)));
        let err = fetch_text(&ctx, &format!("{}/bad.pdf", fx.base())).await.unwrap_err().to_string();
        assert!(err.contains("PDF"), "{err}");
    }

    #[tokio::test]
    async fn a_page_source_gives_one_item_and_a_known_page_is_not_fetched() {
        let fx = Fixtures::start().await;
        fx.route("/job", 200, "text/html", JOB_HTML);
        let url = format!("{}/job#section", fx.base());
        let items = collect(&ctx(&fx, &[]), &url).await.unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].url, format!("{}/job", fx.base()), "fragment dropped");
        assert!(items[0].text.starts_with("# Senior Rust Engineer"), "{}", items[0].text);
        assert_eq!(items[0].title, "Senior Rust Engineer - Acme Robotics", "the page's own title");
        assert_eq!(fx.hits().len(), 1);

        let known = format!("{}/job", fx.base());
        assert!(collect(&ctx(&fx, &[&known]), &url).await.unwrap().is_empty());
        assert_eq!(fx.hits().len(), 1, "no second request");
    }

    #[test]
    fn titles_come_from_the_hint_the_heading_or_the_first_line() {
        let a = make_item("https://x.test/a".into(), Some(" Hint "), None, "body text".into());
        assert_eq!((a.title.as_str(), a.text.as_str()), ("Hint", "# Hint\n\nbody text"));
        let b = make_item("https://x.test/b".into(), None, None, "# Heading\n\nbody".into());
        assert_eq!((b.title.as_str(), b.text.as_str()), ("Heading", "# Heading\n\nbody"));
        let c = make_item("https://x.test/c".into(), None, None, "\nFirst line\nsecond".into());
        assert_eq!((c.title.as_str(), c.text.as_str()), ("First line", "\nFirst line\nsecond"));
        let d = make_item("https://x.test/d".into(), None, None, String::new());
        assert_eq!(d.title, "https://x.test/d");
    }
}
