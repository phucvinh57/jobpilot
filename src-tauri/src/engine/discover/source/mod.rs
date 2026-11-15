//! Job sources: code that turns the preferences into candidate postings. The model never picks
//! URLs; every candidate comes from an adapter reading a public API or page.
//!
//! | `sources.kind` | `sources.url` | How |
//! |----------------|---------------|-----|
//! | `greenhouse` | board URL or slug (`boards.greenhouse.io/acme`) | public board JSON |
//! | `lever` | `jobs.lever.co/acme` or slug | public postings JSON |
//! | `ashby` | `jobs.ashbyhq.com/acme` or slug | public job-board JSON |
//! | `smartrecruiters` | `jobs.smartrecruiters.com/Acme` or company id | public postings JSON, one API page (100) at a time |
//! | `recruitee` | `acme.recruitee.com` or name | public offers JSON |
//! | `personio` | `acme.jobs.personio.de` (or `.com`) or name | public XML job feed |
//! | `workday` | `https://acme.wd5.myworkdayjobs.com/[en-US/]External` | the board page's own job search (POST), one query per wanted role |
//! | `hn_hiring` | unused | HN Algolia, the latest "Who is hiring?" thread, one comment per candidate |
//! | `yc` | `https://www.ycombinator.com/jobs` (default) or a workatastartup.com page | the job links of the listing page; the browser (and a bound session) when the page needs it |
//! | `career_page` | a careers page | its `JobPosting` JSON-LD, else the same-site links that look like postings; else the site's `sitemap.xml`; else the browser (a page drawn by JavaScript) |
//!
//! The API adapters read through [`Http`] and a base URL, so tests answer from saved fixture JSON; the
//! page adapters read through [`Pages`] (plain HTTP, then the managed browser), so tests answer from
//! fixture HTML. A login, captcha or block page met in the browser is a `ChallengeFound` error, which
//! the runner turns into a pause for the user.

mod ashby;
pub mod boards;
mod career_page;
mod greenhouse;
mod hn_hiring;
mod http;
pub(crate) mod jsonld;
mod lever;
mod personio;
mod recruitee;
mod sitemap;
mod smartrecruiters;
pub(crate) mod web;
mod workday;
mod yc;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub use ashby::Ashby;
pub use boards::{detect_boards, kind_for_host, BoardHint};
pub use career_page::CareerPage;
pub use greenhouse::Greenhouse;
pub use hn_hiring::HnHiring;
pub use http::{Http, NetHttp};
pub use lever::Lever;
pub use personio::Personio;
pub use recruitee::Recruitee;
pub use smartrecruiters::SmartRecruiters;
pub use web::{NetPages, Page, Pages};
pub use workday::Workday;
pub use yc::Yc;

#[cfg(test)]
pub(crate) use http::fake::FakeHttp;

use crate::engine::pipeline::ingest::clean;
use crate::engine::prefs::Preferences;
use crate::engine::store::Source;

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

    /// The job boards its last listing read pointed to when that read found no postings (a careers
    /// page that embeds a Greenhouse board). Only a suggestion for the user: nothing reads them.
    fn board_hints(&self) -> Vec<BoardHint> {
        Vec::new()
    }
}

/// The adapter for a `sources` row that reads a public API: `Err` for a kind without one (the page
/// adapters, `yc` and `career_page`, are built by [`adapter_for_pages`]) and for a `url` the adapter
/// cannot read, with a message the user can act on.
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
        "smartrecruiters" => {
            let id = board_slug(&source.url, &["smartrecruiters.com"]).context("SmartRecruiters source")?;
            Arc::new(SmartRecruiters::new(http, smartrecruiters::DEFAULT_BASE, &id, &company(&id)))
        }
        "recruitee" => {
            let (name, suffix) = board_host(&source.url, recruitee::HOSTS).context("Recruitee source")?;
            Arc::new(Recruitee::new(http, &format!("https://{name}.{suffix}"), &company(&name)))
        }
        "personio" => {
            let (name, suffix) = board_host(&source.url, personio::HOSTS).context("Personio source")?;
            Arc::new(Personio::new(http, &format!("https://{name}.{suffix}"), &company(&name)))
        }
        "hn_hiring" => Arc::new(HnHiring::new(http, hn_hiring::DEFAULT_BASE)),
        "workday" => {
            let board = workday::Board::parse(&source.url).context("Workday source")?;
            let company = company(&board.tenant);
            Arc::new(Workday::new(http, board, &company))
        }
        other => bail!("no adapter for source kind `{other}` yet"),
    })
}

