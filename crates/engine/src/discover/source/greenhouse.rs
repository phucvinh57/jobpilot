//! Greenhouse: the public board API (`boards-api.greenhouse.io/v1/boards/<slug>/jobs`).
//!
//! The listing is small (no descriptions); a posting's text is one request for that job.

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{
    html_to_text, page_of, title_fits_roles, unescape_markup, Cached, Candidate, Http, JobSource, SearchCtx,
    SearchPage, LISTING_TTL,
};
use crate::prefs::Preferences;

pub(super) const DEFAULT_BASE: &str = "https://boards-api.greenhouse.io";

pub struct Greenhouse {
    http: Arc<dyn Http>,
    base: String,
    slug: String,
    company: String,
    listing: Cached<Vec<Candidate>>,
}

impl Greenhouse {
    /// `base`: the API origin (`https://boards-api.greenhouse.io`, or a test server). `company` is
    /// the name used for postings that do not carry one.
    pub fn new(http: Arc<dyn Http>, base: &str, slug: &str, company: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            slug: slug.to_string(),
            company: company.to_string(),
            listing: Cached::new(LISTING_TTL),
        }
    }

    async fn jobs(&self) -> Result<Arc<Vec<Candidate>>> {
        self.listing
            .get_or_load(|| async {
                let url = format!("{}/v1/boards/{}/jobs", self.base, self.slug);
                let body = self.http.get_json(&url).await.context("reading the Greenhouse board")?;
                let jobs = body.get("jobs").and_then(Value::as_array).context("the Greenhouse board has no job list")?;
                Ok(jobs.iter().filter_map(|j| self.candidate(j)).collect())
            })
            .await
    }

    fn candidate(&self, job: &Value) -> Option<Candidate> {
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        let url = text(job.get("absolute_url"));
        let title = text(job.get("title"));
        if url.is_empty() || title.is_empty() {
            return None;
        }
        let company = Some(text(job.get("company_name"))).filter(|c| !c.is_empty()).unwrap_or_else(|| self.company.clone());
        Some(Candidate { url, title, company, location: text(job.pointer("/location/name")), snippet: String::new() })
    }
}

#[async_trait]
impl JobSource for Greenhouse {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.jobs().await?.iter().filter(|c| title_fits_roles(&c.title, &prefs.roles)).cloned().collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let Some(id) = job_id(&candidate.url) else { return Ok(None) };
        let url = format!("{}/v1/boards/{}/jobs/{id}", self.base, self.slug);
        let job = self.http.get_json(&url).await.context("reading the Greenhouse posting")?;
        let content = job.get("content").and_then(Value::as_str).unwrap_or_default();
        let body = html_to_text(&unescape_markup(content));
        if body.is_empty() {
            return Ok(None);
        }
        let title = job.get("title").and_then(Value::as_str).unwrap_or(&candidate.title);
        let location = job.pointer("/location/name").and_then(Value::as_str).unwrap_or(&candidate.location);
        Ok(Some(format!("# {title}\n\nCompany: {}\nLocation: {location}\n\n{body}", candidate.company)))
    }
}

/// The numeric job id: `/jobs/4011001` on a board, or `?gh_jid=4011001` on a company's own page.
fn job_id(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if let Some((_, v)) = url.query_pairs().find(|(k, _)| k == "gh_jid") {
        if numeric(&v) {
            return Some(v.into_owned());
        }
    }
    url.path_segments()?.rfind(|s| !s.is_empty()).filter(|s| numeric(s)).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::FakeHttp;

    const LIST: &str = include_str!("../fixtures/greenhouse_jobs.json");
    const JOB: &str = include_str!("../fixtures/greenhouse_job.json");
    const LIST_URL: &str = "https://gh.test/v1/boards/acme/jobs";

    fn adapter(http: Arc<FakeHttp>) -> Greenhouse {
        Greenhouse::new(http, "https://gh.test/", "acme", "Acme (source name)")
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    #[tokio::test]
    async fn lists_the_board_and_drops_postings_without_a_usable_link() {
        let http = FakeHttp::with([(LIST_URL, LIST)]);
        let page = adapter(http.clone()).search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        // The `javascript:` link is kept here (the runner drops non-http URLs); the one without a link would be gone.
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            ["Senior Backend Engineer", "Account Executive, Enterprise", "Software Engineer, Platform", "Engineering Manager, Fleet", "Staff Data Engineer"]
        );
        let first = &page.candidates[0];
        assert_eq!(first.company, "Acme Robotics");
        assert_eq!(first.location, "Remote - Vietnam");
        assert_eq!(first.url, "https://boards.greenhouse.io/acme/jobs/4011001?gh_src=abc123");
        assert_eq!(page.candidates[2].company, "Acme (source name)", "no company on the posting: the source's name");
        assert_eq!(page.candidates[4].location, "", "a null location is empty");
        assert_eq!(page.next, None);
        assert_eq!(http.requested(), [LIST_URL]);
    }

    #[tokio::test]
    async fn preferences_thin_out_the_board_and_pages_follow_the_cursor() {
        let http = FakeHttp::with([(LIST_URL, LIST)]);
        let gh = adapter(http.clone());
        let prefs = Preferences { roles: vec!["Backend Engineer".into()], ..Default::default() };
        let first = gh.search(&prefs, &ctx(None, 2)).await.unwrap();
        assert_eq!(first.candidates.len(), 2);
        assert_eq!(first.next.as_deref(), Some("2"));
        let second = gh.search(&prefs, &ctx(first.next.as_deref(), 2)).await.unwrap();
        assert_eq!(second.candidates.len(), 2);
        assert_eq!(second.next, None);
        let all: Vec<_> = first.candidates.iter().chain(&second.candidates).map(|c| c.title.as_str()).collect();
        assert!(!all.contains(&"Account Executive, Enterprise"), "{all:?}");
        assert!(all.contains(&"Engineering Manager, Fleet"), "{all:?}");
        assert_eq!(http.requested().len(), 1, "one listing request serves every page");
    }

    #[tokio::test]
    async fn a_posting_is_read_from_its_job_endpoint() {
        let http = FakeHttp::with([(LIST_URL, LIST), ("https://gh.test/v1/boards/acme/jobs/4011001", JOB)]);
        let gh = adapter(http.clone());
        let page = gh.search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        let text = gh.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.starts_with("# Senior Backend Engineer"), "{text}");
        assert!(text.contains("Rust & Go") && text.contains("Kubernetes on AWS"), "{text}");
        assert!(text.contains("Location: Remote - Vietnam") && text.contains("Company: Acme Robotics"), "{text}");
        assert!(!text.contains("<p>") && !text.contains("&lt;"), "markup is turned into text: {text}");
        let err = gh.posting(&page.candidates[2]).await.unwrap_err();
        assert!(format!("{err:#}").contains("404"), "{err:#}");
    }

    #[test]
    fn job_ids_come_from_the_path_or_the_gh_jid_parameter() {
        assert_eq!(job_id("https://boards.greenhouse.io/acme/jobs/4011001?gh_src=x").as_deref(), Some("4011001"));
        assert_eq!(job_id("https://careers.acme.test/open?gh_jid=77").as_deref(), Some("77"));
        assert_eq!(job_id("https://careers.acme.test/open"), None);
    }

    #[tokio::test]
    async fn a_broken_board_is_an_error_naming_the_step() {
        let http = FakeHttp::with([(LIST_URL, r#"{"nope": 1}"#)]);
        let err = adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err();
        assert!(format!("{err:#}").contains("no job list"), "{err:#}");
    }
}
