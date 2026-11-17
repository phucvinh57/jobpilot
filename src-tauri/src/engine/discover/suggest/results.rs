//! Reading a search results page for job sources, in code.
//!
//! The page is untrusted: its links are only **candidates**, and code decides what one is. A link is
//! kept when it is
//!
//! - on a job board the app knows (`boards::board_at`: Greenhouse, Lever, Ashby, SmartRecruiters,
//!   Recruitee, Personio, Workday): that board, as the source to add (one per board);
//! - or a page that looks like a place where a company lists jobs: a `careers.` / `jobs.` host, or a
//!   path segment that is a job word (`web::JOB_WORDS`). The page address is cut after that segment
//!   (`/en/careers/rust-dev-42` -> `/en/careers`), as a `career_page` source (one per host).
//!
//! Search engines' own links (`/url?q=` redirects are unwrapped first), other job aggregators (an
//! ATS host that is not a known board, LinkedIn), social sites, single-label and private hosts are
//! dropped. So are the sources the user has, the suggestions already stored (any state: a dismissed
//! one never comes back) and companies the preferences exclude. Every kept candidate is checked
//! afterwards (the verify step) before it is suggested.

use std::collections::HashSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::engine::discover::score::excluded_company;
use crate::engine::discover::source::boards::{board_at, kind_for_host};
use crate::engine::discover::source::web::{links, normalize_page_url, Page, JOB_WORDS};
use crate::engine::mail::classify::{is_ats_host, registrable_domain};
use crate::engine::net::http::is_public_host;
use crate::engine::prefs::Preferences;
use crate::engine::store::{SourcePurpose, Store};

/// A job source a results page points to, before it is checked.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Found {
    /// The source kind: a board kind or `career_page`.
    pub kind: String,
    /// What to add (and to check): `boards.greenhouse.io/acme`, `https://careers.acme.com/`.
    pub input: String,
    /// A name until the check reads a better one: the board's name or the page's host.
    pub name: String,
    /// The search that found it.
    pub query: String,
}

/// Registrable-domain names of search engines and the services their result pages link to.
const ENGINE_NAMES: [&str; 16] = [
    "google", "gstatic", "googleusercontent", "googleapis", "googleadservices", "googlesyndication", "googletagmanager",
    "doubleclick", "youtube", "youtu", "blogger", "bing", "msn", "duckduckgo", "yahoo", "schema",
];
/// Registrable-domain names of sites that talk about jobs but list none of a company's.
const NOISE_NAMES: [&str; 14] = [
    "wikipedia", "reddit", "quora", "medium", "facebook", "twitter", "x", "instagram", "tiktok", "pinterest", "indeed",
    "glassdoor", "ziprecruiter", "monster",
];
/// First host labels that mark a careers site.
const JOB_HOST_LABELS: [&str; 4] = ["careers", "career", "jobs", "job"];

/// The sources a key is compared by: `(kind, id)`, where the id is the board's name (lower case;
/// the whole address for Workday) or the careers page's host without `www.`.
pub(crate) type Key = (String, String);

/// What must not be suggested: the job sources the user has and every stored suggestion.
#[derive(Clone, Debug, Default)]
pub(crate) struct Known {
    keys: HashSet<Key>,
}

impl Known {
    /// The user's job sources and every suggestion ever stored (added and dismissed ones too).
    pub(crate) fn load(store: &Store) -> Result<Self> {
        let mut known = Self::default();
        for source in store.list_sources(SourcePurpose::Jobs)? {
            known.insert(&source.kind, &source.url);
        }
        for suggestion in store.all_suggestions()? {
            known.insert(&suggestion.kind, &suggestion.input);
        }
        Ok(known)
    }

    /// Remember a source; `false` when it was known already.
    pub(crate) fn insert(&mut self, kind: &str, input: &str) -> bool {
        self.keys.insert(key_of(kind, input))
    }

    pub(crate) fn contains(&self, kind: &str, input: &str) -> bool {
        self.keys.contains(&key_of(kind, input))
    }
}

/// The [`Key`] of a source of `kind` with the address (or bare board name) `input`.
pub(crate) fn key_of(kind: &str, input: &str) -> Key {
    let input = input.trim();
    let id = match kind {
        "career_page" => normalize_page_url(input).ok().map(|u| bare_host(&u)),
        _ if kind_for_kind(kind) => board_id(kind, input),
        _ => None,
    };
    (kind.to_string(), id.unwrap_or_else(|| input.to_ascii_lowercase()))
}

