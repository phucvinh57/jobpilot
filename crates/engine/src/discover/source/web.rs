//! The web-page side of the browser adapters (`yc`, `career_page`): reading a listing page, finding
//! the links on it that are postings, and the same-site rule.
//!
//! **Code decides which pages are visited.** A link is only followed when it is on the same site as
//! the address the user configured ([`same_site`]) and its address looks like a posting
//! ([`looks_like_posting`]); the page's text never chooses a URL, and neither does the model.
//!
//! A listing is read with a plain HTTP request first; only when that fails with a wall (HTTP 401, 403,
//! 406, 429, 503) or the page has no postings in it (a job list that JavaScript draws) is it rendered
//! in the browser (the user's signed-in profile when the source has a session). A login, captcha or block
//! page shown there surfaces as [`ChallengeFound`], so the run waits for the user.

use std::collections::HashMap;
use std::sync::OnceLock;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use regex::Regex;
use url::Url;

use super::{html_to_text, one_line, Candidate};
use crate::browser::challenge::ChallengeFound;
use crate::browser::render::Renderer;
use crate::mail::classify::{is_ats_host, registrable_domain};
use crate::net::http::{checked_url, FetchError, Http};
use crate::pipeline::ingest::NeedsBrowser;
use crate::store::Source;

/// A page an adapter read: its address after redirects, and its HTML.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub url: String,
    pub html: String,
}

/// How an adapter reads web pages, so tests answer from fixtures and never touch the network.
#[async_trait]
pub trait Pages: Send + Sync {
    /// A plain HTTP read. A wall (a bot check or login answer) is `Err` carrying [`NeedsBrowser`].
    async fn plain(&self, url: &str) -> Result<Page>;

    /// The page rendered in the browser (the signed-in profile if the source has a session). `Err` carrying
    /// [`ChallengeFound`] when a login, captcha or block page was shown instead.
    async fn rendered(&self, source: &Source, url: &str) -> Result<Page>;
}

/// `Accept` only: any other header would make the client refuse a redirect to another origin.
const ACCEPT: &str = "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5";

/// The real page reader: the shared safe HTTP client, and the browser when there is one.
pub struct NetPages {
    http: Http,
    renderer: Option<Renderer>,
}

impl NetPages {
    pub fn new(http: Http, renderer: Option<Renderer>) -> Self {
        Self { http, renderer }
    }
}

#[async_trait]
impl Pages for NetPages {
    async fn plain(&self, url: &str) -> Result<Page> {
        let parsed = checked_url(url)?;
        let host = parsed.host_str().unwrap_or_default().to_string();
        let url = parsed.to_string();
        match self.http.get(&url, &[("Accept", ACCEPT)]).await {
            Ok(resp) => Ok(Page { url: resp.url.clone(), html: resp.text() }),
            Err(FetchError::Status(code @ (401 | 403 | 406 | 429 | 503))) => Err(anyhow::Error::new(NeedsBrowser {
                url,
                reason: format!("the server answered HTTP {code} (bot check or login wall)"),
            })),
            Err(e) => Err(anyhow::Error::new(e)).with_context(|| format!("fetching {host}")),
        }
    }

    async fn rendered(&self, source: &Source, url: &str) -> Result<Page> {
        let renderer = self.renderer.as_ref().context("this page needs a browser to render, and none is set up")?;
        let rendered = renderer.render(source.session_id, url).await?;
        if let Some(kind) = rendered.challenge {
            return Err(anyhow::Error::new(ChallengeFound { kind, url: url.to_string() }));
        }
        Ok(Page { url: rendered.url, html: rendered.html })
    }
}

/// The candidates on a listing page: plain first; the browser when the plain read met a wall or
/// found no posting in the page. `extract` turns a page into candidates. A [`ChallengeFound`] or any
/// other error from the browser is the caller's error.
pub(crate) async fn read_listing(
    pages: &dyn Pages,
    source: &Source,
    url: &str,
    extract: impl Fn(&Page) -> Vec<Candidate>,
) -> Result<Vec<Candidate>> {
    if let Some(page) = read_plain_page(pages, url).await? {
        let found = extract(&page);
        if !found.is_empty() {
            return Ok(found);
        }
    }
    let page = pages.rendered(source, url).await?;
    Ok(extract(&page))
}

