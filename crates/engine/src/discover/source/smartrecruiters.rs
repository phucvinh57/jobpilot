//! SmartRecruiters: the public postings API (`api.smartrecruiters.com/v1/companies/<id>/postings`).
//!
//! Big companies list thousands of postings, so the listing is read one API page (`limit=100`) at
//! a time, each cached. The cursor is `<API offset>:<index into the role-filtered API page>`, so the
//! same cursor gives the same page. The listing has no description: a posting's text is one
//! request for that posting.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use super::{html_to_text, last_segment, title_fits_roles, Cached, Candidate, Http, JobSource, SearchCtx, SearchPage, LISTING_TTL};
use crate::prefs::Preferences;

pub(super) const DEFAULT_BASE: &str = "https://api.smartrecruiters.com";

/// Where a posting is shown to people: `<BOARD>/<company id>/<posting id>`.
const BOARD: &str = "https://jobs.smartrecruiters.com";

/// Postings per API page (the API's own limit).
const API_PAGE: usize = 100;

/// API pages one search reads at most while the roles filter out everything on them; the cursor
/// then points at the next API page, and the runner's page limits decide whether to go on.
const API_PAGES_PER_SEARCH: usize = 3;

/// The sections of a job ad, in reading order.
const SECTIONS: [&str; 4] = ["companyDescription", "jobDescription", "qualifications", "additionalInformation"];

pub struct SmartRecruiters {
    http: Arc<dyn Http>,
    base: String,
    company_id: String,
    company: String,
    /// One cached listing per API offset.
    pages: Mutex<HashMap<usize, Arc<Cached<ApiPage>>>>,
}

/// One API page of the listing.
struct ApiPage {
    candidates: Vec<Candidate>,
    /// Another API page follows.
    more: bool,
}

impl SmartRecruiters {
    /// `base`: the API origin (`https://api.smartrecruiters.com`, or a test server). `company_id`
    /// is checked by `board_slug`; `company` names postings that do not carry a company name.
    pub fn new(http: Arc<dyn Http>, base: &str, company_id: &str, company: &str) -> Self {
        Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            company_id: company_id.to_string(),
            company: company.to_string(),
            pages: Mutex::default(),
        }
    }

    async fn api_page(&self, offset: usize) -> Result<Arc<ApiPage>> {
        let slot = {
            let mut pages = self.pages.lock().unwrap_or_else(|e| e.into_inner());
            pages.entry(offset).or_insert_with(|| Arc::new(Cached::new(LISTING_TTL))).clone()
        };
        slot.get_or_load(|| async {
            let url = format!("{}/v1/companies/{}/postings?limit={API_PAGE}&offset={offset}", self.base, self.company_id);
            let body = self.http.get_json(&url).await.context("reading the SmartRecruiters postings")?;
            let content = body.get("content").and_then(Value::as_array).context("the SmartRecruiters answer has no posting list")?;
            let more = match body.get("totalFound").and_then(Value::as_u64) {
                Some(total) => !content.is_empty() && offset.saturating_add(API_PAGE) < usize::try_from(total).unwrap_or(usize::MAX),
                None => content.len() >= API_PAGE,
            };
            let candidates: Vec<Candidate> = content.iter().filter_map(|p| self.candidate(p)).collect();
            tracing::info!(offset, postings = content.len(), kept = candidates.len(), "SmartRecruiters page read");
            Ok(ApiPage { candidates, more })
        })
        .await
    }

    fn candidate(&self, p: &Value) -> Option<Candidate> {
        let id = text(p.get("id"));
        let title = text(p.get("name"));
        if !valid_id(&id) || title.is_empty() {
            return None;
        }
        let company = Some(text(p.pointer("/company/name"))).filter(|c| !c.is_empty()).unwrap_or_else(|| self.company.clone());
        let snippet = [text(p.pointer("/experienceLevel/label")), text(p.pointer("/typeOfEmployment/label"))]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", ");
        Some(Candidate {
            url: format!("{BOARD}/{}/{id}", self.company_id),
            title,
            company,
            location: location_of(p.get("location")),
            snippet,
        })
    }
}

