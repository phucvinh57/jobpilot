//! Ashby: the public job-board API (`api.ashbyhq.com/posting-api/job-board/<slug>`).
//!
//! The listing carries every description (and the pay range when the company publishes it), so
//! postings are read from it (one request, cached).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{
    last_segment, one_line, page_of, title_fits_roles, Cached, Candidate, Http, JobSource, SearchCtx, SearchPage,
    LISTING_TTL, SNIPPET_CHARS,
};
use crate::engine::prefs::Preferences;

pub(super) const DEFAULT_BASE: &str = "https://api.ashbyhq.com";

pub struct Ashby {
    http: Arc<dyn Http>,
    base: String,
    slug: String,
    company: String,
    listing: Cached<Vec<Posting>>,
}

struct Posting {
    candidate: Candidate,
    text: String,
}

impl Ashby {
    pub fn new(http: Arc<dyn Http>, base: &str, slug: &str, company: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            slug: slug.to_string(),
            company: company.to_string(),
            listing: Cached::new(LISTING_TTL),
        }
    }

    async fn postings(&self) -> Result<Arc<Vec<Posting>>> {
        self.listing
            .get_or_load(|| async {
                let url = format!("{}/posting-api/job-board/{}?includeCompensation=true", self.base, self.slug);
                let body = self.http.get_json(&url).await.context("reading the Ashby job board")?;
                let jobs = body.get("jobs").and_then(Value::as_array).context("the Ashby board has no job list")?;
                Ok(jobs.iter().filter_map(|j| self.posting_of(j)).collect())
            })
            .await
    }

    fn posting_of(&self, job: &Value) -> Option<Posting> {
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        if job.get("isListed").and_then(Value::as_bool) == Some(false) {
            return None;
        }
        let url = text(job.get("jobUrl"));
        let title = text(job.get("title"));
        if url.is_empty() || title.is_empty() {
            return None;
        }
        let mut location = text(job.get("location"));
        if job.get("isRemote").and_then(Value::as_bool) == Some(true) && !location.to_lowercase().contains("remote") {
            location = if location.is_empty() { "Remote".into() } else { format!("{location} (Remote)") };
        }
        let description = text(job.get("descriptionPlain"));
        let pay = text(job.pointer("/compensation/compensationTierSummary"));
        let mut full = format!("# {title}\n\nCompany: {}\nLocation: {location}\n", self.company);
        if !pay.is_empty() {
            full.push_str(&format!("Compensation: {pay}\n"));
        }
        full.push_str(&format!("\n{description}"));
        Some(Posting {
            candidate: Candidate {
                url,
                title,
                company: self.company.clone(),
                location,
                snippet: one_line(&description, SNIPPET_CHARS),
            },
            text: full,
        })
    }
}

#[async_trait]
impl JobSource for Ashby {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> = self
            .postings()
            .await?
            .iter()
            .filter(|p| title_fits_roles(&p.candidate.title, &prefs.roles))
            .map(|p| p.candidate.clone())
            .collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let wanted = last_segment(&candidate.url);
        let postings = self.postings().await?;
        Ok(postings
            .iter()
            .find(|p| wanted.is_some() && last_segment(&p.candidate.url) == wanted)
            .map(|p| p.text.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::discover::source::FakeHttp;

    const BOARD: &str = include_str!("../fixtures/ashby_board.json");
    const URL: &str = "https://ashby.test/posting-api/job-board/initech?includeCompensation=true";

    #[tokio::test]
    async fn lists_listed_jobs_with_remote_and_pay() {
        let http = FakeHttp::with([(URL, BOARD)]);
        let ashby = Ashby::new(http.clone(), "https://ashby.test", "initech", "Initech");
        let ctx = SearchCtx { cursor: None, page_size: 10 };
        let page = ashby.search(&Preferences::default(), &ctx).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Staff Software Engineer, Infrastructure", "Product Designer"], "unlisted jobs are skipped");
        assert_eq!(page.candidates[0].location, "Remote - EMEA");
        assert_eq!(page.candidates[1].location, "Austin, TX");
        assert!(page.candidates[0].snippet.starts_with("Initech makes workflow software"));

        let text = ashby.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.contains("Compensation: $180K - $220K") && text.contains("storage layer written in Rust"), "{text}");
        let other = Candidate { url: "https://jobs.ashbyhq.com/initech/nope".into(), ..Default::default() };
        assert_eq!(ashby.posting(&other).await.unwrap(), None);
        assert_eq!(http.requested().len(), 1);
    }

    #[tokio::test]
    async fn the_posting_is_found_when_the_runner_stripped_tracking_parameters() {
        let http = FakeHttp::with([(URL, BOARD)]);
        let ashby = Ashby::new(http, "https://ashby.test", "initech", "Initech");
        let c = Candidate { url: "https://jobs.ashbyhq.com/initech/7b1d0000-0000-4000-8000-0000000000a2".into(), ..Default::default() };
        assert!(ashby.posting(&c).await.unwrap().unwrap().contains("Design things."));
    }

    #[tokio::test]
    async fn a_board_that_is_down_is_a_network_class_error() {
        let http = FakeHttp::with([]);
        let err = Ashby::new(http, "https://ashby.test", "initech", "Initech")
            .search(&Preferences::default(), &SearchCtx { cursor: None, page_size: 5 })
            .await
            .unwrap_err();
        assert!(format!("{err:#}").contains("reading the Ashby job board"), "{err:#}");
    }
}
