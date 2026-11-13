//! Job sources: code that turns the preferences into candidate postings. The model never picks
//! URLs; every candidate comes from an adapter reading a public API.
//!
//! | `sources.kind` | `sources.url` | How |
//! |----------------|---------------|-----|
//! | `greenhouse` | board URL or slug (`boards.greenhouse.io/acme`) | public board JSON |
//! | `lever` | `jobs.lever.co/acme` or slug | public postings JSON |
//! | `ashby` | `jobs.ashbyhq.com/acme` or slug | public job-board JSON |
//! | `hn_hiring` | unused | HN Algolia, the latest "Who is hiring?" thread, one comment per candidate |
//!
//! Adapters read through [`Http`] and a base URL, so tests answer from saved fixture JSON.

mod ashby;
mod greenhouse;
mod hn_hiring;
mod http;
mod lever;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use ashby::Ashby;
pub use greenhouse::Greenhouse;
pub use hn_hiring::HnHiring;
pub use http::{Http, ReqwestHttp};
pub use lever::Lever;

#[cfg(test)]
pub(crate) use http::fake::FakeHttp;

use crate::pipeline::ingest::clean;
use crate::prefs::Preferences;
use crate::store::Source;

/// A posting an adapter found, before it is fetched, filtered and scored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Candidate {
    /// Where the posting lives (http or https; anything else is dropped).
    pub url: String,
    /// The job title as the source writes it.
    pub title: String,
    pub company: String,
    /// `""` when the source does not say.
    pub location: String,
    /// A short extract (at most [`SNIPPET_CHARS`] characters), for lists.
    pub snippet: String,
}

/// Longest [`Candidate::snippet`]. Candidates travel in the checkpointed state of a run.
pub const SNIPPET_CHARS: usize = 280;

/// What to search for and where to continue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCtx {
    /// `None` for the first page; else the `next` of the previous [`SearchPage`].
    pub cursor: Option<String>,
    /// At most this many candidates per page.
    pub page_size: usize,
}

/// One page of candidates.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchPage {
    pub candidates: Vec<Candidate>,
    /// Where the next page starts; `None` when this was the last one.
    pub next: Option<String>,
}

/// A place to find postings.
#[async_trait]
pub trait JobSource: Send + Sync {
    /// One page of candidates matching `prefs`, starting at `ctx.cursor`. The same cursor gives
    /// the same page again (a paused run searches it a second time after "Continue").
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage>;

    /// The posting's full text when the adapter has it without loading the posting's web page
    /// (all of the API adapters do); `None` makes the runner fetch the page.
    async fn posting(&self, _candidate: &Candidate) -> Result<Option<String>> {
        Ok(None)
    }
}

/// The adapter for a `sources` row. `Err` for a kind without an adapter and for a `url` the
/// adapter cannot read, with a message the user can act on.
pub fn adapter_for(source: &Source, http: Arc<dyn Http>) -> Result<Arc<dyn JobSource>> {
    let company = |slug: &str| if source.name.trim().is_empty() { slug.to_string() } else { source.name.trim().to_string() };
    Ok(match source.kind.as_str() {
        "greenhouse" => {
            let slug = board_slug(&source.url, &["greenhouse.io"]).context("Greenhouse source")?;
            Arc::new(Greenhouse::new(http, greenhouse::DEFAULT_BASE, &slug, &company(&slug)))
        }
        "lever" => {
            let slug = board_slug(&source.url, &["lever.co"]).context("Lever source")?;
            Arc::new(Lever::new(http, lever::DEFAULT_BASE, &slug, &company(&slug)))
        }
        "ashby" => {
            let slug = board_slug(&source.url, &["ashbyhq.com"]).context("Ashby source")?;
            Arc::new(Ashby::new(http, ashby::DEFAULT_BASE, &slug, &company(&slug)))
        }
        "hn_hiring" => Arc::new(HnHiring::new(http, hn_hiring::DEFAULT_BASE)),
        other => bail!("no adapter for source kind `{other}` yet"),
    })
}