/// The plain read of [`read_listing`]: `None` when it met a wall (the browser may do better), any
/// other failure as the error.
pub(crate) async fn read_plain_page(pages: &dyn Pages, url: &str) -> Result<Option<Page>> {
    match pages.plain(url).await {
        Ok(page) => Ok(Some(page)),
        Err(e) if e.downcast_ref::<NeedsBrowser>().is_none() => Err(e),
        Err(_) => Ok(None),
    }
}

// ---------------------------------------------------------------- links

/// An anchor of a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    /// Absolute, without fragment.
    pub url: Url,
    /// Its visible text on one line (may be empty).
    pub text: String,
}

/// Longest link text kept.
const TITLE_CHARS: usize = 160;

/// The links of `html`, resolved against `base`: http(s) only, no fragments, each address once
/// (the first one that has a text keeps it), in page order.
pub(crate) fn links(html: &str, base: &Url) -> Vec<Link> {
    static ANCHOR: OnceLock<Regex> = OnceLock::new();
    let anchor = ANCHOR.get_or_init(|| {
        Regex::new(r#"(?is)<a\b[^>]*?\bhref\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))[^>]*>(.*?)</a\s*>"#).expect("static regex")
    });
    let mut out: Vec<Link> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for caps in anchor.captures_iter(html) {
        let href = caps.get(1).or_else(|| caps.get(2)).or_else(|| caps.get(3)).map(|m| m.as_str().trim()).unwrap_or_default();
        let Some(mut url) = resolve(base, href) else { continue };
        url.set_fragment(None);
        let inner = caps.get(4).map(|m| m.as_str()).unwrap_or_default();
        let text = one_line(&html_to_text(&strip_tags_keep_words(inner)), TITLE_CHARS);
        match at.get(url.as_str()) {
            Some(&i) => {
                if out[i].text.is_empty() || is_generic(&out[i].text) {
                    out[i].text = text;
                }
            }
            None => {
                at.insert(url.to_string(), out.len());
                out.push(Link { url, text });
            }
        }
    }
    out
}

/// `href` as an absolute http(s) address with a host, resolved against `base`; `None` for an empty
/// one, a fragment, and `javascript:`, `mailto:`, `tel:` and `data:` links.
pub(super) fn resolve(base: &Url, href: &str) -> Option<Url> {
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    let lower = href.to_ascii_lowercase();
    if ["javascript:", "mailto:", "tel:", "data:"].iter().any(|p| lower.starts_with(p)) {
        return None;
    }
    let url = base.join(href).ok()?;
    (matches!(url.scheme(), "http" | "https") && url.host_str().is_some()).then_some(url)
}

/// Inner HTML of an anchor with its tags replaced by spaces, so `<h3>Title</h3><p>Where</p>` does
/// not run together.
fn strip_tags_keep_words(inner: &str) -> String {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| Regex::new(r"(?s)<[^>]*>").expect("static regex"));
    tag.replace_all(inner, " ").into_owned()
}

/// A link text that says nothing about the job.
pub(crate) fn is_generic(text: &str) -> bool {
    let t: String = text.trim().to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    t.is_empty()
        || matches!(
            t.as_str(),
            "apply" | "apply now" | "apply here" | "learn more" | "read more" | "more" | "details" | "view" | "view job"
                | "view details" | "view role" | "see details" | "see more" | "view all jobs" | "see all jobs" | "all jobs"
                | "next" | "previous" | "back" | "open" | "link"
        )
}

// ---------------------------------------------------------------- the same-site rule

/// Hosts without a leading `www.`.
fn bare_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().trim_end_matches('.').to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// Hosting platforms where every customer is a subdomain: the platform's own name stands for all of
/// them. (Not exhaustive: a customer's own subdomain, `acme.github.io`, is fine.)
const PLATFORM_ROOTS: [&str; 24] = [
    "github.io", "gitlab.io", "vercel.app", "netlify.app", "pages.dev", "workers.dev", "herokuapp.com", "onrender.com", "fly.dev",
    "web.app", "firebaseapp.com", "appspot.com", "azurewebsites.net", "cloudfront.net", "amazonaws.com", "blogspot.com",
    "wordpress.com", "wixsite.com", "squarespace.com", "webflow.io", "notion.site", "myshopify.com", "carrd.co", "glitch.me",
];