#[async_trait]
impl JobSource for SmartRecruiters {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let (mut offset, mut skip) = parse_cursor(ctx.cursor.as_deref())?;
        let want = ctx.page_size.max(1);
        let mut candidates = Vec::new();
        for _ in 0..API_PAGES_PER_SEARCH {
            let page = self.api_page(offset).await?;
            let fitting: Vec<&Candidate> = page.candidates.iter().filter(|c| title_fits_roles(&c.title, &prefs.roles)).collect();
            let taken: Vec<Candidate> = fitting.iter().skip(skip).take(want - candidates.len()).map(|c| (*c).clone()).collect();
            let used = skip + taken.len();
            candidates.extend(taken);
            if used < fitting.len() {
                return Ok(SearchPage { candidates, next: Some(format!("{offset}:{used}")) });
            }
            if !page.more {
                return Ok(SearchPage { candidates, next: None });
            }
            offset = offset.saturating_add(API_PAGE);
            skip = 0;
            if candidates.len() >= want {
                break;
            }
        }
        Ok(SearchPage { candidates, next: Some(format!("{offset}:0")) })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let Some(id) = posting_id(&candidate.url) else { return Ok(None) };
        let url = format!("{}/v1/companies/{}/postings/{id}", self.base, self.company_id);
        let job = self.http.get_json(&url).await.context("reading the SmartRecruiters posting")?;
        let sections = job.pointer("/jobAd/sections").and_then(Value::as_object).context("the SmartRecruiters posting has no job ad")?;
        let mut body = String::new();
        for key in SECTIONS {
            let Some(section) = sections.get(key) else { continue };
            let content = html_to_text(&text(section.get("text")));
            if content.is_empty() {
                continue;
            }
            let heading = text(section.get("title"));
            if !heading.is_empty() {
                body.push_str(&format!("## {heading}\n\n"));
            }
            body.push_str(&format!("{content}\n\n"));
        }
        if body.is_empty() {
            return Ok(None);
        }
        let title = Some(text(job.get("name"))).filter(|t| !t.is_empty()).unwrap_or_else(|| candidate.title.clone());
        let location = Some(location_of(job.get("location"))).filter(|l| !l.is_empty()).unwrap_or_else(|| candidate.location.clone());
        let mut head = format!("# {title}\n\nCompany: {}\nLocation: {location}\n", candidate.company);
        for (label, pointer) in [("Employment", "/typeOfEmployment/label"), ("Experience", "/experienceLevel/label")] {
            let value = text(job.pointer(pointer));
            if !value.is_empty() {
                head.push_str(&format!("{label}: {value}\n"));
            }
        }
        Ok(Some(format!("{head}\n{}", body.trim_end())))
    }
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

/// A posting id: letters and digits only (it goes into the API path).
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// `fullLocation` (or city, region, country), with "(Remote)" or "(Hybrid)" when the posting says so.
fn location_of(loc: Option<&Value>) -> String {
    let Some(loc) = loc else { return String::new() };
    let mut location = text(loc.get("fullLocation"));
    if location.is_empty() {
        location = ["city", "region", "country"].iter().map(|k| text(loc.get(*k))).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
    }
    let flag = |k: &str| loc.get(k).and_then(Value::as_bool) == Some(true);
    let mode = if flag("remote") {
        "Remote"
    } else if flag("hybrid") {
        "Hybrid"
    } else {
        return location;
    };
    if location.is_empty() {
        mode.to_string()
    } else if location.to_lowercase().contains(&mode.to_lowercase()) {
        location
    } else {
        format!("{location} ({mode})")
    }
}

/// `"<offset>:<index>"`, or the start for no cursor.
fn parse_cursor(cursor: Option<&str>) -> Result<(usize, usize)> {
    let Some(c) = cursor else { return Ok((0, 0)) };
    let parsed = c.split_once(':').and_then(|(o, i)| Some((o.parse::<usize>().ok()?, i.parse::<usize>().ok()?)));
    match parsed {
        Some(position) => Ok(position),
        None => bail!("the search position `{c}` is not valid"),
    }
}

