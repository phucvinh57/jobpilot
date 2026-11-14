//! Workday: the job search a Workday board page itself calls
//! (`POST https://<tenant>.wd<N>.myworkdayjobs.com/wday/cxs/<tenant>/<site>/jobs`).
//!
//! It is not a documented API, so the adapter is best effort: an answer of another shape is a clear
//! error, never a guess. The board is searched once per wanted role (the search text comes from the
//! preferences only), 20 postings at a time; a posting's text is one request for that posting. Every
//! address is built here from the board the user entered and the posting's path; nothing in an
//! answer chooses a host.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{html_to_text, title_fits_roles, Candidate, Http, JobSource, SearchCtx, SearchPage};
use crate::engine::prefs::Preferences;

/// The most postings Workday returns per request (it answers 400 above that).
const PAGE_LIMIT: usize = 20;
/// How deep one role's search goes.
const MAX_OFFSET_PER_ROLE: usize = 200;
/// How deep the search goes when no role is set (everything on the board, newest first).
const MAX_OFFSET_NO_ROLES: usize = 100;

const UNKNOWN_FORMAT: &str = "Workday answered in an unknown format";

/// A Workday board: `https://<tenant>.wd<N>.myworkdayjobs.com/[<locale>/]<site>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    /// `<tenant>.wd<N>.myworkdayjobs.com`, lower case.
    pub host: String,
    pub tenant: String,
    pub site: String,
}

impl Board {
    /// Read the board URL the user entered (with or without `https://`). Only
    /// `<tenant>.wd<N>.myworkdayjobs.com` hosts are accepted, and the tenant and site are plain
    /// names (`[A-Za-z0-9_-]`), so the API path cannot be walked out of.
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        if input.is_empty() {
            bail!("no Workday board is set: enter the board URL, e.g. https://acme.wd5.myworkdayjobs.com/External");
        }
        let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
        let url = url::Url::parse(&with_scheme).with_context(|| format!("`{input}` is not a Workday board URL"))?;
        if !matches!(url.scheme(), "http" | "https") || url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
            bail!("`{input}` is not a Workday board URL");
        }
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        let labels: Vec<&str> = host.split('.').collect();
        let tenant = match labels.as_slice() {
            [tenant, wd, "myworkdayjobs", "com"] if is_name(tenant) && is_data_center(wd) => tenant.to_string(),
            _ => bail!("`{host}` is not a Workday board: the address looks like https://<company>.wd<N>.myworkdayjobs.com/<site>"),
        };
        let segments: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
        let site = match segments.as_slice() {
            [first, second, ..] if is_locale(first) => *second,
            [first, ..] => *first,
            [] => "",
        };
        if site.is_empty() || is_locale(site) {
            bail!("the board's site name is missing in `{input}` (https://{host}/<site>)");
        }
        if !is_name(site) {
            bail!("`{site}` is not a Workday site name");
        }
        Ok(Self { host, tenant, site: site.to_string() })
    }

    fn search_url(&self) -> String {
        format!("https://{}/wday/cxs/{}/{}/jobs", self.host, self.tenant, self.site)
    }

    fn detail_url(&self, path: &str) -> String {
        format!("https://{}/wday/cxs/{}/{}{path}", self.host, self.tenant, self.site)
    }

    /// The posting's page on the board: what `externalUrl` says for a posting of this board.
    fn page_url(&self, path: &str) -> String {
        format!("https://{}/{}{path}", self.host, self.site)
    }

    /// The posting path of a page URL made by [`Board::page_url`]; `None` for any other address.
    fn path_of(&self, page_url: &str) -> Option<String> {
        let url = url::Url::parse(page_url).ok()?;
        if url.host_str()? != self.host {
            return None;
        }
        let path = url.path().strip_prefix('/')?.strip_prefix(self.site.as_str())?;
        is_posting_path(path).then(|| path.to_string())
    }
}

fn is_name(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// `wd1`, `wd5`, `wd103`, ...
fn is_data_center(label: &str) -> bool {
    label.strip_prefix("wd").is_some_and(|n| (1..=3).contains(&n.len()) && n.chars().all(|c| c.is_ascii_digit()))
}

/// `en-US`, `fr-FR`, `de`.
fn is_locale(s: &str) -> bool {
    let letters = |p: &str| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic());
    match s.split_once('-') {
        Some((lang, region)) => letters(lang) && letters(region),
        None => letters(s),
    }
}

