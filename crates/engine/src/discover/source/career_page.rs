//! `career_page`: a job page the user added: a company's careers page, or any other page that lists
//! jobs (a job board's search, an aggregator). The candidates come from the first of these that
//! gives any, in order of cost:
//!
//! 1. the page read plain: its schema.org `JobPosting` JSON-LD ([`super::jsonld`]), else its links
//!    that look like postings;
//! 2. the site's `sitemap.xml` ([`super::sitemap`]), except on a host shared by many companies
//!    (its sitemap lists every company's postings);
//! 3. the page rendered in the user's browser (a job list that JavaScript draws, or a wall on the
//!    plain read): its JSON-LD, else its links.
//!
//! The model never picks a URL: only addresses on the configured site ([`super::web::same_site`]: the host
//! the user entered or a subdomain of it) are candidates, whether a link, a JSON-LD `url` or a
//! sitemap `<loc>`; links and sitemap entries must also have a posting-shaped path
//! ([`super::web::looks_like_posting`]). A posting's text is read by the runner's own fetch.
//!
//! When no attempt finds a posting, the adapter keeps the job boards the pages it read point to
//! ([`super::boards`]): the run's status suggests adding that board as a source. It never reads
//! the board itself.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::info;
use url::Url;

use super::boards::{page_hints, BoardHint};
use super::jsonld::job_postings;
use super::sitemap::read_sitemap;
use super::web::{check_site, is_generic, is_shared_host, links, looks_like_posting, normalize_page_url, page_url, read_plain_page, same_site, title_from_url, Page, Pages};
use super::{page_of, title_fits_roles, Cached, Candidate, JobSource, SearchCtx, SearchPage, LISTING_TTL};
use crate::prefs::Preferences;
use crate::store::Source;

/// Most postings taken from one page: a page with more links than this is not a careers page.
const MAX_POSTINGS: usize = 500;

pub struct CareerPage {
    pages: Arc<dyn Pages>,
    source: Source,
    url: Url,
    company: String,
    listing: Cached<Vec<Candidate>>,
    /// The boards of the last read, when it found no postings.
    hints: Mutex<Vec<BoardHint>>,
}

impl CareerPage {
    /// `company` names the postings whose JSON-LD does not say (an aggregator's page lists many
    /// companies). `Err` for an address that is not an http(s) page of a real site.
    pub fn new(pages: Arc<dyn Pages>, source: &Source, company: &str) -> Result<Self> {
        let url = normalize_page_url(&source.url).context("job page source")?;
        check_site(&url).context("job page source")?;
        Ok(Self { pages, source: source.clone(), url, company: company.to_string(), listing: Cached::new(LISTING_TTL), hints: Mutex::new(Vec::new()) })
    }

    async fn postings(&self) -> Result<Arc<Vec<Candidate>>> {
        self.listing.get_or_load(|| self.read()).await
    }

    /// One read of the page for the add check (`discover::check`), without the role filter or the
    /// cache: every posting found, and the job boards the pages it read point to (kept whether or
    /// not the read failed, so a page that embeds a board can name it even when the browser could
    /// not render it).
    pub(crate) async fn read_for_check(&self) -> (Result<Vec<Candidate>>, Vec<BoardHint>) {
        let mut hints = Vec::new();
        let read = self.read_pages(&mut hints).await;
        (read, hints)
    }

    /// The candidates, trying the plain page, the sitemap and the browser in turn (see the module
    /// docs). A [`ChallengeFound`](crate::browser::challenge::ChallengeFound) or any other
    /// browser error is the caller's error, and so is a plain failure that is not a wall.
    async fn read(&self) -> Result<Vec<Candidate>> {
        let mut hints = Vec::new();
        let read = self.read_pages(&mut hints).await;
        let kept = match &read {
            Ok(found) if found.is_empty() => hints,
            _ => Vec::new(),
        };
        *self.hints.lock().unwrap_or_else(|e| e.into_inner()) = kept;
        read
    }