/// The board name in a board URL (`https://boards.greenhouse.io/acme/jobs/1` -> `acme`) or a bare
/// slug. `hosts` are the registrable domains a board URL may have.
pub(crate) fn board_slug(input: &str, hosts: &[&str]) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no company board is set: enter the board URL or its short name");
    }
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !input.contains('/') && !input.contains(':') {
        return if valid(input) { Ok(input.to_string()) } else { bail!("`{input}` is not a board name") };
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = url::Url::parse(&with_scheme).with_context(|| format!("`{input}` is not a board URL"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if !hosts.iter().any(|h| host == *h || host.ends_with(&format!(".{h}"))) {
        bail!("`{host}` is not one of {}", hosts.join(", "));
    }
    // `boards.greenhouse.io/embed/job_board?for=acme`
    let from_query = url.query_pairs().find(|(k, _)| k == "for").map(|(_, v)| v.into_owned());
    let from_path = url.path_segments().and_then(|mut s| s.find(|p| !p.is_empty() && *p != "embed")).map(str::to_string);
    match from_query.or(from_path) {
        Some(slug) if valid(&slug) => Ok(slug),
        _ => bail!("the board name is missing in `{input}`"),
    }
}

// ---------------------------------------------------------------- shared helpers

/// Lower-case words of `text` (letters and digits).
pub(crate) fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Words that say how senior or which kind of job it is, not what it is: they do not decide
/// whether a title fits a wanted role.
const ROLE_NOISE: &[&str] = &["senior", "junior", "staff", "principal", "lead", "head", "the", "and", "for", "intern"];

/// Does a job title fit one of the wanted `roles`? A title fits when it shares a meaningful word
/// with a role (`engineer` matches `engineering`), so "Software Engineer, Payments" fits
/// "Backend Engineer" and "Account Executive" does not. No roles means everything fits. This only
/// thins out big boards before the expensive steps; precision is the scoring's job.
pub(crate) fn title_fits_roles(title: &str, roles: &[String]) -> bool {
    let wanted: HashSet<String> = roles
        .iter()
        .flat_map(|r| words(r))
        .filter(|w| w.chars().count() >= 3 && !ROLE_NOISE.contains(&w.as_str()))
        .collect();
    if wanted.is_empty() {
        return true;
    }
    words(title).iter().any(|w| {
        wanted.iter().any(|t| w == t || (t.chars().count() >= 5 && w.chars().count() >= 5 && (w.starts_with(t) || t.starts_with(w))))
    })
}

/// Does `text` mention `term` as a word (so `go` is not in `google`, while `c++` and `.net`
/// work)? Case-insensitive.
pub(crate) fn mentions(text: &str, term: &str) -> bool {
    let (text, term) = (text.to_lowercase(), term.trim().to_lowercase());
    if term.is_empty() {
        return false;
    }
    text.match_indices(&term).any(|(i, m)| {
        let before = text[..i].chars().next_back().is_none_or(|c| !c.is_alphanumeric());
        let after = text[i + m.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        before && after
    })
}

/// The last path segment of a URL (a posting's id on Lever and Ashby).
pub(crate) fn last_segment(url: &str) -> Option<String> {
    let url = url::Url::parse(url).ok()?;
    url.path_segments()?.rfind(|s| !s.is_empty()).map(str::to_string)
}

/// Readable text of an HTML fragment (job descriptions arrive as HTML).
pub(crate) fn html_to_text(html: &str) -> String {
    clean(&html2text::from_read(html.as_bytes(), 120).unwrap_or_default())
}

/// `html` as the Greenhouse API sends it: markup with its angle brackets escaped (`&lt;p&gt;`).
pub(crate) fn unescape_markup(escaped: &str) -> String {
    escaped.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
}

/// `text` on one line, at most `max` characters.
pub(crate) fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match line.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", line[..i].trim_end()),
        None => line,
    }
}

/// The page `ctx` asks for out of `items` (which must come in the same order every time), and the
/// cursor of the following page. The cursor is the offset of its first item.
pub(crate) fn page_of<T: Clone>(items: &[T], ctx: &SearchCtx) -> Result<(Vec<T>, Option<String>)> {
    let start = match ctx.cursor.as_deref() {
        None => 0,
        Some(c) => c.parse::<usize>().with_context(|| format!("the search position `{c}` is not valid"))?,
    };
    let end = start.saturating_add(ctx.page_size.max(1)).min(items.len());
    let page = items.get(start..end).unwrap_or_default().to_vec();
    Ok((page, (end < items.len()).then(|| end.to_string())))
}

/// A value an adapter loaded and may need again within minutes (a board listing that every page
/// and every posting read comes out of). Concurrent callers wait for one load.
pub(crate) struct Cached<T> {
    ttl: Duration,
    slot: tokio::sync::Mutex<Option<(Instant, Arc<T>)>>,
}

impl<T> Cached<T> {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self { ttl, slot: tokio::sync::Mutex::new(None) }
    }

    pub(crate) async fn get_or_load<F, Fut>(&self, load: F) -> Result<Arc<T>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        let mut slot = self.slot.lock().await;
        if let Some((at, value)) = slot.as_ref() {
            if at.elapsed() < self.ttl {
                return Ok(value.clone());
            }
        }
        let value = Arc::new(load().await?);
        *slot = Some((Instant::now(), value.clone()));
        Ok(value)
    }
}