/// Why `host` (without `www.`) cannot be the address of a site at all: a single-word name or a
/// public suffix (`co.uk`). `None` for any other name (a company's own domain, a hosting platform, a
/// job board) and an IP address.
pub(crate) fn not_a_site(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    if !host.contains('.') {
        return Some("a single-word name, not a company site");
    }
    // `x.<host>` is its own registrable domain only under a public suffix (`co.uk`, `com.vn`).
    let probe = format!("x.{host}");
    if registrable_domain(&probe) == probe {
        return Some("a public domain ending shared by many sites");
    }
    None
}

/// Is `host` (without `www.`) shared by many companies, so that its subdomains are other sites: a
/// hosting platform's own name (`github.io`, while `acme.github.io` is one customer's) or a job board
/// or aggregator (`linkedin.com`, `boards.greenhouse.io`, `apply.workable.com`)? Such a page is a job
/// source like any other, but only its exact host is "the same site" ([`same_site`]).
pub(crate) fn is_shared_host(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    PLATFORM_ROOTS.contains(&host.as_str()) || is_ats_host(&host)
}

/// Fail for an address that is not a site at all ([`not_a_site`]). Any other page may be a job
/// source: a company's careers page, a job board, an aggregator's search page. (A board address
/// pasted as a job page becomes that board's source in [`super::check_source`].)
pub(crate) fn check_site(url: &Url) -> Result<()> {
    let host = url.host_str().unwrap_or_default();
    match not_a_site(&bare_host(url)) {
        None => Ok(()),
        Some(why) => bail!("`{host}` is {why}: enter the address of a page that lists jobs"),
    }
}

/// Is `link` on the site `site`, the address the user **configured**? The same host (ignoring
/// `www.`), or a subdomain of it (`jobs.acme.com` under `acme.com`). The parent of a configured
/// subdomain, its sibling subdomains and look-alike names (`notacme.com`) are other sites. On a host
/// shared by many companies ([`is_shared_host`]: `linkedin.com`, `github.io`) only that exact host
/// counts, never a subdomain (another tenant); a `site` that is no site at all ([`not_a_site`]) has
/// no same-site links. Redirects and links found on a page do not move `site`.
pub(crate) fn same_site(site: &Url, link: &Url) -> bool {
    let (site, link) = (bare_host(site), bare_host(link));
    if site.is_empty() || not_a_site(&site).is_some() {
        return false;
    }
    if is_shared_host(&site) {
        return link == site;
    }
    link == site || link.ends_with(&format!(".{site}"))
}

/// Path words that mark a place where jobs live.
pub(crate) const JOB_WORDS: [&str; 14] = [
    "job", "jobs", "career", "careers", "position", "positions", "opening", "openings", "vacancy", "vacancies", "posting",
    "postings", "role", "roles",
];

/// Does `link` look like one posting of the listing at `listing`? On the configured `site` (see
/// [`same_site`]), with a path that goes below a jobs-ish segment (`/careers/senior-engineer`,
/// `/jobs/4011001`) or ends in a long id under a jobs-ish path, and not the listing itself nor a
/// file. Anything else (about pages, the footer, `mailto:`) is not a posting; the model's scoring
/// weeds out what slips through.
pub(crate) fn looks_like_posting(site: &Url, listing: &Url, link: &Url) -> bool {
    if !same_site(site, link) {
        return false;
    }
    let segments: Vec<String> =
        link.path_segments().map(|s| s.filter(|p| !p.is_empty()).map(str::to_ascii_lowercase).collect()).unwrap_or_default();
    let listing_segments: Vec<String> =
        listing.path_segments().map(|s| s.filter(|p| !p.is_empty()).map(str::to_ascii_lowercase).collect()).unwrap_or_default();
    if segments.is_empty() || segments == listing_segments {
        return false;
    }
    if let Some(last) = segments.last() {
        let file = last.rsplit_once('.').map(|(_, ext)| ext).unwrap_or_default();
        if ["css", "js", "png", "jpg", "jpeg", "gif", "svg", "ico", "webp", "woff", "woff2", "xml", "json", "zip"].contains(&file) {
            return false;
        }
    }
    // Listing pagination and search results are listings, not postings.
    if let Some(last) = segments.last() {
        if matches!(last.as_str(), "search" | "page" | "all" | "teams" | "locations" | "benefits" | "culture" | "life" | "internships") {
            return false;
        }
    }
    let is_job_word = |segment: &str| segment.split(['-', '_']).any(|token| JOB_WORDS.contains(&token));
    match segments.iter().position(|s| is_job_word(s)) {
        // Something below the jobs-ish segment.
        Some(i) if i + 1 < segments.len() => true,
        // The jobs-ish segment is the last one: a posting only when it carries an id (`/open-role-4011001`).
        _ => segments.last().is_some_and(|last| has_id(last) && is_job_word(last)),
    }
}

