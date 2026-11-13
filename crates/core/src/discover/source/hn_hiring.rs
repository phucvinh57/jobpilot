//! Hacker News "Ask HN: Who is hiring?": the latest monthly thread through the Algolia API, one
//! top-level comment per candidate. A comment is the whole posting, so no page is fetched.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{
    html_to_text, mentions, mentions_lowered, one_line, page_of, title_fits_roles, Cached, Candidate, Http, JobSource, SearchCtx,
    SearchPage, LISTING_TTL, SNIPPET_CHARS,
};
use crate::prefs::Preferences;

pub(super) const DEFAULT_BASE: &str = "https://hn.algolia.com";
const ITEM_URL: &str = "https://news.ycombinator.com/item?id=";
/// Longest header line kept as the title.
const TITLE_CHARS: usize = 120;

pub struct HnHiring {
    http: Arc<dyn Http>,
    base: String,
    thread: Cached<Vec<Comment>>,
}

/// A top-level comment of the thread, prepared once when the thread is loaded.
struct Comment {
    id: String,
    /// The first line (by HN convention `Company | Role | Location | ...`).
    header: String,
    /// The whole comment as plain text.
    text: String,
    /// `text` in lower case, for matching skills.
    lower: String,
    candidate: Candidate,
}

impl HnHiring {
    pub fn new(http: Arc<dyn Http>, base: &str) -> Self {
        Self { http, base: base.trim_end_matches('/').to_string(), thread: Cached::new(LISTING_TTL) }
    }

    /// The newest "Who is hiring?" story posted by `whoishiring`.
    async fn latest_thread(&self) -> Result<String> {
        let url = format!(
            "{}/api/v1/search_by_date?query=%22who%20is%20hiring%22&tags=story,author_whoishiring&hitsPerPage=10",
            self.base
        );
        let body = self.http.get_json(&url).await.context("looking for the Who is hiring thread")?;
        let hits = body.get("hits").and_then(Value::as_array).context("the Hacker News search has no hits")?;
        let newest = hits
            .iter()
            .filter(|h| h.get("title").and_then(Value::as_str).is_some_and(|t| t.to_lowercase().contains("who is hiring")))
            .max_by_key(|h| h.get("created_at_i").and_then(Value::as_i64).unwrap_or(0));
        match newest.and_then(|h| h.get("objectID")).and_then(Value::as_str) {
            Some(id) if id.chars().all(|c| c.is_ascii_digit()) => Ok(id.to_string()),
            _ => bail!("no Who is hiring thread was found on Hacker News"),
        }
    }

    async fn comments(&self) -> Result<Arc<Vec<Comment>>> {
        self.thread
            .get_or_load(|| async {
                let id = self.latest_thread().await?;
                let url = format!("{}/api/v1/items/{id}", self.base);
                let thread = self.http.get_json(&url).await.context("reading the Who is hiring thread")?;
                let children = thread.get("children").and_then(Value::as_array).context("the thread has no comments")?;
                Ok(children.iter().filter_map(comment_of).collect())
            })
            .await
    }
}

fn comment_of(c: &Value) -> Option<Comment> {
    let id = c.get("id").and_then(Value::as_i64)?.to_string();
    let text = html_to_text(c.get("text").and_then(Value::as_str)?);
    let mut lines = text.lines().skip_while(|l| l.trim().is_empty());
    let header = one_line(lines.next()?, usize::MAX);
    let rest = lines.collect::<Vec<_>>().join(" ");
    let candidate = candidate_of(&id, &header, &rest);
    Some(Comment { id, header, lower: text.to_lowercase(), text, candidate })
}

/// `Company | Role | Location | ...` -> a candidate. Every part of the header that is not the
/// company stays in the title, so nothing the poster wrote is lost.
fn candidate_of(id: &str, header: &str, rest: &str) -> Candidate {
    let parts: Vec<&str> = header.split('|').map(str::trim).filter(|p| !p.is_empty()).collect();
    let company = one_line(parts.first().copied().unwrap_or_default(), 60);
    let location = parts
        .iter()
        .skip(1)
        .find(|p| ["remote", "onsite", "on-site", "hybrid"].iter().any(|k| mentions(p, k)))
        .map(|p| one_line(p, 80))
        .unwrap_or_default();
    Candidate {
        url: format!("{ITEM_URL}{id}"),
        title: one_line(header, TITLE_CHARS),
        company,
        location,
        snippet: one_line(rest, SNIPPET_CHARS),
    }
}