/// [`adapter_for`] plus the adapters that read web pages through `pages`: `yc` and `career_page`.
pub fn adapter_for_pages(source: &Source, http: Arc<dyn Http>, pages: Arc<dyn Pages>) -> Result<Arc<dyn JobSource>> {
    Ok(match source.kind.as_str() {
        "yc" => Arc::new(Yc::new(pages, source)?),
        "career_page" => {
            let company = match source.name.trim() {
                "" => web::normalize_page_url(&source.url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default(),
                name => name.to_string(),
            };
            Arc::new(CareerPage::new(pages, source, &company)?)
        }
        _ => return adapter_for(source, http),
    })
}

/// Check a new source before it is stored: the adapter that would read it must accept its address.
/// Returns the address to store (an empty one becomes the kind's default where there is one) and a
/// name for it when the user gave none.
pub fn check_source(kind: &str, url: &str) -> Result<(String, String)> {
    let url = url.trim();
    let slug_of = |hosts: &[&str], what: &str| -> Result<(String, String)> {
        let slug = board_slug(url, hosts).context(what.to_string())?;
        Ok((url.to_string(), slug))
    };
    match kind {
        "greenhouse" => slug_of(&["greenhouse.io"], "Greenhouse source"),
        "lever" => slug_of(&["lever.co"], "Lever source"),
        "ashby" => slug_of(&["ashbyhq.com"], "Ashby source"),
        "smartrecruiters" => slug_of(&["smartrecruiters.com"], "SmartRecruiters source"),
        "recruitee" => Ok((url.to_string(), board_host(url, recruitee::HOSTS).context("Recruitee source")?.0)),
        "personio" => Ok((url.to_string(), board_host(url, personio::HOSTS).context("Personio source")?.0)),
        "hn_hiring" => Ok((String::new(), "Hacker News: Who is hiring".to_string())),
        "workday" => Ok((url.to_string(), workday::Board::parse(url).context("Workday source")?.tenant)),
        "yc" => {
            let url = if url.is_empty() { yc::DEFAULT_URL } else { url };
            yc::check_url(url)?;
            Ok((url.to_string(), "Y Combinator jobs".to_string()))
        }
        "career_page" => {
            let parsed = web::normalize_page_url(url).context("career page source")?;
            web::check_site(&parsed).context("career page source")?;
            Ok((url.to_string(), parsed.host_str().unwrap_or_default().trim_start_matches("www.").to_string()))
        }
        other => bail!("no adapter for source kind `{other}` yet"),
    }
}

/// A board name that is safe in an API path: `[A-Za-z0-9._-]`, not empty, and not starting with a dot
/// (`.` and `..`, and anything starting with a dot, would walk out of the API path).
pub(crate) fn is_board_name(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('.') && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The board name in a board URL (`https://boards.greenhouse.io/acme/jobs/1` -> `acme`) or a bare
/// slug. `hosts` are the registrable domains a board URL may have.
pub(crate) fn board_slug(input: &str, hosts: &[&str]) -> Result<String> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no company board is set: enter the board URL or its short name");
    }
    let valid = is_board_name;
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

/// The company of a board that lives on its own subdomain (`https://acme.recruitee.com/o/x` with
/// `suffixes` `["recruitee.com"]` -> `acme`) or a bare name, and the suffix the address used (the
/// first one for a bare name). The name becomes a host label in the address the adapter builds, so
/// it must be exactly one label of `[A-Za-z0-9_-]` (no dots, so no `..` and no other domain); it is
/// lower-cased.
pub(crate) fn board_host(input: &str, suffixes: &[&'static str]) -> Result<(String, &'static str)> {
    let input = input.trim();
    if input.is_empty() {
        bail!("no company board is set: enter the board URL or its short name");
    }
    let Some(&default) = suffixes.first() else { bail!("no board host is known for this kind") };
    let valid = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if !input.contains(['/', ':', '.']) {
        return if valid(input) { Ok((input.to_ascii_lowercase(), default)) } else { bail!("`{input}` is not a board name") };
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let url = url::Url::parse(&with_scheme).with_context(|| format!("`{input}` is not a board URL"))?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    for &suffix in suffixes {
        if let Some(label) = host.strip_suffix(suffix).and_then(|h| h.strip_suffix('.')) {
            return if valid(label) { Ok((label.to_string(), suffix)) } else { bail!("`{host}` is not a company board on {suffix}") };
        }
    }
    let forms: Vec<String> = suffixes.iter().map(|s| format!("<company>.{s}")).collect();
    bail!("`{host}` is not {}", forms.join(" or "))
}

// ---------------------------------------------------------------- shared helpers

/// Lower-case words of `text` (letters and digits).
pub(crate) fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Words that say how senior or which kind of job it is, not what it is: they do not decide
/// whether a title fits a wanted role.
pub(crate) const ROLE_NOISE: &[&str] = &["senior", "junior", "staff", "principal", "lead", "head", "the", "and", "for", "intern"];

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
    mentions_lowered(&text.to_lowercase(), term)
}

/// [`mentions`] for a `text` that is lower case already (matched against many terms).
pub(crate) fn mentions_lowered(text: &str, term: &str) -> bool {
    let term = term.trim().to_lowercase();
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
    use crate::engine::store::{NewSource, SourcePurpose, Store};

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
        for dots in [".", "..", ".hidden", "https://boards.greenhouse.io/..", "https://boards.greenhouse.io/.", "https://boards.greenhouse.io/embed/job_board?for=.."] {
            assert!(board_slug(dots, &gh).is_err(), "{dots}");
        }
        assert_eq!(board_slug("a.b-c_d", &gh).unwrap(), "a.b-c_d", "dots inside a name are fine");
        assert!(board_slug("https://boards.greenhouse.io/", &gh).is_err());
    }

    #[test]
    fn board_hosts_from_subdomains_and_names() {
        let rc = ["recruitee.com"];
        assert_eq!(board_host("acme", &rc).unwrap(), ("acme".into(), "recruitee.com"));
        assert_eq!(board_host(" Acme-Labs_2 ", &rc).unwrap(), ("acme-labs_2".into(), "recruitee.com"), "lower-cased");
        assert_eq!(board_host("acme.recruitee.com", &rc).unwrap().0, "acme");
        assert_eq!(board_host("https://ACME.recruitee.com/o/backend-engineer?x=1", &rc).unwrap().0, "acme");
        let pe = ["jobs.personio.de", "jobs.personio.com"];
        assert_eq!(board_host("acme", &pe).unwrap(), ("acme".into(), "jobs.personio.de"), "a bare name gets the first suffix");
        assert_eq!(board_host("https://acme.jobs.personio.com/job/1", &pe).unwrap(), ("acme".into(), "jobs.personio.com"));
        assert_eq!(board_host("acme.jobs.personio.de", &pe).unwrap(), ("acme".into(), "jobs.personio.de"));
        for bad in [
            "",
            ".",
            "..",
            "a b",
            "acme/..",
            "recruitee.com",
            ".recruitee.com",
            "a.b.recruitee.com",
            "...recruitee.com",
            "recruitee.com.evil.test",
            "acme.recruitee.com.evil.test",
            "https://evil.test/acme.recruitee.com",
            "acmerecruitee.com",
            "https://acme.recruitee.com@evil.test/",
            "javascript:alert(1)",
        ] {
            assert!(board_host(bad, &rc).is_err(), "{bad}");
        }
        assert!(board_host("acme.personio.de", &pe).unwrap_err().to_string().contains("<company>.jobs.personio.de or <company>.jobs.personio.com"));
        assert!(board_host("acme.jobs.personio.de.evil.test", &pe).is_err());
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
        assert!(adapter_for(&add("smartrecruiters", "jobs.smartrecruiters.com/Acme"), http.clone()).is_ok());
        assert!(adapter_for(&add("recruitee", "acme.recruitee.com"), http.clone()).is_ok());
        assert!(adapter_for(&add("personio", "acme.jobs.personio.com"), http.clone()).is_ok());
        let err = |s: Source| format!("{:#}", adapter_for(&s, http.clone()).err().unwrap());
        assert!(err(add("greenhouse", "")).contains("no company board"));
        assert!(err(add("recruitee", "recruitee.com.evil.test")).contains("Recruitee source"));
        assert!(err(add("personio", "..")).contains("Personio source"));
        assert!(err(add("smartrecruiters", "https://evil.test/Acme")).contains("SmartRecruiters source"));
        assert!(adapter_for(&add("workday", "https://acme.wd5.myworkdayjobs.com/External"), http.clone()).is_ok());
        assert!(err(add("workday", "https://acme.wd5.myworkdayjobs.com.evil.test/External")).contains("not a Workday board"));
        assert!(err(add("nope", "")).contains("no adapter for source kind `nope`"));
        assert!(err(add("yc", "")).contains("no adapter for source kind `yc`"), "the page adapters need `adapter_for_pages`");
    }

    #[test]
    fn the_page_factory_builds_the_browser_adapters_and_hands_the_rest_on() {
        let store = Store::open_in_memory().unwrap();
        let add = |kind: &str, url: &str| {
            store.add_source(&NewSource { kind: kind.into(), url: url.into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap()
        };
        let (http, pages) = (FakeHttp::with([]), web::fake::pages(web::fake::FakePages::new()));
        let build = |s: Source| adapter_for_pages(&s, http.clone(), pages.clone());
        assert!(build(add("yc", "")).is_ok(), "the default listing");
        assert!(build(add("career_page", "acme.test/careers")).is_ok());
        assert!(build(add("greenhouse", "acme")).is_ok(), "an API kind goes to adapter_for");
        let err = |s: Source| format!("{:#}", build(s).err().unwrap());
        assert!(err(add("career_page", "")).contains("no page address"));
        assert!(err(add("career_page", "ftp://x.test")).contains("http"));
        assert!(err(add("yc", "https://evil.test/jobs")).contains("not a ycombinator.com"));
        assert!(err(add("nope", "")).contains("no adapter"));
    }

    #[test]
    fn a_new_source_is_checked_and_named_before_it_is_stored() {
        assert_eq!(check_source("greenhouse", " https://boards.greenhouse.io/acme ").unwrap(), ("https://boards.greenhouse.io/acme".into(), "acme".into()));
        assert_eq!(check_source("lever", "acme").unwrap(), ("acme".into(), "acme".into()));
        assert_eq!(check_source("ashby", "jobs.ashbyhq.com/acme").unwrap().1, "acme");
        assert_eq!(check_source("hn_hiring", "ignored").unwrap().0, "");
        assert_eq!(
            check_source("workday", " acme.wd5.myworkdayjobs.com/en-US/External ").unwrap(),
            ("acme.wd5.myworkdayjobs.com/en-US/External".into(), "acme".into())
        );
        assert_eq!(check_source("smartrecruiters", "https://jobs.smartrecruiters.com/AcmeGroup").unwrap().1, "AcmeGroup");
        assert_eq!(check_source("smartrecruiters", "AcmeGroup").unwrap(), ("AcmeGroup".into(), "AcmeGroup".into()));
        assert_eq!(check_source("recruitee", " https://acme.recruitee.com/ ").unwrap(), ("https://acme.recruitee.com/".into(), "acme".into()));
        assert_eq!(check_source("personio", "acme.jobs.personio.com").unwrap().1, "acme");
        assert_eq!(check_source("personio", "acme").unwrap().1, "acme");
        assert_eq!(check_source("yc", "").unwrap(), (yc::DEFAULT_URL.into(), "Y Combinator jobs".into()));
        assert_eq!(check_source("career_page", "https://www.acme.test/careers").unwrap(), ("https://www.acme.test/careers".into(), "acme.test".into()));
        for (kind, url) in [("greenhouse", ""), ("lever", "https://evil.test/acme"), ("career_page", "javascript:x"), ("yc", "https://evil.test"), ("linkedin", "https://www.linkedin.com/jobs"),
            ("smartrecruiters", ""), ("smartrecruiters", ".."), ("smartrecruiters", "https://smartrecruiters.com.evil.test/Acme"),
            ("recruitee", ""), ("recruitee", "recruitee.com.evil.test"), ("recruitee", "a.b.recruitee.com"), ("recruitee", "https://evil.test/o/x"),
            ("personio", ".."), ("personio", "acme.jobs.personio.de.evil.test"), ("personio", "acme.personio.de"), ("personio", "jobs.personio.de"),
            ("workday", ""), ("workday", "https://acme.wd5.myworkdayjobs.com/.."), ("workday", "myworkdayjobs.com.evil.test/External")] {
            assert!(check_source(kind, url).is_err(), "{kind} {url}");
        }
    }
}