/// A long number or a UUID inside a path segment.
fn has_id(segment: &str) -> bool {
    static ID: OnceLock<Regex> = OnceLock::new();
    let id = ID.get_or_init(|| {
        Regex::new(r"[0-9]{4,}|[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").expect("static regex")
    });
    id.is_match(segment)
}

/// A title for a link without a usable text: the last path segment's words without a leading id
/// (`/jobs/aaMaqQu-software-engineering-intern` -> "software engineering intern"). `None` when the
/// segment has no words.
pub(crate) fn title_from_url(url: &Url) -> Option<String> {
    let last = url.path_segments()?.rfind(|s| !s.is_empty())?;
    let words: Vec<&str> = last.split(['-', '_', '+']).filter(|w| !w.is_empty()).collect();
    // `aaMaqQu`, `4011001`: a leading token with a digit, or a short mixed-case id, is an id.
    let skip = usize::from(words.first().is_some_and(|w| w.chars().any(|c| c.is_ascii_digit()) || is_mixed_case_id(w)) && words.len() > 1);
    let title = words[skip..].join(" ");
    (title.chars().any(char::is_alphabetic)).then_some(title)
}

fn is_mixed_case_id(word: &str) -> bool {
    word.len() >= 5 && word.chars().skip(1).any(|c| c.is_ascii_uppercase()) && word.chars().any(|c| c.is_ascii_lowercase())
}

/// Parse a page's address.
pub(crate) fn page_url(page: &Page) -> Result<Url> {
    Url::parse(&page.url).map_err(|_| anyhow!("the page has no valid address"))
}

/// `https://` when the scheme is missing; http(s) only, with a host.
pub(crate) fn normalize_page_url(raw: &str) -> Result<Url> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("no page address is set: enter the address of a page that lists jobs");
    }
    let with_scheme = if raw.contains("://") { raw.to_string() } else { format!("https://{raw}") };
    let url = Url::parse(&with_scheme).map_err(|_| anyhow!("`{raw}` is not a valid address"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        bail!("the page address must be http or https");
    }
    Ok(url)
}