    /// [`read`](Self::read), adding the board hints of each page it reads to `hints`.
    async fn read_pages(&self, hints: &mut Vec<BoardHint>) -> Result<Vec<Candidate>> {
        let pages = self.pages.as_ref();
        let url = self.url.as_str();
        // Postings the page's JSON-LD says are closed: the fallbacks (links, sitemap, browser) would
        // otherwise bring them back.
        let mut closed = HashSet::new();
        if let Some(page) = read_plain_page(pages, url).await? {
            add_hints(hints, &page);
            let found = self.candidates(&page, &mut closed);
            if !found.is_empty() {
                return Ok(found);
            }
        }
        // A host shared by many companies (linkedin.com, apply.workable.com, github.io) has one
        // sitemap for all of them: only the page itself says which postings are this source's.
        let host = self.url.host_str().unwrap_or_default().trim_start_matches("www.").to_ascii_lowercase();
        if !is_shared_host(&host) {
            let found: Vec<Candidate> =
                read_sitemap(pages, &self.url, &self.url, &self.company, MAX_POSTINGS).await.into_iter().filter(|c| !closed.contains(&c.url)).collect();
            if !found.is_empty() {
                return Ok(found);
            }
        }
        let page = pages.rendered(&self.source, url).await?;
        add_hints(hints, &page);
        Ok(self.candidates(&page, &mut closed))
    }

    /// The postings of `page`: its open JSON-LD postings when it has any on the configured site, else
    /// its same-site, posting-shaped links, titled by their text (or their address). The addresses of
    /// closed JSON-LD postings are added to `closed`, and no link to one of `closed` is a candidate.
    fn candidates(&self, page: &Page, closed: &mut HashSet<String>) -> Vec<Candidate> {
        let Ok(base) = page_url(page) else { return Vec::new() };
        // The site is the one the user configured: a redirect to another site (or links that lead
        // there) does not move it, and a page read from another site has no candidates at all.
        if !same_site(&self.url, &base) {
            return Vec::new();
        }
        let structured = self.structured(page, &base, Utc::now(), closed);
        if !structured.is_empty() {
            return structured;
        }
        links(&page.html, &base)
            .into_iter()
            .filter(|l| looks_like_posting(&self.url, &base, &l.url) && !closed.contains(l.url.as_str()))
            .filter_map(|l| {
                let title = if is_generic(&l.text) { title_from_url(&l.url)? } else { l.text };
                Some(Candidate { url: l.url.to_string(), title, company: self.company.clone(), location: String::new(), snippet: String::new() })
            })
            .take(MAX_POSTINGS)
            .collect()
    }

    /// The page's `JobPosting` JSON-LD items that are still open at `now` and whose address is on
    /// the configured site (JSON-LD is page text: its `url` gets the same rule as a link). The
    /// addresses of the closed ones go to `closed`.
    fn structured(&self, page: &Page, base: &Url, now: DateTime<Utc>, closed: &mut HashSet<String>) -> Vec<Candidate> {
        let postings = job_postings(&page.html, base);
        if postings.is_empty() {
            return Vec::new();
        }
        let total = postings.len();
        let mut seen = HashSet::new();
        let mut found = Vec::new();
        for p in postings {
            let Some(url) = p.url.as_ref().filter(|u| same_site(&self.url, u) && **u != self.url) else { continue };
            if p.expired(now) {
                closed.insert(url.to_string());
                continue;
            }
            if found.len() >= MAX_POSTINGS || !seen.insert(url.to_string()) {
                continue;
            }
            let Some(title) = (if p.title.is_empty() { title_from_url(url) } else { Some(p.title.clone()) }) else { continue };
            // An aggregator's page lists many companies: the posting's own `hiringOrganization` first.
            let company = if p.company.is_empty() { self.company.clone() } else { p.company.clone() };
            found.push(Candidate { url: url.to_string(), title, company, location: p.location.clone(), snippet: String::new() });
        }
        info!(postings = total, candidates = found.len(), closed = closed.len(), "career page: JSON-LD postings read");
        found
    }
}