/// Is `kind` one of the board kinds?
fn kind_for_kind(kind: &str) -> bool {
    matches!(kind, "greenhouse" | "lever" | "ashby" | "smartrecruiters" | "recruitee" | "personio" | "workday")
}

/// The board name of a board source's input: a bare name as typed, or the name in its address.
fn board_id(kind: &str, input: &str) -> Option<String> {
    if input.is_empty() {
        return None;
    }
    if !input.contains(['/', '.', ':']) {
        return Some(input.to_ascii_lowercase());
    }
    let with_scheme = if input.contains("://") { input.to_string() } else { format!("https://{input}") };
    let hint = board_at(&Url::parse(&with_scheme).ok()?)?;
    if hint.kind != kind {
        return None;
    }
    let id = match kind {
        "recruitee" | "personio" => hint.input.split('.').next()?.to_string(),
        "workday" => hint.input.clone(),
        _ => hint.input.rsplit('/').next()?.to_string(),
    };
    Some(id.to_ascii_lowercase())
}

/// A host without a leading `www.`, lower case.
fn bare_host(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().trim_end_matches('.').to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// The name of a host's registrable domain (`careers.acme.co.uk` -> `acme`).
fn domain_name(host: &str) -> String {
    registrable_domain(host).split('.').next().unwrap_or_default().to_string()
}

/// One kept link of a results page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Hit {
    pub found: Found,
    /// Who the source is likely to be, for the excluded companies: the board's name, or the
    /// careers site's domain name.
    pub company: String,
}

/// The job sources `page` (a results page of the search at `search`) points to, in page order, each
/// once. `query` is recorded on each.
pub(crate) fn hits_on(page: &Page, search: &Url, query: &str) -> Vec<Hit> {
    let Ok(base) = Url::parse(&page.url) else { return Vec::new() };
    let engines: Vec<String> = [&base, search].iter().map(|u| domain_name(&bare_host(u))).filter(|n| !n.is_empty()).collect();
    let is_engine = |url: &Url| {
        let name = domain_name(&bare_host(url));
        engines.contains(&name) || ENGINE_NAMES.contains(&name.as_str())
    };
    let mut out: Vec<Hit> = Vec::new();
    let mut seen: HashSet<Key> = HashSet::new();
    for link in links(&page.html, &base) {
        let url = if is_engine(&link.url) { redirect_target(&link.url) } else { Some(link.url) };
        let Some(url) = url else { continue };
        if !matches!(url.scheme(), "http" | "https") || !is_public_host(&url) || is_engine(&url) {
            continue;
        }
        let host = bare_host(&url);
        if NOISE_NAMES.contains(&domain_name(&host).as_str()) {
            continue;
        }
        let Some(hit) = classify(&url, &host, query) else { continue };
        if seen.insert(key_of(&hit.found.kind, &hit.found.input)) {
            out.push(hit);
        }
    }
    out
}

/// The address a search engine's redirect link (`/url?q=<target>`, `/url?url=<target>`) leads to;
/// `None` for any other link of the engine.
fn redirect_target(url: &Url) -> Option<Url> {
    if !matches!(url.path(), "/url" | "/interstitial" | "/link") {
        return None;
    }
    url.query_pairs()
        .filter(|(k, _)| k == "q" || k == "url")
        .find_map(|(_, v)| Url::parse(&v).ok().filter(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()))
        .map(|mut u| {
            u.set_fragment(None);
            u
        })
}

