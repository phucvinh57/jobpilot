//! Lever: the public postings API (`api.lever.co/v0/postings/<slug>?mode=json`).
//!
//! The listing carries every description, so postings are read from it (one request, cached).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{
    html_to_text, last_segment, one_line, page_of, title_fits_roles, Cached, Candidate, Http, JobSource, SearchCtx, SearchPage,
    LISTING_TTL, SNIPPET_CHARS,
};
use crate::engine::prefs::Preferences;

pub(super) const DEFAULT_BASE: &str = "https://api.lever.co";

pub struct Lever {
    http: Arc<dyn Http>,
    base: String,
    slug: String,
    company: String,
    listing: Cached<Vec<Posting>>,
}

/// A posting of the listing with its text.
struct Posting {
    id: String,
    candidate: Candidate,
    text: String,
}

impl Lever {
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
                let url = format!("{}/v0/postings/{}?mode=json", self.base, self.slug);
                let body = self.http.get_json(&url).await.context("reading the Lever postings")?;
                let list = body.as_array().context("the Lever answer is not a list of postings")?;
                Ok(list.iter().filter_map(|p| self.posting_of(p)).collect())
            })
            .await
    }

    fn posting_of(&self, p: &Value) -> Option<Posting> {
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        let url = text(p.get("hostedUrl"));
        let title = text(p.get("text"));
        if url.is_empty() || title.is_empty() {
            return None;
        }
        let id = text(p.get("id"));
        let mut location = text(p.pointer("/categories/location"));
        if location.is_empty() {
            let all: Vec<&str> = p
                .pointer("/categories/allLocations")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            location = all.join(", ");
        }
        if p.get("workplaceType").and_then(Value::as_str) == Some("remote") && !location.to_lowercase().contains("remote") {
            location = if location.is_empty() { "Remote".into() } else { format!("{location} (Remote)") };
        }
        let mut body = text(p.get("descriptionPlain"));
        for list in p.get("lists").and_then(Value::as_array).into_iter().flatten() {
            body.push_str(&format!("\n\n{}\n{}", text(list.get("text")), html_to_text(&text(list.get("content")))));
        }
        let extra = text(p.get("additionalPlain"));
        if !extra.is_empty() {
            body.push_str(&format!("\n\n{extra}"));
        }
        let snippet = one_line(&text(p.get("descriptionPlain")), SNIPPET_CHARS);
        let full = format!("# {title}\n\nCompany: {}\nLocation: {location}\n\n{}", self.company, body.trim());
        Some(Posting { id, candidate: Candidate { url, title, company: self.company.clone(), location, snippet }, text: full })
    }
}

#[async_trait]
impl JobSource for Lever {
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
        Ok(postings.iter().find(|p| Some(&p.id) == wanted.as_ref() || p.candidate.url == candidate.url).map(|p| p.text.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::discover::source::FakeHttp;

    const LIST: &str = include_str!("../fixtures/lever_postings.json");
    const URL: &str = "https://lever.test/v0/postings/globex?mode=json";

    fn ctx(page_size: usize) -> SearchCtx {
        SearchCtx { cursor: None, page_size }
    }

    #[tokio::test]
    async fn lists_postings_with_locations_and_snippets() {
        let http = FakeHttp::with([(URL, LIST)]);
        let lever = Lever::new(http.clone(), "https://lever.test", "globex", "Globex Corp");
        let page = lever.search(&Preferences::default(), &ctx(10)).await.unwrap();
        assert_eq!(page.candidates.len(), 3);
        let [go, sales, sre] = &page.candidates[..] else { panic!("three postings") };
        assert_eq!((go.title.as_str(), go.company.as_str(), go.location.as_str()), ("Backend Engineer (Go)", "Globex Corp", "Ho Chi Minh City"));
        assert!(go.snippet.starts_with("Globex is a payments company"), "{}", go.snippet);
        assert_eq!(go.url, "https://jobs.lever.co/globex/5f2a1c9e-0000-4000-8000-000000000001");
        assert_eq!(sales.location, "London");
        assert_eq!(sre.location, "Remote, Singapore", "all locations when there is no main one");
        assert_eq!(page.next, None);
    }

    #[tokio::test]
    async fn postings_are_read_from_the_cached_listing() {
        let http = FakeHttp::with([(URL, LIST)]);
        let lever = Lever::new(http.clone(), "https://lever.test/", "globex", "Globex");
        let prefs = Preferences { roles: vec!["Software Engineer".into()], ..Default::default() };
        let page = lever.search(&prefs, &ctx(10)).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Backend Engineer (Go)", "Site Reliability Engineer"], "no sales in an engineering search");

        let text = lever.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.starts_with("# Backend Engineer (Go)"), "{text}");
        assert!(text.contains("move money between banks") && text.contains("3+ years of Go"), "{text}");
        assert!(text.contains("remote-friendly hours"), "{text}");
        let unknown = Candidate { url: "https://jobs.lever.co/globex/zzz".into(), ..Default::default() };
        assert_eq!(lever.posting(&unknown).await.unwrap(), None);
        assert_eq!(http.requested().len(), 1, "listing, search and postings: one request");
    }

    #[tokio::test]
    async fn the_remote_flag_shows_in_the_location() {
        let body = r#"[{"id":"1","text":"Dev","hostedUrl":"https://jobs.lever.co/x/1","categories":{"location":"Hanoi"},"workplaceType":"remote","descriptionPlain":"d"}]"#;
        let http = FakeHttp::with([("https://l.test/v0/postings/x?mode=json", body)]);
        let page = Lever::new(http, "https://l.test", "x", "X").search(&Preferences::default(), &ctx(5)).await.unwrap();
        assert_eq!(page.candidates[0].location, "Hanoi (Remote)");
    }

    #[tokio::test]
    async fn an_unexpected_answer_is_an_error() {
        let http = FakeHttp::with([(URL, r#"{"ok": false}"#)]);
        let err = Lever::new(http, "https://lever.test", "globex", "G").search(&Preferences::default(), &ctx(5)).await.unwrap_err();
        assert!(format!("{err:#}").contains("not a list"), "{err:#}");
    }
}