/// A posting path as Workday lists it (`/job/US-CA-Remote/Engineer_JR1`): it only ever extends the
/// board's own path, never leaves it.
fn is_posting_path(path: &str) -> bool {
    path.starts_with('/')
        && !path.contains("..")
        && !path.contains("//")
        && path.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '?' | '#' | '\\'))
}

/// One search per wanted role, in the order of the preferences; one empty search when none is set.
fn queries(roles: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    let wanted: Vec<String> =
        roles.iter().map(|r| r.trim().to_string()).filter(|r| !r.is_empty() && seen.insert(r.to_lowercase())).collect();
    if wanted.is_empty() {
        vec![String::new()]
    } else {
        wanted
    }
}

/// `"<role index>:<offset>"`; `None` is the start.
fn parse_cursor(cursor: Option<&str>) -> Result<(usize, usize)> {
    let Some(c) = cursor else { return Ok((0, 0)) };
    let invalid = || format!("the search position `{c}` is not valid");
    let (role, offset) = c.split_once(':').with_context(invalid)?;
    Ok((role.parse().with_context(invalid)?, offset.parse().with_context(invalid)?))
}

pub struct Workday {
    http: Arc<dyn Http>,
    board: Board,
    company: String,
    /// Posting path -> the first role whose search returned it, so a posting found by two roles is
    /// listed once, under the first, and a page searched again is the same page.
    seen: Mutex<HashMap<String, usize>>,
}

impl Workday {
    /// `company`: the name given to every posting (the source's name, or the tenant).
    pub fn new(http: Arc<dyn Http>, board: Board, company: &str) -> Self {
        Self { http, board, company: company.to_string(), seen: Mutex::default() }
    }

    /// A listed posting: its path and its candidate; `None` without a title or a usable path.
    fn candidate(&self, p: &Value) -> Option<(String, Candidate)> {
        let text = |key: &str| p.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string();
        let (title, path) = (text("title"), text("externalPath"));
        if title.is_empty() || !is_posting_path(&path) {
            return None;
        }
        let candidate = Candidate {
            url: self.board.page_url(&path),
            title,
            company: self.company.clone(),
            location: text("locationsText"),
            // "Posted 3 Days Ago", as Workday writes it.
            snippet: text("postedOn"),
        };
        Some((path, candidate))
    }
}

#[async_trait]
impl JobSource for Workday {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let queries = queries(&prefs.roles);
        let (role, offset) = parse_cursor(ctx.cursor.as_deref())?;
        let Some(query) = queries.get(role) else { return Ok(SearchPage::default()) };
        let limit = ctx.page_size.clamp(1, PAGE_LIMIT);
        let max_offset = if query.is_empty() { MAX_OFFSET_NO_ROLES } else { MAX_OFFSET_PER_ROLE };
        let body = json!({ "appliedFacets": {}, "limit": limit, "offset": offset, "searchText": query });
        let answer = self.http.post_json(&self.board.search_url(), &body).await.context("reading the Workday board")?;
        let postings = answer.get("jobPostings").and_then(Value::as_array).context(UNKNOWN_FORMAT)?;
        // Some boards only count on the first page; 0 then means "not said".
        let total = answer.get("total").and_then(Value::as_u64).map_or(0, |t| usize::try_from(t).unwrap_or(usize::MAX));

        let candidates = {
            let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            postings
                .iter()
                .filter_map(|p| self.candidate(p))
                .filter(|(path, _)| {
                    let first = seen.entry(path.clone()).or_insert(role);
                    *first = (*first).min(role);
                    *first == role
                })
                .map(|(_, c)| c)
                .filter(|c| title_fits_roles(&c.title, &prefs.roles))
                .collect()
        };