/// Add the boards `page` points to that `hints` does not have yet.
fn add_hints(hints: &mut Vec<BoardHint>, page: &Page) {
    for hint in page_hints(page) {
        if !hints.contains(&hint) {
            hints.push(hint);
        }
    }
}

#[async_trait]
impl JobSource for CareerPage {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.postings().await?.iter().filter(|c| title_fits_roles(&c.title, &prefs.roles)).cloned().collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    fn board_hints(&self) -> Vec<BoardHint> {
        self.hints.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::challenge::ChallengeFound;
    use crate::discover::source::web::fake::{pages, FakePages};
    use crate::graph::nodes::ChallengeKind;
    use crate::store::SourcePurpose;

    const LIST: &str = "https://acme.test/careers";
    const PAGE: &str = include_str!("../fixtures/career_page.html");
    const JSONLD_PAGE: &str = include_str!("../fixtures/career_page_jsonld.html");
    const SITEMAP: &str = include_str!("../fixtures/sitemap.xml");
    const SITEMAP_URL: &str = "https://acme.test/sitemap.xml";
    const EMBED: &str = include_str!("../fixtures/career_page_embed.html");

    fn source(url: &str) -> Source {
        Source { id: 1, kind: "career_page".into(), name: "Acme".into(), url: url.into(), purpose: SourcePurpose::Jobs, enabled: true, session_id: None, last_run_at: None, last_status: None }
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    #[tokio::test]
    async fn only_same_site_posting_links_become_candidates() {
        let fake = pages(FakePages::new().plain(LIST, PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        let found: Vec<_> = page.candidates.iter().map(|c| (c.url.as_str(), c.title.as_str())).collect();
        assert_eq!(
            found,
            [
                ("https://acme.test/careers/senior-rust-engineer", "Senior Rust Engineer Remote (Vietnam)"),
                ("https://acme.test/careers/data-engineer", "Data Engineer"),
                ("https://careers.acme.test/jobs/4011001", "Staff Platform Engineer"),
                ("https://acme.test/jobs/771234-devops-engineer", "devops engineer"),
            ]
        );
        assert!(page.candidates.iter().all(|c| c.company == "Acme"));
        assert_eq!(fake.asked(), [format!("plain {LIST}")], "a page with postings is not rendered");
        // Never followed: another site, a look-alike name, mail links, the listing itself, plain pages.
        let urls: Vec<_> = page.candidates.iter().map(|c| c.url.as_str()).collect();
        for never in ["evil.test", "notacme", "mailto", "greenhouse", "/about", "/privacy", "benefits"] {
            assert!(!urls.iter().any(|u| u.contains(never)), "{never}: {urls:?}");
        }
    }

    #[tokio::test]
    async fn roles_thin_out_the_page_and_pages_follow_the_cursor() {
        let fake = pages(FakePages::new().plain(LIST, PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let prefs = Preferences { roles: vec!["Rust Engineer".into()], ..Default::default() };
        let first = adapter.search(&prefs, &ctx(None, 2)).await.unwrap();
        assert_eq!(first.candidates.len(), 2);
        assert_eq!(first.next.as_deref(), Some("2"));
        let second = adapter.search(&prefs, &ctx(first.next.as_deref(), 2)).await.unwrap();
        assert_eq!(second.next, None);
        assert_eq!(fake.asked().len(), 1, "one read serves every page");
    }

    #[tokio::test]
    async fn a_javascript_page_is_rendered_in_the_browser() {
        let fake = pages(FakePages::new().plain(LIST, "<html><body><div id=root></div><script src=app.js></script></body></html>").rendered(LIST, PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert_eq!(page.candidates.len(), 4);
        assert_eq!(fake.asked(), [format!("plain {LIST}"), format!("plain {SITEMAP_URL}"), format!("browser {LIST}")], "the sitemap before the browser");
    }

    #[tokio::test]
    async fn json_ld_postings_come_first_and_keep_to_the_site() {
        let fake = pages(FakePages::new().plain(LIST, JSONLD_PAGE).plain(SITEMAP_URL, SITEMAP).rendered(LIST, PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        let found: Vec<_> = page.candidates.iter().map(|c| (c.url.as_str(), c.title.as_str(), c.location.as_str())).collect();
        // Not the expired posting, not the one on another site, not the page's plain link.
        assert_eq!(
            found,
            [
                ("https://acme.test/careers/senior-rust-engineer", "Senior Rust Engineer", "Ho Chi Minh City, VN"),
                ("https://careers.acme.test/jobs/4011001", "Data Engineer", "Berlin, Germany"),
            ]
        );
        assert!(page.candidates.iter().all(|c| c.company == "Acme"));
        assert_eq!(fake.asked(), [format!("plain {LIST}")], "a JSON-LD page is never rendered, and its sitemap never read");
    }

    #[tokio::test]
    async fn a_page_without_postings_reads_the_sitemap_before_the_browser() {
        let empty = "<html><body><div id=root></div></body></html>";
        let fake = pages(FakePages::new().plain(LIST, empty).plain(SITEMAP_URL, SITEMAP).rendered(LIST, PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert_eq!(page.candidates.len(), 5);
        assert_eq!(page.candidates[0].url, "https://acme.test/careers/senior-rust-engineer", "newest first");
        assert_eq!(fake.asked(), [format!("plain {LIST}"), format!("plain {SITEMAP_URL}")], "no browser");

        // A sitemap without postings: the browser, whose page may carry JSON-LD.
        let fake = pages(FakePages::new().plain(LIST, empty).plain(SITEMAP_URL, "<urlset></urlset>").rendered(LIST, JSONLD_PAGE));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert_eq!(page.candidates.len(), 2);
        assert_eq!(fake.asked(), [format!("plain {LIST}"), format!("plain {SITEMAP_URL}"), format!("browser {LIST}")]);
    }

    #[tokio::test]
    async fn a_page_on_a_shared_host_never_reads_the_hosts_sitemap() {
        // apply.workable.com lists every company: its sitemap is not this source's.
        let list = "https://apply.workable.com/acme/";
        let sitemap = "https://apply.workable.com/sitemap.xml";
        let other = r#"<urlset><url><loc>https://apply.workable.com/otherco/j/123456/</loc></url></urlset>"#;
        let empty = "<html><body><div id=root></div></body></html>";
        let fake = pages(FakePages::new().plain(list, empty).plain(sitemap, other).rendered(list, empty));
        let adapter = CareerPage::new(fake.clone(), &source(list), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert!(page.candidates.is_empty(), "no other company's posting");
        assert_eq!(fake.asked(), [format!("plain {list}"), format!("browser {list}")], "no sitemap read");
    }

    #[tokio::test]
    async fn postings_the_json_ld_calls_closed_do_not_come_back_through_the_fallbacks() {
        let closed = r#"<script type="application/ld+json">{"@type":"JobPosting","title":"Platform Engineer","url":"/careers/platform-engineer","validThrough":"2020-01-31"}</script>"#;
        // The page's links.
        let html = format!(r#"{closed}<a href="/careers/platform-engineer">Platform Engineer</a><a href="/careers/rust-engineer">Rust Engineer</a>"#);
        let adapter = CareerPage::new(pages(FakePages::new().plain(LIST, &html)), &source(LIST), "Acme").unwrap();
        let urls: Vec<_> = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://acme.test/careers/rust-engineer"]);
        // The sitemap.
        let sitemap = r#"<urlset><url><loc>https://acme.test/careers/platform-engineer</loc></url><url><loc>https://acme.test/careers/data-engineer</loc></url></urlset>"#;
        let fake = pages(FakePages::new().plain(LIST, closed).plain(SITEMAP_URL, sitemap));
        let adapter = CareerPage::new(fake, &source(LIST), "Acme").unwrap();
        let urls: Vec<_> = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://acme.test/careers/data-engineer"]);
        // The rendered page.
        let rendered = r#"<a href="/careers/platform-engineer">Platform Engineer</a><a href="/careers/ml-engineer">ML Engineer</a>"#;
        let fake = pages(FakePages::new().plain(LIST, closed).rendered(LIST, rendered));
        let adapter = CareerPage::new(fake, &source(LIST), "Acme").unwrap();
        let urls: Vec<_> = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://acme.test/careers/ml-engineer"]);
    }

    #[test]
    fn json_ld_on_a_listing_that_moved_to_another_site_is_ignored() {
        let adapter = CareerPage::new(pages(FakePages::new()), &source(LIST), "Acme").unwrap();
        let moved = Page { url: "https://evil.test/careers".into(), html: JSONLD_PAGE.into() };
        assert!(adapter.candidates(&moved, &mut HashSet::new()).is_empty());
        // Later, the dated posting has closed; the undated one stays.
        let now = DateTime::parse_from_rfc3339("2100-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        let page = Page { url: LIST.into(), html: JSONLD_PAGE.into() };
        let open: Vec<_> = adapter.structured(&page, &Url::parse(LIST).unwrap(), now, &mut HashSet::new()).into_iter().map(|c| c.title).collect();
        assert_eq!(open, ["Data Engineer"]);
        // JSON-LD with no posting on the site: the page's links are read instead.
        let html = r#"<script type="application/ld+json">{"@type":"JobPosting","title":"Elsewhere","url":"https://evil.test/jobs/1"}</script><a href="/careers/rust-engineer">Rust Engineer</a>"#;
        let urls: Vec<_> = adapter.candidates(&Page { url: LIST.into(), html: html.into() }, &mut HashSet::new()).into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://acme.test/careers/rust-engineer"]);
    }

    #[tokio::test]
    async fn a_wall_is_rendered_and_a_challenge_in_the_browser_is_raised_not_swallowed() {
        let walled = pages(FakePages::new().wall(LIST, 403).rendered(LIST, PAGE));
        let adapter = CareerPage::new(walled, &source(LIST), "Acme").unwrap();
        assert_eq!(adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.len(), 4);

        let hard = pages(FakePages::new().wall(LIST, 403).challenge(LIST, ChallengeKind::Captcha));
        let adapter = CareerPage::new(hard.clone(), &source(LIST), "Acme").unwrap();
        let err = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap_err();
        assert_eq!(err.downcast_ref::<ChallengeFound>().map(|c| c.kind), Some(ChallengeKind::Captcha));
        // Not cached: after the user dealt with it, the next search asks again.
        assert!(adapter.search(&Preferences::default(), &ctx(None, 20)).await.is_err());
        assert_eq!(hard.asked().iter().filter(|a| a.starts_with("browser")).count(), 2);
    }

    #[tokio::test]
    async fn links_resolve_against_the_page_that_was_read() {
        // The page redirected to the careers subdomain: relative links belong there, and so does the same-site rule.
        let html = r#"<a href="/jobs/8812345">Backend Engineer</a><a href="https://acme.test/jobs/9912345">Other host, same site</a><a href="https://evil.test/jobs/1234567">Evil</a>"#;
        let adapter = CareerPage::new(pages(FakePages::new().plain(LIST, html)), &source(LIST), "Acme").unwrap();
        let page = Page { url: "https://careers.acme.test/open".into(), html: html.into() };
        let urls: Vec<_> = adapter.candidates(&page, &mut HashSet::new()).into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://careers.acme.test/jobs/8812345", "https://acme.test/jobs/9912345"]);
    }

    #[tokio::test]
    async fn a_listing_that_redirected_to_another_site_has_no_candidates() {
        // The configured page answers from evil.test (a redirect): its links, relative or absolute,
        // are evil.test's, and "same site" stays the configured acme.test.
        let html = r#"<a href="/careers/rust-engineer">Rust Engineer</a><a href="https://evil.test/jobs/1234567">Evil</a><a href="https://acme.test/jobs/8812345">Acme job</a>"#;
        let fake = pages(FakePages::new().plain(LIST, html).rendered(LIST, html).moved(LIST, "https://evil.test/careers"));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert!(page.candidates.is_empty(), "{:?}", page.candidates);
        // Redirected within the site (to a subdomain of it), links on the configured site still count.
        let moved = pages(FakePages::new().plain(LIST, html).moved(LIST, "https://careers.acme.test/open"));
        let adapter = CareerPage::new(moved, &source(LIST), "Acme").unwrap();
        let urls: Vec<_> = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://careers.acme.test/careers/rust-engineer", "https://acme.test/jobs/8812345"]);
    }

    #[tokio::test]
    async fn a_page_without_postings_keeps_the_boards_it_points_to() {
        let fake = pages(FakePages::new().plain(LIST, EMBED).plain(SITEMAP_URL, "<urlset></urlset>").rendered(LIST, EMBED));
        let adapter = CareerPage::new(fake.clone(), &source(LIST), "Acme").unwrap();
        assert!(adapter.board_hints().is_empty(), "nothing read yet");
        let page = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap();
        assert!(page.candidates.is_empty());
        let hints: Vec<_> = adapter.board_hints().into_iter().map(|h| (h.kind, h.input)).collect();
        assert_eq!(hints, [("greenhouse", "boards.greenhouse.io/acme".to_string()), ("lever", "jobs.lever.co/acme".to_string())]);
        // Only suggested: no board was read.
        assert_eq!(fake.asked(), [format!("plain {LIST}"), format!("plain {SITEMAP_URL}"), format!("browser {LIST}")]);

        // A page with postings keeps none, even when it also links to a board.
        let html = format!("{EMBED}<a href=\"/careers/rust-engineer\">Rust Engineer</a>");
        let adapter = CareerPage::new(pages(FakePages::new().plain(LIST, &html)), &source(LIST), "Acme").unwrap();
        assert_eq!(adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.len(), 1);
        assert!(adapter.board_hints().is_empty());
    }

    #[test]
    fn any_page_of_a_real_site_is_accepted_as_a_job_page() {
        let fake = pages(FakePages::new());
        for page in ["https://www.linkedin.com/jobs", "https://github.io/careers", "https://acme.github.io/careers", "https://careers.acme.co.uk/", "https://boards.greenhouse.io/acme"] {
            assert!(CareerPage::new(fake.clone(), &source(page), "Acme").is_ok(), "{page}");
        }
        for none in ["https://co.uk/careers", "https://localhost/careers"] {
            let err = CareerPage::new(fake.clone(), &source(none), "Acme").err().expect(none);
            let text = format!("{err:#}");
            assert!(text.starts_with("job page source") && text.contains("page that lists jobs"), "{none}: {text}");
        }
    }

    #[tokio::test]
    async fn an_aggregator_page_names_each_postings_own_company() {
        let html = r#"<script type="application/ld+json">[
            {"@type":"JobPosting","title":"Rust Engineer","url":"/jobs/view/1","hiringOrganization":{"@type":"Organization","name":"Beta Corp"}},
            {"@type":"JobPosting","title":"Data Engineer","url":"/jobs/view/2"}
        ]</script>"#;
        let list = "https://www.linkedin.com/jobs/search?keywords=rust";
        let fake = pages(FakePages::new().plain(list, html));
        let adapter = CareerPage::new(fake, &source(list), "linkedin.com").unwrap();
        let found: Vec<_> = adapter.search(&Preferences::default(), &ctx(None, 20)).await.unwrap().candidates.into_iter().map(|c| (c.title, c.company)).collect();
        assert_eq!(found, [("Rust Engineer".to_string(), "Beta Corp".to_string()), ("Data Engineer".to_string(), "linkedin.com".to_string())]);
    }

    #[test]
    fn a_bad_address_is_an_error_naming_the_source() {
        let fake = pages(FakePages::new());
        for bad in ["", "ftp://x.test/careers", "javascript:alert(1)"] {
            let err = CareerPage::new(fake.clone(), &source(bad), "Acme").err().expect(bad);
            assert!(format!("{err:#}").starts_with("job page source"), "{err:#}");
        }
    }
}
