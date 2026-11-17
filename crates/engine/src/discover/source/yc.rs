//! `yc`: Y Combinator's job listing (Work at a Startup).
//!
//! YC has no public JSON API for it. Its own board, `https://www.ycombinator.com/jobs`, is plain
//! server-rendered HTML with one link per job (`/companies/<company>/jobs/<id>-<title>`), so the
//! default needs neither a browser nor a login. `workatastartup.com` itself refuses plain requests
//! (HTTP 406) and wants a signed-in visitor: set it as the source's address, bind a browser
//! session in which you are logged in, and the page is read in that browser (jobs there are
//! `/jobs/<number>` or `/companies/<company>/jobs/...`).
//!
//! Only links on the listing's own site that match those shapes are candidates; a posting's text is
//! read by the runner's fetch.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use url::Url;

use super::web::{is_generic, links, normalize_page_url, page_url, read_listing, same_site, title_from_url, Page, Pages};
use super::{page_of, title_fits_roles, Cached, Candidate, JobSource, SearchCtx, SearchPage, LISTING_TTL};
use crate::prefs::Preferences;
use crate::store::Source;

/// The listing read when the source has no address.
pub const DEFAULT_URL: &str = "https://www.ycombinator.com/jobs";

/// The sites this source reads.
const SITES: [&str; 2] = ["https://www.ycombinator.com/", "https://www.workatastartup.com/"];

pub struct Yc {
    pages: Arc<dyn Pages>,
    source: Source,
    url: Url,
    listing: Cached<Vec<Candidate>>,
}

impl Yc {
    /// `Err` for an address that is not on ycombinator.com or workatastartup.com.
    pub fn new(pages: Arc<dyn Pages>, source: &Source) -> Result<Self> {
        let raw = if source.url.trim().is_empty() { DEFAULT_URL } else { source.url.as_str() };
        let url = check_url(raw)?;
        Ok(Self { pages, source: source.clone(), url, listing: Cached::new(LISTING_TTL) })
    }

    async fn jobs(&self) -> Result<Arc<Vec<Candidate>>> {
        self.listing
            .get_or_load(|| async { read_listing(self.pages.as_ref(), &self.source, self.url.as_str(), |page| candidates(page, &self.url)).await })
            .await
    }
}

/// `raw` as the address of a YC listing, or why it is not one.
pub(super) fn check_url(raw: &str) -> Result<Url> {
    let url = normalize_page_url(raw).context("YC source")?;
    if !on_yc(&url) {
        bail!("YC source: `{}` is not a ycombinator.com or workatastartup.com address", url.host_str().unwrap_or_default());
    }
    Ok(url)
}

fn on_yc(url: &Url) -> bool {
    SITES.iter().filter_map(|s| Url::parse(s).ok()).any(|site| same_site(&site, url))
}

/// The job links of a listing page, read for the listing the user configured (`site`): a page
/// from any other site (a redirect) and links that lead elsewhere are not followed.
fn candidates(page: &Page, site: &Url) -> Vec<Candidate> {
    let Ok(base) = page_url(page) else { return Vec::new() };
    if !on_yc(&base) || !same_site(site, &base) {
        return Vec::new();
    }
    links(&page.html, &base)
        .into_iter()
        .filter(|l| same_site(site, &l.url))
        .filter_map(|l| {
            let company = company_of(&l.url)?;
            let title = if is_generic(&l.text) { title_from_url(&l.url)? } else { l.text };
            Some(Candidate { url: l.url.to_string(), title, company, location: String::new(), snippet: String::new() })
        })
        .collect()
}

/// The company of a job link (`""` when the link does not name it), or `None` when the link is not a
/// job: `/companies/<company>/jobs/<job>` on either site, and `/jobs/<number>` on Work at a Startup.
fn company_of(url: &Url) -> Option<String> {
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        ["companies", company, "jobs", job] if !job.is_empty() => Some(company_name(company)),
        ["jobs", id] if id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty() && url.host_str().is_some_and(|h| h.contains("workatastartup")) => {
            Some(String::new())
        }
        _ => None,
    }
}