/// What one result link is as a source, if anything.
fn classify(url: &Url, host: &str, query: &str) -> Option<Hit> {
    if kind_for_host(host).is_some() {
        let hint = board_at(url)?;
        let name = board_id(hint.kind, &hint.input).unwrap_or_default();
        let company = match hint.kind {
            "workday" => host.split('.').next().unwrap_or_default().to_string(),
            _ => name.replace(['-', '_'], " "),
        };
        return Some(Hit { found: Found { kind: hint.kind.into(), input: hint.input, name, query: query.into() }, company });
    }
    if is_ats_host(host) {
        return None;
    }
    let segments: Vec<&str> = url.path_segments().map(|s| s.filter(|p| !p.is_empty()).collect()).unwrap_or_default();
    let job_segment = segments.iter().position(|s| s.to_ascii_lowercase().split(['-', '_', '.']).any(|t| JOB_WORDS.contains(&t)));
    let job_host = host.split('.').next().is_some_and(|label| JOB_HOST_LABELS.contains(&label)) && host.split('.').count() > 2;
    let path = match job_segment {
        Some(i) => format!("/{}/", segments[..=i].join("/")).trim_end_matches('/').to_string(),
        None if job_host => "/".to_string(),
        None => return None,
    };
    let mut page = url.clone();
    page.set_path(&path);
    page.set_query(None);
    page.set_fragment(None);
    let _ = page.set_username("");
    let _ = page.set_password(None);
    let company = domain_name(host);
    Some(Hit { found: Found { kind: "career_page".into(), input: page.to_string(), name: host.to_string(), query: query.into() }, company })
}