/// The posting id in a candidate's URL: `.../<id>` or `.../<id>-<title slug>`.
fn posting_id(url: &str) -> Option<String> {
    let segment = last_segment(url)?;
    let id = segment.split('-').next()?;
    valid_id(id).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::FakeHttp;
    use crate::health::{classify, Problem};

    const LIST: &str = include_str!("../fixtures/smartrecruiters_postings.json");
    const JOB: &str = include_str!("../fixtures/smartrecruiters_posting.json");
    const LIST_URL: &str = "https://sr.test/v1/companies/AcmeGroup/postings?limit=100&offset=0";
    const NEXT_URL: &str = "https://sr.test/v1/companies/AcmeGroup/postings?limit=100&offset=100";
    const JOB_URL: &str = "https://sr.test/v1/companies/AcmeGroup/postings/744000100000001";
    const NEXT: &str = r#"{"offset":100,"limit":100,"totalFound":150,"content":[
        {"id":"744000100000101","name":"Platform Engineer","location":{"fullLocation":"Berlin, Germany"}},
        {"id":"744000100000102","name":"Office Manager","location":{"fullLocation":"Berlin, Germany"}}]}"#;

    fn adapter(http: Arc<FakeHttp>) -> SmartRecruiters {
        SmartRecruiters::new(http, "https://sr.test/", "AcmeGroup", "AcmeGroup (source name)")
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    fn engineers() -> Preferences {
        Preferences { roles: vec!["Backend Engineer".into()], ..Default::default() }
    }

    #[tokio::test]
    async fn lists_postings_with_urls_companies_and_locations() {
        let http = FakeHttp::with([(LIST_URL, LIST)]);
        let page = adapter(http.clone()).search(&Preferences::default(), &ctx(None, 4)).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Senior Backend Engineer (Rust)", "Account Executive, DACH", "Site Reliability Engineer", "Data Engineer"], "no title: dropped");
        let first = &page.candidates[0];
        assert_eq!(first.url, "https://jobs.smartrecruiters.com/AcmeGroup/744000100000001");
        assert_eq!(first.company, "Acme Group");
        assert_eq!(first.location, "Berlin, Berlin, Germany (Hybrid)");
        assert_eq!(first.snippet, "Mid-Senior Level, Full-time");
        assert_eq!(page.candidates[2].company, "AcmeGroup (source name)", "no company name: the source's name");
        assert_eq!(page.candidates[2].location, "Lisbon, pt (Remote)", "no full location: its parts");
        assert_eq!(page.candidates[3].location, "Ho Chi Minh City, Vietnam");
        assert_eq!(page.next.as_deref(), Some("100:0"), "the API has 150: the next API page");
        assert_eq!(http.requested(), [LIST_URL]);
    }

    #[tokio::test]
    async fn roles_thin_the_list_and_pages_follow_the_cursor_across_api_pages() {
        let http = FakeHttp::with([(LIST_URL, LIST), (NEXT_URL, NEXT)]);
        let sr = adapter(http.clone());
        let first = sr.search(&engineers(), &ctx(None, 2)).await.unwrap();
        let titles: Vec<_> = first.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Senior Backend Engineer (Rust)", "Site Reliability Engineer"], "no sales in an engineering search");
        assert_eq!(first.next.as_deref(), Some("0:2"));

        let again = sr.search(&engineers(), &ctx(Some("0:2"), 2)).await.unwrap();
        let second = sr.search(&engineers(), &ctx(Some("0:2"), 2)).await.unwrap();
        assert_eq!(again, second, "the same cursor gives the same page");
        let titles: Vec<_> = second.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Data Engineer", "Platform Engineer"], "the rest of the first API page, then the second");
        assert_eq!(second.next, None, "150 found: no third API page");
        assert_eq!(http.requested(), [LIST_URL, NEXT_URL], "each API page is read once");
    }

    #[tokio::test]
    async fn a_search_reads_a_few_api_pages_at_most() {
        // Five API pages (500 found), none with a fitting title.
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let pages: Vec<(&'static str, &'static str)> = (0..5)
            .map(|i| {
                let url = leak(format!("https://sr.test/v1/companies/AcmeGroup/postings?limit=100&offset={}", i * 100));
                let body = leak(format!(r#"{{"offset":{},"limit":100,"totalFound":500,"content":[{{"id":"9{i}","name":"Office Manager"}}]}}"#, i * 100));
                (url, body)
            })
            .collect();
        let http = FakeHttp::with(pages.clone());
        let sr = adapter(http.clone());
        let prefs = Preferences { roles: vec!["Pastry Chef".into()], ..Default::default() };
        let page = sr.search(&prefs, &ctx(None, 5)).await.unwrap();
        assert!(page.candidates.is_empty());
        assert_eq!(page.next.as_deref(), Some("300:0"), "three API pages read, the next one is where the cursor points");
        assert_eq!(http.requested(), pages[..3].iter().map(|(u, _)| u.to_string()).collect::<Vec<_>>());
        let rest = sr.search(&prefs, &ctx(page.next.as_deref(), 5)).await.unwrap();
        assert_eq!((rest.candidates.len(), rest.next), (0, None), "the last two pages, then the end");
        assert_eq!(http.requested().len(), 5);

        let two = FakeHttp::with([(LIST_URL, LIST), (NEXT_URL, NEXT)]);
        let page = adapter(two).search(&prefs, &ctx(None, 5)).await.unwrap();
        assert_eq!((page.candidates.len(), page.next), (0, None), "two API pages, both read, nothing fits");
        let far = format!("{}:0", usize::MAX);
        assert!(adapter(FakeHttp::with([])).search(&prefs, &ctx(Some(&far), 5)).await.is_err(), "no overflow on a huge offset");
        assert!(adapter(FakeHttp::with([])).search(&prefs, &ctx(Some("x"), 5)).await.unwrap_err().to_string().contains("not valid"));
        assert!(adapter(FakeHttp::with([])).search(&prefs, &ctx(Some("1:a"), 5)).await.is_err());
    }

    #[tokio::test]
    async fn a_posting_is_read_from_its_detail_endpoint() {
        let http = FakeHttp::with([(LIST_URL, LIST), (JOB_URL, JOB)]);
        let sr = adapter(http.clone());
        let page = sr.search(&Preferences::default(), &ctx(None, 1)).await.unwrap();
        let text = sr.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.starts_with("# Senior Backend Engineer (Rust)\n\nCompany: Acme Group\nLocation: Berlin, Berlin, Germany (Hybrid)"), "{text}");
        assert!(text.contains("Employment: Full-time") && text.contains("Experience: Mid-Senior Level"), "{text}");
        assert!(text.contains("## Job Description") && text.contains("payment services in Rust") && text.contains("PostgreSQL"), "{text}");
        assert!(text.contains("anvils & rockets") && !text.contains("<p>"), "markup is turned into text: {text}");
        assert!(!text.contains("Additional Information"), "an empty section is left out: {text}");
        assert_eq!(http.requested().last().map(String::as_str), Some(JOB_URL));
        // A posting URL with the title slug after the id works too; one without an id is not read.
        let slugged = Candidate { url: "https://jobs.smartrecruiters.com/AcmeGroup/744000100000001-senior-backend-engineer".into(), ..Default::default() };
        assert!(sr.posting(&slugged).await.unwrap().is_some());
        let odd = Candidate { url: "https://jobs.smartrecruiters.com/AcmeGroup/..".into(), ..Default::default() };
        assert_eq!(sr.posting(&odd).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unknown_shape_is_an_error_naming_the_source() {
        let http = FakeHttp::with([(LIST_URL, r#"{"message": "nope"}"#), (JOB_URL, r#"{"id": "1"}"#)]);
        let sr = adapter(http);
        let err = format!("{:#}", sr.search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
        assert!(err.contains("SmartRecruiters answer has no posting list"), "{err}");
        let job = Candidate { url: "https://jobs.smartrecruiters.com/AcmeGroup/744000100000001".into(), ..Default::default() };
        let err = format!("{:#}", sr.posting(&job).await.unwrap_err());
        assert!(err.contains("SmartRecruiters posting has no job ad"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_request_is_a_network_error() {
        let http = FakeHttp::with_statuses([], [(LIST_URL, 503)]);
        let err = format!("{:#}", adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
        assert!(err.contains("reading the SmartRecruiters postings") && err.contains("HTTP 503"), "{err}");
        assert_eq!(classify(&err), Problem::Network);
    }
}