/// `candid-health` -> `Candid Health`.
fn company_name(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            chars.next().map(|f| f.to_uppercase().chain(chars).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[async_trait]
impl JobSource for Yc {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.jobs().await?.iter().filter(|c| title_fits_roles(&c.title, &prefs.roles)).cloned().collect();
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

    const PAGE: &str = include_str!("../fixtures/yc_jobs.html");

    fn source(url: &str) -> Source {
        Source { id: 1, kind: "yc".into(), name: "YC".into(), url: url.into(), purpose: SourcePurpose::Jobs, enabled: true, session_id: None, last_run_at: None, last_status: None }
    }

    fn ctx() -> SearchCtx {
        SearchCtx { cursor: None, page_size: 50 }
    }

    #[tokio::test]
    async fn the_public_board_lists_every_job_link_with_its_company() {
        let fake = pages(FakePages::new().plain(DEFAULT_URL, PAGE));
        let yc = Yc::new(fake.clone(), &source("")).unwrap();
        let page = yc.search(&Preferences::default(), &ctx()).await.unwrap();
        let found: Vec<_> = page.candidates.iter().map(|c| (c.company.as_str(), c.title.as_str(), c.url.as_str())).collect();
        assert_eq!(
            found,
            [
                ("Aviator", "Software Engineering Intern", "https://www.ycombinator.com/companies/aviator/jobs/aaMaqQu-software-engineering-intern"),
                ("Candid Health", "Senior Software Engineer", "https://www.ycombinator.com/companies/candid-health/jobs/WIMeHdk-senior-software-engineer"),
                ("Glimpse 2", "founding engineer", "https://www.ycombinator.com/companies/glimpse-2/jobs/5hmOmrr-founding-engineer"),
            ],
            "company pages, the apply sub-page, other sites and the nav are not jobs"
        );
        assert_eq!(fake.asked(), [format!("plain {DEFAULT_URL}")], "the public board needs no browser");
    }

    #[tokio::test]
    async fn work_at_a_startup_is_read_in_the_browser_when_it_refuses_plain_requests() {
        let waas = "https://www.workatastartup.com/jobs";
        let html = r#"<a href="/jobs/55012">Backend Engineer</a><a href="/companies/acme-robotics/jobs/ZxCvB-staff-engineer">Staff Engineer</a><a href="/companies/acme-robotics">Acme</a><a href="https://evil.test/jobs/1">x</a>"#;
        let fake = pages(FakePages::new().wall(waas, 406).rendered(waas, html));
        let yc = Yc::new(fake.clone(), &source(waas)).unwrap();
        let page = yc.search(&Preferences::default(), &ctx()).await.unwrap();
        let found: Vec<_> = page.candidates.iter().map(|c| (c.company.as_str(), c.title.as_str())).collect();
        assert_eq!(found, [("", "Backend Engineer"), ("Acme Robotics", "Staff Engineer")]);
        assert_eq!(fake.asked(), [format!("plain {waas}"), format!("browser {waas}")]);
    }

    #[tokio::test]
    async fn a_login_wall_in_the_browser_is_raised_for_the_run_to_wait_on() {
        let waas = "https://www.workatastartup.com/jobs";
        let fake = pages(FakePages::new().wall(waas, 406).challenge(waas, ChallengeKind::Login));
        let err = Yc::new(fake, &source(waas)).unwrap().search(&Preferences::default(), &ctx()).await.unwrap_err();
        assert_eq!(err.downcast_ref::<ChallengeFound>().map(|c| c.kind), Some(ChallengeKind::Login));
    }

    #[test]
    fn only_yc_addresses_are_accepted() {
        let fake = pages(FakePages::new());
        assert!(Yc::new(fake.clone(), &source("https://www.ycombinator.com/jobs/role/software-engineer")).is_ok());
        assert!(Yc::new(fake.clone(), &source("workatastartup.com/jobs")).is_ok());
        for bad in ["https://evil.test/jobs", "https://notycombinator.com/jobs", "ftp://ycombinator.com", "https://ycombinator.com.evil.test/"] {
            assert!(Yc::new(fake.clone(), &source(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_page_on_another_site_has_no_candidates() {
        let site = Url::parse(DEFAULT_URL).unwrap();
        let page = Page { url: "https://evil.test/jobs".into(), html: r#"<a href="/companies/x/jobs/1-a-b">Job</a>"#.into() };
        assert!(candidates(&page, &site).is_empty());
        // A page of the other YC site than the configured one is not the configured listing either.
        let other = Page { url: "https://www.workatastartup.com/jobs".into(), html: r#"<a href="/companies/x/jobs/1-a-b">Job</a>"#.into() };
        assert!(candidates(&other, &site).is_empty());
        let same = Page { url: DEFAULT_URL.into(), html: r#"<a href="/companies/x/jobs/1-a-b">Job</a><a href="https://www.workatastartup.com/companies/y/jobs/2-c-d">Other site</a>"#.into() };
        let found: Vec<_> = candidates(&same, &site).into_iter().map(|c| c.url).collect();
        assert_eq!(found, ["https://www.ycombinator.com/companies/x/jobs/1-a-b"], "links to the other YC site are not followed");
    }

    #[tokio::test]
    async fn a_listing_that_redirected_to_another_site_has_no_candidates() {
        let html = r#"<a href="/companies/x/jobs/1-a-b">Job</a>"#;
        let fake = pages(FakePages::new().plain(DEFAULT_URL, html).rendered(DEFAULT_URL, html).moved(DEFAULT_URL, "https://evil.test/jobs"));
        let page = Yc::new(fake, &source("")).unwrap().search(&Preferences::default(), &ctx()).await.unwrap();
        assert!(page.candidates.is_empty(), "{:?}", page.candidates);
    }
}
