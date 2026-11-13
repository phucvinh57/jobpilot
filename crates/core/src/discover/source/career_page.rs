//! `career_page`: a careers page the user added. The links on it that look like postings and stay on
//! the same site are the candidates; the page is read plain first and rendered in the managed browser
//! when it has no postings in its HTML (a job list that JavaScript draws) or answers with a wall.
//!
//! The model never picks a URL: only same-site links with a posting-shaped path are followed
//! ([`web::looks_like_posting`]), and a posting's text is read by the runner's own fetch.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use url::Url;

use super::web::{links, looks_like_posting, normalize_page_url, page_url, read_listing, title_from_url, Page, Pages, is_generic};
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
}

impl CareerPage {
    /// `company` names the postings (the page does not say). `Err` for an address that is not an
    /// http(s) page.
    pub fn new(pages: Arc<dyn Pages>, source: &Source, company: &str) -> Result<Self> {
        let url = normalize_page_url(&source.url).context("career page source")?;
        Ok(Self { pages, source: source.clone(), url, company: company.to_string(), listing: Cached::new(LISTING_TTL) })
    }

    async fn postings(&self) -> Result<Arc<Vec<Candidate>>> {
        self.listing
            .get_or_load(|| async { read_listing(self.pages.as_ref(), &self.source, self.url.as_str(), |page| self.candidates(page)).await })
            .await
    }

    /// The same-site, posting-shaped links of `page`, titled by their text (or their address).
    fn candidates(&self, page: &Page) -> Vec<Candidate> {
        let Ok(base) = page_url(page) else { return Vec::new() };
        links(&page.html, &base)
            .into_iter()
            .filter(|l| looks_like_posting(&base, &l.url))
            .filter_map(|l| {
                let title = if is_generic(&l.text) { title_from_url(&l.url)? } else { l.text };
                Some(Candidate { url: l.url.to_string(), title, company: self.company.clone(), location: String::new(), snippet: String::new() })
            })
            .take(MAX_POSTINGS)
            .collect()
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
        assert_eq!(fake.asked(), [format!("plain {LIST}"), format!("browser {LIST}")]);
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
        let urls: Vec<_> = adapter.candidates(&page).into_iter().map(|c| c.url).collect();
        assert_eq!(urls, ["https://careers.acme.test/jobs/8812345", "https://acme.test/jobs/9912345"]);
    }

    #[test]
    fn a_bad_address_is_an_error_naming_the_source() {
        let fake = pages(FakePages::new());
        for bad in ["", "ftp://x.test/careers", "javascript:alert(1)"] {
            let err = CareerPage::new(fake.clone(), &source(bad), "Acme").err().expect(bad);
            assert!(format!("{err:#}").starts_with("career page source"), "{err:#}");
        }
    }
}