        let end = offset.saturating_add(postings.len());
        let more_here = postings.len() >= limit && end < max_offset && (total == 0 || end < total);
        let next = if more_here {
            Some(format!("{role}:{end}"))
        } else {
            (role + 1 < queries.len()).then(|| format!("{}:0", role + 1))
        };
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let Some(path) = self.board.path_of(&candidate.url) else { return Ok(None) };
        let answer = self.http.get_json(&self.board.detail_url(&path)).await.context("reading the Workday posting")?;
        let info = answer.get("jobPostingInfo").filter(|v| v.is_object()).context(UNKNOWN_FORMAT)?;
        // `externalUrl` is not read: the posting's address is built from the board (above), so
        // an answer cannot send the user or the runner to another host.
        let text = |key: &str| info.get(key).and_then(Value::as_str).map(str::trim).unwrap_or_default();
        let body = html_to_text(text("jobDescription"));
        if body.is_empty() {
            return Ok(None);
        }
        let title = Some(text("title")).filter(|t| !t.is_empty()).unwrap_or(&candidate.title);
        let mut locations: Vec<&str> = vec![text("location")];
        locations.extend(info.get("additionalLocations").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str));
        let locations: Vec<&str> = locations.into_iter().map(str::trim).filter(|l| !l.is_empty()).collect();
        let location = if locations.is_empty() { candidate.location.clone() } else { locations.join("; ") };
        let mut head = format!("# {title}\n\nCompany: {}\nLocation: {location}", candidate.company);
        if !text("timeType").is_empty() {
            head.push_str(&format!("\nTime type: {}", text("timeType")));
        }
        Ok(Some(format!("{head}\n\n{body}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::discover::source::FakeHttp;

    const LIST: &str = include_str!("../fixtures/workday_jobs.json");
    const JOB: &str = include_str!("../fixtures/workday_job.json");
    const SEARCH: &str = "https://acme.wd5.myworkdayjobs.com/wday/cxs/acme/External/jobs";
    const DETAIL: &str =
        "https://acme.wd5.myworkdayjobs.com/wday/cxs/acme/External/job/US-CA-Remote/Site-Reliability-Engineer_JR1000003";

    fn board() -> Board {
        Board::parse("https://acme.wd5.myworkdayjobs.com/en-US/External").unwrap()
    }

    fn adapter(http: Arc<FakeHttp>) -> Workday {
        Workday::new(http, board(), "Acme")
    }

    fn body(query: &str, offset: usize, limit: usize) -> Value {
        json!({ "appliedFacets": {}, "limit": limit, "offset": offset, "searchText": query })
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    fn roles(r: &[&str]) -> Preferences {
        Preferences { roles: r.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    /// A listing answer of `n` postings named `<prefix><i>`, out of `total`.
    fn listing(prefix: &str, n: usize, total: usize) -> &'static str {
        let postings: Vec<Value> = (0..n)
            .map(|i| json!({ "title": format!("Engineer {prefix}{i}"), "externalPath": format!("/job/X/{prefix}{i}"), "locationsText": "X", "postedOn": "Posted Today" }))
            .collect();
        Box::leak(json!({ "total": total, "jobPostings": postings }).to_string().into_boxed_str())
    }

    #[test]
    fn board_urls_are_read_strictly() {
        let b = board();
        assert_eq!((b.host.as_str(), b.tenant.as_str(), b.site.as_str()), ("acme.wd5.myworkdayjobs.com", "acme", "External"));
        assert_eq!(Board::parse("Acme.WD12.myworkdayjobs.com/Careers_2/job/X/Y_1").unwrap().site, "Careers_2", "a posting URL names its board");
        assert_eq!(Board::parse("https://acme.wd1.myworkdayjobs.com/fr-FR/Jobs?q=x").unwrap().site, "Jobs");
        for bad in [
            "",
            "acme",
            "https://acme.wd5.myworkdayjobs.com",
            "https://acme.wd5.myworkdayjobs.com/en-US",
            "https://acme.wd5.myworkdayjobs.com/..",
            "https://acme.wd5.myworkdayjobs.com/a%2Fb",
            "https://acme.wd5.myworkdayjobs.com/Ext.ernal",
            "https://acme.wd5.myworkdayjobs.com.evil.test/External",
            "https://acme.wd5.myworkdayjobs.com./External",
            "https://myworkdayjobs.com/External",
            "https://wd5.myworkdayjobs.com/External",
            "https://a.b.wd5.myworkdayjobs.com/External",
            "https://acme.www.myworkdayjobs.com/External",
            "https://acme.wd5.myworkdayjobs.com:8443/External",
            "https://user@acme.wd5.myworkdayjobs.com/External",
            "ftp://acme.wd5.myworkdayjobs.com/External",
            "https://acme.myworkdaysite.com/External",
        ] {
            assert!(Board::parse(bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn lists_the_board_with_one_query_per_role() {
        let http = FakeHttp::with_posts([], [(SEARCH, body("Backend Engineer", 0, 20), LIST), (SEARCH, body("SRE", 0, 20), r#"{"total":0,"jobPostings":[]}"#)]);
        let wd = adapter(http.clone());
        let prefs = roles(&["Backend Engineer", " backend engineer ", "", "SRE"]);
        let page = wd.search(&prefs, &ctx(None, 25)).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Senior Backend Engineer, Payments", "Site Reliability Engineer"], "no sales, no untitled posting");
        let first = &page.candidates[0];
        assert_eq!(first.url, "https://acme.wd5.myworkdayjobs.com/External/job/Vietnam-Ho-Chi-Minh-City/Senior-Backend-Engineer--Payments_JR1000001");
        assert_eq!((first.company.as_str(), first.location.as_str(), first.snippet.as_str()), ("Acme", "Vietnam, Ho Chi Minh City", "Posted 3 Days Ago"));
        assert_eq!(page.next.as_deref(), Some("1:0"), "the whole first role fit one request; the second role is next");

        let second = wd.search(&prefs, &ctx(page.next.as_deref(), 25)).await.unwrap();
        assert_eq!((second.candidates.len(), second.next), (0, None));
        let bodies: Vec<Value> = http.posted().into_iter().map(|(url, b)| { assert_eq!(url, SEARCH); b }).collect();
        assert_eq!(bodies, [body("Backend Engineer", 0, 20), body("SRE", 0, 20)], "the search text is the role, nothing else");
    }

    #[tokio::test]
    async fn no_roles_is_one_empty_search_with_a_lower_cap() {
        let pages: Vec<_> = (0..6).map(|i| (SEARCH, body("", i * 20, 20), listing(&format!("p{i}-"), 20, 1000))).collect();
        let http = FakeHttp::with_posts([], pages);
        let wd = adapter(http.clone());
        let (mut cursor, mut seen) = (None, 0);
        loop {
            let page = wd.search(&Preferences::default(), &ctx(cursor.as_deref(), 25)).await.unwrap();
            seen += page.candidates.len();
            cursor = page.next;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, MAX_OFFSET_NO_ROLES);
        assert_eq!(http.posted().len(), MAX_OFFSET_NO_ROLES / PAGE_LIMIT);
    }

    #[tokio::test]
    async fn a_posting_found_by_two_roles_is_listed_once() {
        let rust = r#"{"total":2,"jobPostings":[
            {"title":"Rust Engineer","externalPath":"/job/A/Rust-Engineer_1","locationsText":"A","postedOn":"Posted Today"},
            {"title":"Platform Engineer","externalPath":"/job/A/Platform-Engineer_2","locationsText":"A","postedOn":"Posted Today"}]}"#;
        let platform = r#"{"total":2,"jobPostings":[
            {"title":"Platform Engineer","externalPath":"/job/A/Platform-Engineer_2","locationsText":"A","postedOn":"Posted Today"},
            {"title":"Platform Lead Engineer","externalPath":"/job/A/Platform-Lead_3","locationsText":"A","postedOn":"Posted Today"}]}"#;
        let http = FakeHttp::with_posts([], [(SEARCH, body("Rust Engineer", 0, 10), rust), (SEARCH, body("Platform Engineer", 0, 10), platform)]);
        let wd = adapter(http);
        let prefs = roles(&["Rust Engineer", "Platform Engineer"]);
        let first = wd.search(&prefs, &ctx(None, 10)).await.unwrap();
        assert_eq!(first.candidates.len(), 2);
        let second = wd.search(&prefs, &ctx(first.next.as_deref(), 10)).await.unwrap();
        let titles: Vec<_> = second.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Platform Lead Engineer"], "Platform Engineer came with the first role");
        let again = wd.search(&prefs, &ctx(None, 10)).await.unwrap();
        assert_eq!(again, first, "searching the first role again gives the same page");
    }

    #[tokio::test]
    async fn pages_follow_the_cursor_and_a_cursor_gives_the_same_page() {
        let http = FakeHttp::with_posts(
            [],
            [
                (SEARCH, body("Engineer", 0, 20), listing("a", 20, 45)),
                (SEARCH, body("Engineer", 20, 20), listing("b", 20, 45)),
                (SEARCH, body("Engineer", 40, 20), listing("c", 5, 45)),
            ],
        );
        let wd = adapter(http);
        let prefs = roles(&["Engineer"]);
        let one = wd.search(&prefs, &ctx(None, 25)).await.unwrap();
        assert_eq!((one.candidates.len(), one.next.as_deref()), (20, Some("0:20")));
        let two = wd.search(&prefs, &ctx(Some("0:20"), 25)).await.unwrap();
        assert_eq!(two.next.as_deref(), Some("0:40"));
        assert_eq!(wd.search(&prefs, &ctx(Some("0:20"), 25)).await.unwrap(), two, "a resumed run gets the same page");
        let three = wd.search(&prefs, &ctx(Some("0:40"), 25)).await.unwrap();
        assert_eq!((three.candidates.len(), three.next), (5, None), "the board has 45");
        assert_eq!(wd.search(&prefs, &ctx(Some("3:0"), 25)).await.unwrap(), SearchPage::default(), "past the last role");
        assert!(wd.search(&prefs, &ctx(Some("x"), 25)).await.is_err());
    }

    #[tokio::test]
    async fn a_posting_is_read_from_the_board_by_its_path() {
        let http = FakeHttp::with_posts([(DETAIL, JOB)], [(SEARCH, body("", 0, 20), LIST)]);
        let wd = adapter(http.clone());
        let page = wd.search(&Preferences::default(), &ctx(None, 25)).await.unwrap();
        let sre = page.candidates.iter().find(|c| c.title == "Site Reliability Engineer").unwrap();
        let text = wd.posting(sre).await.unwrap().unwrap();
        assert!(text.starts_with("# Site Reliability Engineer\n\nCompany: Acme\nLocation: US, CA, Remote; US, TX, Remote; US, NY, Remote\nTime type: Full time"), "{text}");
        assert!(text.contains("Run Kubernetes clusters") && text.contains("Terraform and Go") && text.contains("What You'll Be Doing"), "{text}");
        assert!(!text.contains('<'), "{text}");
        assert_eq!(http.requested().last().map(String::as_str), Some(DETAIL));
        let elsewhere = Candidate { url: "https://other.wd5.myworkdayjobs.com/External/job/X/Y_1".into(), ..sre.clone() };
        assert_eq!(wd.posting(&elsewhere).await.unwrap(), None, "not a posting of this board");
    }

    #[tokio::test]
    async fn an_external_url_on_another_host_is_ignored() {
        let foreign = JOB.replace("https://acme.wd5.myworkdayjobs.com/External/job", "https://evil.test/job");
        let http = FakeHttp::with_posts([(DETAIL, Box::leak(foreign.into_boxed_str()) as &str)], [(SEARCH, body("", 0, 20), LIST)]);
        let wd = adapter(http.clone());
        let page = wd.search(&Preferences::default(), &ctx(None, 25)).await.unwrap();
        assert!(page.candidates.iter().all(|c| c.url.starts_with("https://acme.wd5.myworkdayjobs.com/External/job/")));
        let sre = page.candidates.iter().find(|c| c.title == "Site Reliability Engineer").unwrap();
        let text = wd.posting(sre).await.unwrap().unwrap();
        assert!(!text.contains("evil.test"), "{text}");
        assert!(http.requested().iter().all(|u| u.starts_with("https://acme.wd5.myworkdayjobs.com/")));
    }

    #[tokio::test]
    async fn listed_paths_cannot_leave_the_board() {
        let odd = r#"{"total":3,"jobPostings":[
            {"title":"A Engineer","externalPath":"/../../other/jobs","locationsText":"","postedOn":""},
            {"title":"B Engineer","externalPath":"//evil.test/x","locationsText":"","postedOn":""},
            {"title":"C Engineer","externalPath":"job/no-slash","locationsText":"","postedOn":""}]}"#;
        let http = FakeHttp::with_posts([], [(SEARCH, body("", 0, 20), odd)]);
        let page = adapter(http).search(&Preferences::default(), &ctx(None, 25)).await.unwrap();
        assert!(page.candidates.is_empty(), "{page:?}");
    }

    #[tokio::test]
    async fn an_unknown_shape_is_a_clear_error() {
        let http = FakeHttp::with_posts([(DETAIL, r#"{"job": {}}"#)], [(SEARCH, body("", 0, 20), r#"{"total": 3, "items": []}"#)]);
        let wd = adapter(http);
        let err = format!("{:#}", wd.search(&Preferences::default(), &ctx(None, 25)).await.unwrap_err());
        assert!(err.contains("Workday answered in an unknown format"), "{err}");
        let sre = Candidate { url: board().page_url("/job/US-CA-Remote/Site-Reliability-Engineer_JR1000003"), ..Default::default() };
        let err = format!("{:#}", wd.posting(&sre).await.unwrap_err());
        assert!(err.contains("Workday answered in an unknown format"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_request_names_the_source_and_its_status() {
        let wd = adapter(FakeHttp::with([]));
        let err = format!("{:#}", wd.search(&Preferences::default(), &ctx(None, 25)).await.unwrap_err());
        assert!(err.contains("reading the Workday board") && err.contains("HTTP 404"), "{err}");
    }
}