/// Does a comment fit the preferences? Free text, so a skill or role word anywhere counts; no
/// skills and no roles means everything does.
fn relevant(c: &Comment, prefs: &Preferences) -> bool {
    let skills: Vec<&String> = prefs.must_skills.iter().chain(&prefs.nice_skills).collect();
    if skills.is_empty() && prefs.roles.is_empty() {
        return true;
    }
    skills.iter().any(|s| mentions_lowered(&c.lower, s)) || (!prefs.roles.is_empty() && title_fits_roles(&c.header, &prefs.roles))
}

#[async_trait]
impl JobSource for HnHiring {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.comments().await?.iter().filter(|c| relevant(c, prefs)).map(|c| c.candidate.clone()).collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let id = candidate.url.strip_prefix(ITEM_URL).unwrap_or_default();
        match self.comments().await?.iter().find(|c| c.id == id) {
            Some(c) => Ok(Some(c.text.clone())),
            None => bail!("the comment is no longer in the current Who is hiring thread"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::FakeHttp;

    const THREADS: &str = include_str!("../fixtures/hn_threads.json");
    const THREAD: &str = include_str!("../fixtures/hn_thread.json");
    const SEARCH_URL: &str =
        "https://hn.test/api/v1/search_by_date?query=%22who%20is%20hiring%22&tags=story,author_whoishiring&hitsPerPage=10";
    const ITEM: &str = "https://hn.test/api/v1/items/41000003";

    fn hn(http: Arc<FakeHttp>) -> HnHiring {
        HnHiring::new(http, "https://hn.test")
    }

    fn ctx() -> SearchCtx {
        SearchCtx { cursor: None, page_size: 10 }
    }

    #[tokio::test]
    async fn one_top_level_comment_is_one_candidate_from_the_newest_hiring_thread() {
        let http = FakeHttp::with([(SEARCH_URL, THREADS), (ITEM, THREAD)]);
        let page = hn(http.clone()).search(&Preferences::default(), &ctx()).await.unwrap();
        assert_eq!(http.requested(), [SEARCH_URL, ITEM], "the October thread, not 'Who wants to be hired?' or September");
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "Acme Robotics | Senior Rust Engineer | Remote (EU) | Full-time",
                "Sweet Dough | Pastry Chef | Paris, France | Onsite",
                "Globex (YC S20) | Backend Engineer, Go | Singapore | Hybrid"
            ],
            "replies and deleted comments are not candidates"
        );
        let acme = &page.candidates[0];
        assert_eq!(acme.url, "https://news.ycombinator.com/item?id=41000101");
        assert_eq!((acme.company.as_str(), acme.location.as_str()), ("Acme Robotics", "Remote (EU)"));
        assert!(acme.snippet.starts_with("We build warehouse robots"), "{}", acme.snippet);
        assert_eq!(page.candidates[1].location, "Onsite");
        assert_eq!(page.candidates[2].location, "Hybrid");
    }

    #[tokio::test]
    async fn preferences_pick_comments_by_skill_word() {
        let http = FakeHttp::with([(SEARCH_URL, THREADS), (ITEM, THREAD)]);
        let adapter = hn(http);
        let rust = Preferences { must_skills: vec!["Rust".into()], ..Default::default() };
        let page = adapter.search(&rust, &ctx()).await.unwrap();
        assert_eq!(page.candidates.len(), 1);
        assert_eq!(page.candidates[0].company, "Acme Robotics");
        let go = Preferences { nice_skills: vec!["go".into()], roles: vec![], ..Default::default() };
        let page = adapter.search(&go, &ctx()).await.unwrap();
        assert_eq!(page.candidates.len(), 1, "`go` is a word, not a piece of `Google`");
        assert_eq!(page.candidates[0].company, "Globex (YC S20)");
    }

    #[tokio::test]
    async fn the_comment_is_the_posting() {
        let http = FakeHttp::with([(SEARCH_URL, THREADS), (ITEM, THREAD)]);
        let adapter = hn(http.clone());
        let page = adapter.search(&Preferences::default(), &ctx()).await.unwrap();
        let text = adapter.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.contains("Senior Rust Engineer") && text.contains("Rust services and tooling"), "{text}");
        let gone = Candidate { url: format!("{ITEM_URL}1"), ..Default::default() };
        assert!(adapter.posting(&gone).await.unwrap_err().to_string().contains("no longer"));
        assert_eq!(http.requested().len(), 2, "the thread is loaded once");
    }

    #[tokio::test]
    async fn no_hiring_thread_is_an_error() {
        let http = FakeHttp::with([(SEARCH_URL, r#"{"hits": [{"objectID": "1", "title": "Ask HN: Who wants to be hired?", "created_at_i": 1}]}"#)]);
        let err = hn(http).search(&Preferences::default(), &ctx()).await.unwrap_err();
        assert!(format!("{err:#}").contains("no Who is hiring thread"), "{err:#}");
    }
}