/// The new sources on a results page: [`hits_on`] without the `known` ones and the companies
/// `prefs` excludes.
pub(crate) fn new_sources(page: &Page, search: &Url, query: &str, known: &Known, prefs: &Preferences) -> Vec<Found> {
    hits_on(page, search, query)
        .into_iter()
        .filter(|hit| !known.contains(&hit.found.kind, &hit.found.input))
        .filter(|hit| excluded_company(prefs, &hit.company).is_none())
        .map(|hit| hit.found)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::{NewSource, NewSuggestion, SuggestionState};

    const RESULTS: &str = include_str!("fixtures/google_results.html");

    fn search() -> Url {
        Url::parse("https://www.google.com/search?q=%22Rust+Engineer%22+Berlin&num=20").unwrap()
    }

    fn page() -> Page {
        Page { url: search().to_string(), html: RESULTS.into() }
    }

    fn shown(found: &[Found]) -> Vec<(String, String)> {
        found.iter().map(|f| (f.kind.clone(), f.input.clone())).collect()
    }

    fn pair(kind: &str, input: &str) -> (String, String) {
        (kind.into(), input.into())
    }

    #[test]
    fn the_fixture_gives_boards_and_careers_pages_once_each() {
        let hits = hits_on(&page(), &search(), "q1");
        let found: Vec<Found> = hits.iter().map(|h| h.found.clone()).collect();
        assert_eq!(
            shown(&found),
            [
                pair("greenhouse", "boards.greenhouse.io/acme"),
                pair("lever", "jobs.lever.co/known"),
                pair("career_page", "https://careers.gamma-robotics.com/"),
                pair("ashby", "jobs.ashbyhq.com/beta"),
                pair("career_page", "https://www.bigco.com/en/careers"),
                pair("greenhouse", "boards.greenhouse.io/dismissedco"),
                pair("career_page", "https://careers.megacorp.com/"),
                pair("workday", "https://delta.wd5.myworkdayjobs.com/External"),
                pair("career_page", "https://epsilon.io/jobs"),
            ]
        );
        assert!(found.iter().all(|f| f.query == "q1"));
        assert_eq!(found[0].name, "acme");
        assert_eq!(found[2].name, "careers.gamma-robotics.com");
        assert_eq!(hits[6].company, "megacorp");
    }

    #[test]
    fn google_redirects_are_unwrapped_and_its_own_links_dropped() {
        let html = r#"
            <a href="/url?q=https://jobs.lever.co/acme/123&amp;sa=U&amp;ved=x">Acme</a>
            <a href="https://www.google.com/url?url=https://careers.zeta.com/jobs/7&amp;rct=j">Zeta</a>
            <a href="/search?q=rust+jobs&amp;tbm=nws">News</a>
            <a href="https://maps.google.com/maps?q=careers">Maps</a>
            <a href="https://webcache.googleusercontent.com/search?q=cache:x">Cached</a>
            <a href="https://accounts.google.com/ServiceLogin?continue=/jobs">Sign in</a>
            <a href="https://support.google.co.uk/websearch/careers">Help</a>
            <a href="/url?q=javascript:alert(1)">bad</a>
            <a href="/url?q=/relative/jobs">relative</a>
            <a href="https://www.youtube.com/watch?v=jobs">Video</a>"#;
        let page = Page { url: search().to_string(), html: html.into() };
        let found: Vec<Found> = hits_on(&page, &search(), "q").into_iter().map(|h| h.found).collect();
        assert_eq!(shown(&found), [pair("lever", "jobs.lever.co/acme"), pair("career_page", "https://careers.zeta.com/jobs")]);
    }

    #[test]
    fn aggregators_social_sites_private_hosts_and_plain_pages_are_not_sources() {
        let html = r#"
            <a href="https://www.linkedin.com/jobs/view/123">LinkedIn</a>
            <a href="https://apply.workable.com/acme/j/123/">Workable</a>
            <a href="https://de.indeed.com/jobs?q=rust">Indeed</a>
            <a href="https://www.reddit.com/r/rust/comments/jobs">Reddit</a>
            <a href="https://en.wikipedia.org/wiki/Job">Wiki</a>
            <a href="http://intranet/jobs">Intranet</a>
            <a href="http://192.168.1.10/careers">LAN</a>
            <a href="http://hr.local/careers">mDNS</a>
            <a href="https://blog.acme.com/why-we-love-rust">Blog</a>
            <a href="https://www.greenhouse.io/careers">Greenhouse's own careers</a>
            <a href="https://jobs.com/">a bare jobs.com is not a careers subdomain</a>"#;
        let page = Page { url: search().to_string(), html: html.into() };
        assert_eq!(hits_on(&page, &search(), "q"), []);
    }

    #[test]
    fn a_test_search_engine_is_its_own_host_too() {
        let engine = Url::parse("https://search.example/find?q=x").unwrap();
        let html = r#"<a href="/url?q=https://jobs.ashbyhq.com/acme">A</a> <a href="https://search.example/jobs/more">More</a>"#;
        let page = Page { url: engine.to_string(), html: html.into() };
        let found: Vec<Found> = hits_on(&page, &engine, "q").into_iter().map(|h| h.found).collect();
        assert_eq!(shown(&found), [pair("ashby", "jobs.ashbyhq.com/acme")]);
    }

    #[test]
    fn known_sources_stored_suggestions_and_excluded_companies_are_left_out() {
        let store = Store::open_in_memory().unwrap();
        // The user's Lever source, typed as a bare name, and a careers page on www.bigco.com.
        store.add_source(&NewSource { kind: "lever".into(), url: "Known".into(), ..Default::default() }).unwrap();
        store.add_source(&NewSource { kind: "career_page".into(), url: "bigco.com/jobs".into(), ..Default::default() }).unwrap();
        let dismissed = store
            .upsert_suggestion(&NewSuggestion { kind: "greenhouse".into(), input: "job-boards.greenhouse.io/dismissedco".into(), ..Default::default() })
            .unwrap()
            .unwrap();
        store.set_suggestion_state(dismissed.id, SuggestionState::Dismissed).unwrap();
        store.upsert_suggestion(&NewSuggestion { kind: "career_page".into(), input: "https://epsilon.io/careers".into(), ..Default::default() }).unwrap();
        let prefs = Preferences { exclude_companies: vec!["MegaCorp".into(), "Delta".into()], ..Preferences::default() };

        let found = new_sources(&page(), &search(), "q", &Known::load(&store).unwrap(), &prefs);
        assert_eq!(
            shown(&found),
            [
                pair("greenhouse", "boards.greenhouse.io/acme"),
                pair("career_page", "https://careers.gamma-robotics.com/"),
                pair("ashby", "jobs.ashbyhq.com/beta"),
            ]
        );
    }

    #[test]
    fn keys_compare_boards_by_name_and_pages_by_host() {
        assert_eq!(key_of("greenhouse", "Acme"), key_of("greenhouse", "https://job-boards.greenhouse.io/acme/jobs/1"));
        assert_eq!(key_of("recruitee", "acme"), key_of("recruitee", "https://ACME.recruitee.com/o/x"));
        assert_eq!(key_of("career_page", "acme.com/careers"), key_of("career_page", "https://www.acme.com/jobs?x=1"));
        assert_ne!(key_of("career_page", "acme.com"), key_of("career_page", "careers.acme.com"));
        assert_ne!(key_of("lever", "acme"), key_of("ashby", "acme"));
        assert_eq!(key_of("workday", "https://acme.wd5.myworkdayjobs.com/en-US/External/job/1").1, "https://acme.wd5.myworkdayjobs.com/external");
        assert_eq!(key_of("hn_hiring", "").0, "hn_hiring");
    }
}