/// A fixture reader for tests: pages by URL for the plain read and for the browser, and the
/// challenges the browser meets.
#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::graph::nodes::ChallengeKind;

    #[derive(Default)]
    pub(crate) struct FakePages {
        plain: HashMap<String, Result<String, u16>>,
        rendered: HashMap<String, Result<String, ChallengeKind>>,
        /// Where a read of the key ends up after redirects.
        moved: HashMap<String, String>,
        asked: Mutex<Vec<String>>,
    }

    impl FakePages {
        pub(crate) fn new() -> Self {
            Self::default()
        }

        /// The plain read of `url` answers `html`.
        pub(crate) fn plain(mut self, url: &str, html: &str) -> Self {
            self.plain.insert(url.into(), Ok(html.into()));
            self
        }

        /// The plain read of `url` meets HTTP `status`.
        pub(crate) fn wall(mut self, url: &str, status: u16) -> Self {
            self.plain.insert(url.into(), Err(status));
            self
        }

        /// The browser renders `html` at `url`.
        pub(crate) fn rendered(mut self, url: &str, html: &str) -> Self {
            self.rendered.insert(url.into(), Ok(html.into()));
            self
        }

        /// The browser meets `kind` at `url`.
        pub(crate) fn challenge(mut self, url: &str, kind: ChallengeKind) -> Self {
            self.rendered.insert(url.into(), Err(kind));
            self
        }

        /// A read of `url` (plain or in the browser) redirects and ends at `to`.
        pub(crate) fn moved(mut self, url: &str, to: &str) -> Self {
            self.moved.insert(url.into(), to.into());
            self
        }

        fn final_url(&self, url: &str) -> String {
            self.moved.get(url).cloned().unwrap_or_else(|| url.to_string())
        }

        /// Every read so far: `plain <url>` or `browser <url>`.
        pub(crate) fn asked(&self) -> Vec<String> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    #[async_trait]
    impl Pages for FakePages {
        async fn plain(&self, url: &str) -> Result<Page> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).push(format!("plain {url}"));
            match self.plain.get(url) {
                Some(Ok(html)) => Ok(Page { url: self.final_url(url), html: html.clone() }),
                Some(Err(status)) => Err(anyhow::Error::new(NeedsBrowser { url: url.to_string(), reason: format!("HTTP {status}") })),
                None => Err(anyhow!("fetching fixture: HTTP 404 Not Found")),
            }
        }

        async fn rendered(&self, _source: &Source, url: &str) -> Result<Page> {
            self.asked.lock().unwrap_or_else(|e| e.into_inner()).push(format!("browser {url}"));
            match self.rendered.get(url) {
                Some(Ok(html)) => Ok(Page { url: self.final_url(url), html: html.clone() }),
                Some(Err(kind)) => Err(anyhow::Error::new(ChallengeFound { kind: *kind, url: url.to_string() })),
                None => Err(anyhow!("no browser fixture")),
            }
        }
    }

    pub(crate) fn pages(p: FakePages) -> Arc<FakePages> {
        Arc::new(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn links_are_resolved_deduplicated_and_cleaned() {
        let html = r##"
            <a href="/careers/rust-engineer"><h3>Rust Engineer</h3><p>Remote</p></a>
            <a href='https://acme.test/careers/rust-engineer#apply'>Apply</a>
            <a class=x href=/careers/data-engineer>Data &amp; ML Engineer</a>
            <a href="mailto:jobs@acme.test">Mail us</a>
            <a href="javascript:void(0)">Menu</a>
            <a href="#top">Top</a>
            <a href="ftp://acme.test/x">ftp</a>
            <a href="/careers/empty"></a>
            <a href="https://other.test/jobs/1">Elsewhere</a>"##;
        let found = links(html, &url("https://acme.test/careers"));
        let shown: Vec<_> = found.iter().map(|l| (l.url.as_str(), l.text.as_str())).collect();
        assert_eq!(
            shown,
            [
                ("https://acme.test/careers/rust-engineer", "Rust Engineer Remote"),
                ("https://acme.test/careers/data-engineer", "Data & ML Engineer"),
                ("https://acme.test/careers/empty", ""),
                ("https://other.test/jobs/1", "Elsewhere"),
            ]
        );
    }

    #[test]
    fn a_generic_text_is_replaced_by_a_better_one_for_the_same_address() {
        let html = r#"<a href="/jobs/1">Apply now</a> <a href="/jobs/1">Staff Engineer</a> <a href="/jobs/1">Learn more</a>"#;
        let found = links(html, &url("https://acme.test/jobs"));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "Staff Engineer");
        assert!(is_generic("Apply Now!") && is_generic("  ") && is_generic("Read more") && !is_generic("Apply Engineer"));
    }

    #[test]
    fn the_same_site_rule_keeps_to_the_configured_site() {
        let site = url("https://www.acme.test/careers");
        for yes in ["https://acme.test/jobs/1", "https://www.acme.test/jobs/1", "https://careers.acme.test/jobs/1", "http://ACME.test/x", "https://a.b.acme.test/x"] {
            assert!(same_site(&site, &url(yes)), "{yes}");
        }
        for no in ["https://notacme.test/jobs/1", "https://acme.test.evil.test/jobs/1", "https://evil.test/acme.test", "https://boards.greenhouse.io/acme/jobs/1", "https://acme.com/jobs/1"] {
            assert!(!same_site(&site, &url(no)), "{no}");
        }
        // A configured subdomain: its own subdomains are the same site, its parent and siblings are not.
        let sub = url("https://careers.acme.test/");
        assert!(same_site(&sub, &url("https://careers.acme.test/x")) && same_site(&sub, &url("https://eu.careers.acme.test/x")));
        assert!(!same_site(&sub, &url("https://acme.test/x")), "the parent site is another site");
        assert!(!same_site(&sub, &url("https://www.acme.test/x")), "so is its www");
        assert!(!same_site(&sub, &url("https://blog.acme.test/x")), "and a sibling subdomain");
    }

    #[test]
    fn a_public_suffix_or_a_single_word_is_not_a_site() {
        // Nothing is "the same site" as one of these, and none of them is a job page.
        for none in ["https://co.uk/", "https://com.vn/", "https://com/", "https://localhost/", "https://intranet/"] {
            let s = url(none);
            let host = s.host_str().unwrap();
            assert!(!same_site(&s, &s), "{none}");
            assert!(!same_site(&s, &url(&format!("https://evil.{host}/jobs/1"))), "{none}: a subdomain of it");
            let err = check_site(&s).unwrap_err().to_string();
            assert!(err.contains("page that lists jobs"), "{none}: {err}");
        }
        assert!(not_a_site("acme.com").is_none() && not_a_site("co.uk").is_some() && not_a_site("greenhouse.io").is_none());
        assert!(not_a_site("127.0.0.1").is_none(), "an address is left to the network guard");
    }

    #[test]
    fn a_shared_host_is_a_job_page_whose_site_is_its_exact_host() {
        // Platforms and job boards hold many unrelated companies: any page of them may be a job
        // source, but its subdomains are other tenants.
        for shared in [
            "https://github.io/jobs", "https://vercel.app/", "https://www.netlify.app/", "https://herokuapp.com/",
            "https://boards.greenhouse.io/acme", "https://jobs.lever.co/acme", "https://www.linkedin.com/jobs",
            "https://apply.workable.com/acme/", "https://acme.wd1.myworkdayjobs.com/en-US/careers",
        ] {
            let s = url(shared);
            let host = s.host_str().unwrap().trim_start_matches("www.").to_string();
            assert!(check_site(&s).is_ok(), "{shared}");
            assert!(is_shared_host(&host), "{shared}");
            assert!(same_site(&s, &s), "{shared}: its own pages");
            assert!(same_site(&s, &url(&format!("https://www.{host}/jobs/view/1"))), "{shared}: with www.");
            assert!(!same_site(&s, &url(&format!("https://evil.{host}/jobs/1"))), "{shared}: a subdomain of it");
        }
        assert!(same_site(&url("https://www.linkedin.com/jobs/search?keywords=rust"), &url("https://linkedin.com/jobs/view/123")));
        assert!(!same_site(&url("https://www.linkedin.com/jobs"), &url("https://de.linkedin.com/jobs/view/123")));
        // A customer's own subdomain on a platform, and a company domain under a public suffix, are sites.
        assert!(same_site(&url("https://acme.github.io/"), &url("https://acme.github.io/jobs/1")));
        assert!(!same_site(&url("https://acme.github.io/"), &url("https://evil.github.io/jobs/1")), "another tenant of the platform");
        assert!(same_site(&url("https://acme.co.uk/careers"), &url("https://jobs.acme.co.uk/1")));
        assert!(!same_site(&url("https://acme.co.uk/careers"), &url("https://other.co.uk/1")));
        for ok in ["https://acme.test/careers", "https://acme.github.io/", "https://careers.acme.co.uk/", "http://127.0.0.1:8080/jobs", "https://greenhouse.io.acme.test/careers"] {
            assert!(check_site(&url(ok)).is_ok(), "{ok}");
        }
        assert!(!is_shared_host("acme.github.io") && !is_shared_host("acme.com") && !is_shared_host("greenhouse.io.acme.test"));
    }

    #[test]
    fn postings_are_the_links_below_a_jobs_path() {
        let listing = url("https://acme.test/careers");
        for yes in [
            "https://acme.test/careers/senior-rust-engineer",
            "https://acme.test/jobs/4011001",
            "https://acme.test/careers/engineering/backend-engineer",
            "https://acme.test/open-positions/data-engineer?gh_jid=1",
            "https://acme.test/company/careers/role-123456",
            "https://careers.acme.test/jobs/abc",
        ] {
            assert!(looks_like_posting(&listing, &listing, &url(yes)), "{yes}");
        }
        for no in [
            "https://acme.test/careers",
            "https://acme.test/careers/",
            "https://acme.test/about",
            "https://acme.test/blog/why-we-hire",
            "https://acme.test/careers/benefits",
            "https://acme.test/jobs/search",
            "https://acme.test/careers/logo.png",
            "https://acme.test/",
            "https://other.test/jobs/4011001",
            "https://acme.test/privacy",
        ] {
            assert!(!looks_like_posting(&listing, &listing, &url(no)), "{no}");
        }
        // The site is the configured one even when the listing was read at a redirected address.
        let moved = url("https://evil.test/careers");
        assert!(!looks_like_posting(&listing, &moved, &url("https://evil.test/careers/rust-engineer")));
        assert!(looks_like_posting(&listing, &moved, &url("https://acme.test/careers/rust-engineer")));
    }

    #[test]
    fn titles_come_from_the_address_without_the_id() {
        let t = |s: &str| title_from_url(&url(s));
        assert_eq!(t("https://yc.test/companies/aviator/jobs/aaMaqQu-software-engineering-intern").as_deref(), Some("software engineering intern"));
        assert_eq!(t("https://x.test/jobs/4011001-senior-engineer").as_deref(), Some("senior engineer"));
        assert_eq!(t("https://x.test/careers/senior-rust-engineer").as_deref(), Some("senior rust engineer"));
        assert_eq!(t("https://x.test/jobs/4011001"), None);
        assert_eq!(t("https://x.test/"), None);
    }

    #[test]
    fn page_addresses_are_normalised() {
        assert_eq!(normalize_page_url("acme.test/careers").unwrap().as_str(), "https://acme.test/careers");
        assert_eq!(normalize_page_url(" http://acme.test ").unwrap().as_str(), "http://acme.test/");
        for bad in ["", "ftp://x.test", "file:///etc/passwd", "javascript:alert(1)"] {
            assert!(normalize_page_url(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn a_listing_is_read_plain_first_and_rendered_when_plain_finds_nothing() {
        use fake::*;
        let src = Source { id: 1, kind: "career_page".into(), name: "Acme".into(), url: "https://acme.test/careers".into(), purpose: crate::store::SourcePurpose::Jobs, enabled: true, session_id: None, last_run_at: None, last_status: None };
        let extract = |p: &Page| -> Vec<Candidate> {
            let base = url(&p.url);
            links(&p.html, &base).into_iter().map(|l| Candidate { url: l.url.to_string(), title: l.text, ..Default::default() }).collect()
        };
        let list = "https://acme.test/careers";
        // Plain has postings: the browser is not used.
        let plain = FakePages::new().plain(list, r#"<a href="/careers/a">A</a>"#).rendered(list, "<a href='/careers/b'>B</a>");
        assert_eq!(read_listing(&plain, &src, list, extract).await.unwrap().len(), 1);
        assert_eq!(plain.asked(), [format!("plain {list}")]);
        // Plain finds nothing (a JS page): the browser is.
        let spa = FakePages::new().plain(list, "<div id=root></div>").rendered(list, "<a href='/careers/b'>B</a>");
        assert_eq!(read_listing(&spa, &src, list, extract).await.unwrap()[0].title, "B");
        assert_eq!(spa.asked(), [format!("plain {list}"), format!("browser {list}")]);
        // A wall: the browser.
        let wall = FakePages::new().wall(list, 403).rendered(list, "<a href='/careers/b'>B</a>");
        assert_eq!(read_listing(&wall, &src, list, extract).await.unwrap().len(), 1);
        // A challenge in the browser is the error, for the run to wait on.
        let hard = FakePages::new().wall(list, 403).challenge(list, crate::graph::nodes::ChallengeKind::Captcha);
        let err = read_listing(&hard, &src, list, extract).await.unwrap_err();
        assert_eq!(err.downcast_ref::<ChallengeFound>().map(|c| c.kind), Some(crate::graph::nodes::ChallengeKind::Captcha));
        // Another plain failure is not retried in the browser.
        let gone = FakePages::new();
        assert!(read_listing(&gone, &src, list, extract).await.is_err());
        assert_eq!(gone.asked(), [format!("plain {list}")]);
    }
}