/// How long a listing stays cached.
pub(crate) const LISTING_TTL: Duration = Duration::from_secs(300);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewSource, SourcePurpose, Store};

    #[test]
    fn board_slugs_from_urls_and_names() {
        let gh = ["greenhouse.io"];
        assert_eq!(board_slug("acme", &gh).unwrap(), "acme");
        assert_eq!(board_slug("https://boards.greenhouse.io/acme", &gh).unwrap(), "acme");
        assert_eq!(board_slug("job-boards.greenhouse.io/acme/jobs/12", &gh).unwrap(), "acme");
        assert_eq!(board_slug("https://boards.greenhouse.io/embed/job_board?for=acme", &gh).unwrap(), "acme");
        assert!(board_slug("", &gh).is_err());
        assert!(board_slug("https://evil.test/acme", &gh).unwrap_err().to_string().contains("evil.test"));
        assert!(board_slug("a b", &gh).is_err());
        assert!(board_slug("https://boards.greenhouse.io/", &gh).is_err());
    }

    #[test]
    fn titles_are_matched_to_roles_loosely() {
        let roles = vec!["Senior Backend Engineer".to_string()];
        assert!(title_fits_roles("Software Engineer, Payments", &roles));
        assert!(title_fits_roles("Engineering Manager", &roles), "engineer / engineering");
        assert!(title_fits_roles("Backend Developer", &roles));
        assert!(!title_fits_roles("Account Executive", &roles));
        assert!(!title_fits_roles("Senior Recruiter", &roles), "seniority words do not count");
        assert!(title_fits_roles("Anything", &[]), "no roles, no thinning");
        assert!(title_fits_roles("Anything", &["Senior".to_string()]), "only noise words: no thinning");
    }

    #[test]
    fn terms_match_as_words() {
        assert!(mentions("We use Rust and Go.", "go"));
        assert!(!mentions("Google builds things", "go"));
        assert!(mentions("C++ and .NET shop", "c++"));
        assert!(mentions("a .NET shop", ".net"));
        assert!(!mentions("anything", ""));
    }

    #[test]
    fn pages_are_cut_by_offset_cursor() {
        let items: Vec<u32> = (0..5).collect();
        let ctx = |cursor: Option<&str>| SearchCtx { cursor: cursor.map(str::to_string), page_size: 2 };
        assert_eq!(page_of(&items, &ctx(None)).unwrap(), (vec![0, 1], Some("2".into())));
        assert_eq!(page_of(&items, &ctx(Some("2"))).unwrap(), (vec![2, 3], Some("4".into())));
        assert_eq!(page_of(&items, &ctx(Some("4"))).unwrap(), (vec![4], None));
        assert_eq!(page_of(&items, &ctx(Some("9"))).unwrap(), (vec![], None));
        assert!(page_of(&items, &ctx(Some("x"))).is_err());
        assert_eq!(page_of(&[] as &[u32], &ctx(None)).unwrap(), (vec![], None));
    }

    #[test]
    fn text_helpers() {
        assert_eq!(unescape_markup("&lt;p&gt;a &amp; b&lt;/p&gt;"), "<p>a & b</p>");
        let text = html_to_text("<p>Hello <b>world</b></p><ul><li>one</li><li>two</li></ul>");
        assert!(text.contains("Hello") && text.contains("one") && text.contains("two"), "{text}");
        assert_eq!(one_line("a \n b   c", 10), "a b c");
        assert_eq!(one_line("abcdefghij", 4), "abcd...");
    }

    #[tokio::test]
    async fn the_cache_loads_once() {
        let cache = Cached::new(Duration::from_secs(60));
        let loads = std::sync::atomic::AtomicUsize::new(0);
        for _ in 0..3 {
            let v = cache
                .get_or_load(|| async {
                    loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(7)
                })
                .await
                .unwrap();
            assert_eq!(*v, 7);
        }
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
        let expired = Cached::new(Duration::ZERO);
        expired.get_or_load(|| async { Ok(1) }).await.unwrap();
        assert_eq!(*expired.get_or_load(|| async { Ok(2) }).await.unwrap(), 2, "an expired entry is reloaded");
    }

    #[test]
    fn the_factory_builds_by_kind_and_explains_failures() {
        let store = Store::open_in_memory().unwrap();
        let add = |kind: &str, url: &str| {
            store.add_source(&NewSource { kind: kind.into(), url: url.into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap()
        };
        let http = FakeHttp::with([]);
        assert!(adapter_for(&add("greenhouse", "acme"), http.clone()).is_ok());
        assert!(adapter_for(&add("lever", "https://jobs.lever.co/acme"), http.clone()).is_ok());
        assert!(adapter_for(&add("ashby", "jobs.ashbyhq.com/acme"), http.clone()).is_ok());
        assert!(adapter_for(&add("hn_hiring", ""), http.clone()).is_ok());
        let err = |s: Source| format!("{:#}", adapter_for(&s, http.clone()).err().unwrap());
        assert!(err(add("greenhouse", "")).contains("no company board"));
        assert!(err(add("yc", "")).contains("no adapter for source kind `yc`"));
    }
}
