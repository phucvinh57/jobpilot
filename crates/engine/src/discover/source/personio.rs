//! Personio: the public XML job feed of a company's board (`<company>.jobs.personio.de/xml`).
//!
//! The feed carries the descriptions when the company publishes them, so postings are read from it
//! (one request, cached). Some companies leave them empty: then the runner loads the posting's page.

use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use quick_xml::events::Event;
use quick_xml::Reader;

use super::{
    html_to_text, last_segment, one_line, page_of, title_fits_roles, Cached, Candidate, Http, JobSource, SearchCtx, SearchPage,
    LISTING_TTL, SNIPPET_CHARS,
};
use crate::prefs::Preferences;

/// The board hosts: `<company>.jobs.personio.de` (the default) or `.com`.
pub(super) const HOSTS: &[&str] = &["jobs.personio.de", "jobs.personio.com"];

const ACCEPT: &str = "application/xml, text/xml;q=0.9";

/// Deepest element nesting a feed may have (a real one is 5 deep, its descriptions a few more).
const MAX_DEPTH: usize = 64;

pub struct Personio {
    http: Arc<dyn Http>,
    origin: String,
    company: String,
    listing: Cached<Vec<Posting>>,
}

struct Posting {
    id: String,
    candidate: Candidate,
    /// `None` when the feed has no description for it.
    text: Option<String>,
}

/// A `<position>` of the feed.
#[derive(Default)]
struct Position {
    id: String,
    name: String,
    company: String,
    office: String,
    offices: Vec<String>,
    department: String,
    employment_type: String,
    seniority: String,
    schedule: String,
    /// `(heading, HTML)` of each `<jobDescription>`.
    descriptions: Vec<(String, String)>,
}

impl Position {
    fn set(&mut self, field: &str, value: String) {
        match field {
            "id" => self.id = value,
            "name" => self.name = value,
            "subcompany" => self.company = value,
            "office" => self.office = value,
            "department" => self.department = value,
            "employmentType" => self.employment_type = value,
            "seniority" => self.seniority = value,
            "schedule" => self.schedule = value,
            _ => {}
        }
    }
}

impl Personio {
    /// `origin`: the company's board (`https://acme.jobs.personio.de`), built by code from a name
    /// `board_host` checked. `company` names the postings when the feed does not.
    pub fn new(http: Arc<dyn Http>, origin: &str, company: &str) -> Self {
        Self { http, origin: origin.trim_end_matches('/').to_string(), company: company.to_string(), listing: Cached::new(LISTING_TTL) }
    }

    async fn postings(&self) -> Result<Arc<Vec<Posting>>> {
        self.listing
            .get_or_load(|| async {
                let url = format!("{}/xml", self.origin);
                let xml = self.http.get_text(&url, ACCEPT).await.context("reading the Personio job feed")?;
                let positions = parse_positions(&xml)?;
                let postings: Vec<Posting> = positions.into_iter().filter_map(|p| self.posting_of(p)).collect();
                tracing::info!(postings = postings.len(), "Personio feed read");
                Ok(postings)
            })
            .await
    }

    fn posting_of(&self, p: Position) -> Option<Posting> {
        let valid = !p.id.is_empty() && p.id.chars().all(|c| c.is_ascii_alphanumeric());
        if !valid || p.name.is_empty() {
            return None;
        }
        let company = if p.company.is_empty() { self.company.clone() } else { p.company.clone() };
        let location = std::iter::once(&p.office).chain(&p.offices).filter(|o| !o.is_empty()).cloned().collect::<Vec<_>>().join(", ");
        let kind = [&p.department, &p.seniority, &p.schedule].into_iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(", ");
        let mut body = String::new();
        for (heading, html) in &p.descriptions {
            let content = html_to_text(html);
            if content.is_empty() {
                continue;
            }
            if !heading.is_empty() {
                body.push_str(&format!("## {heading}\n\n"));
            }
            body.push_str(&format!("{content}\n\n"));
        }
        let text = (!body.is_empty()).then(|| {
            let mut head = format!("# {}\n\nCompany: {company}\nLocation: {location}\n", p.name);
            for (label, value) in [("Department", &p.department), ("Employment", &p.employment_type), ("Seniority", &p.seniority), ("Schedule", &p.schedule)] {
                if !value.is_empty() {
                    head.push_str(&format!("{label}: {value}\n"));
                }
            }
            format!("{head}\n{}", body.trim_end())
        });
        let candidate = Candidate { url: format!("{}/job/{}", self.origin, p.id), title: p.name, company, location, snippet: one_line(&kind, SNIPPET_CHARS) };
        Some(Posting { id: p.id, candidate, text })
    }
}

#[async_trait]
impl JobSource for Personio {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.postings().await?.iter().filter(|p| title_fits_roles(&p.candidate.title, &prefs.roles)).map(|p| p.candidate.clone()).collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let wanted = last_segment(&candidate.url);
        let postings = self.postings().await?;
        Ok(postings.iter().find(|p| p.candidate.url == candidate.url || Some(&p.id) == wanted.as_ref()).and_then(|p| p.text.clone()))
    }
}

/// The `<position>`s of a `<workzag-jobs>` feed, in document order. Markup inside a description
/// that is not wrapped in CDATA is read as text.
fn parse_positions(xml: &str) -> Result<Vec<Position>> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut seen_root = false;
    let mut positions = Vec::new();
    let mut current: Option<Position> = None;
    let mut description = (String::new(), String::new());
    // Elements open from a description's `<value>` down (0 outside one): markup inside it is text,
    // so it is counted here rather than kept on `stack`.
    let mut value_depth = 0usize;
    loop {
        let event = reader.read_event().map_err(|e| anyhow!("the Personio answer is not readable XML: {e} (byte {})", reader.buffer_position()))?;
        match event {
            Event::Eof if !stack.is_empty() => bail!("the Personio answer is not readable XML: it ends early"),
            Event::Eof => break,
            Event::Start(e) => {
                if stack.len() + value_depth >= MAX_DEPTH {
                    bail!("the Personio answer is not a job feed: its elements nest too deep");
                }
                if value_depth > 0 {
                    value_depth += 1;
                    text.push(' ');
                    continue;
                }
                let name = e.name().as_ref().to_string();
                match (stack.last().map(String::as_str), name.as_str()) {
                    (None, "workzag-jobs") => seen_root = true,
                    (Some("workzag-jobs"), "position") => current = Some(Position::default()),
                    (Some("jobDescriptions"), "jobDescription") => description = Default::default(),
                    _ => {}
                }
                text.clear();
                if name == "value" {
                    value_depth = 1;
                }
                stack.push(name);
            }
            Event::End(_) => {
                if value_depth > 1 {
                    value_depth -= 1;
                    text.push(' ');
                    continue;
                }
                value_depth = 0;
                let name = stack.pop().unwrap_or_default();
                let value = std::mem::take(&mut text).trim().to_string();
                let parent = stack.last().map(String::as_str).unwrap_or_default();
                if (parent, name.as_str()) == ("workzag-jobs", "position") {
                    positions.extend(current.take());
                    continue;
                }
                match (parent, name.as_str(), current.as_mut()) {
                    ("position", field, Some(p)) => p.set(field, value),
                    ("additionalOffices", "office", Some(p)) if !value.is_empty() => p.offices.push(value),
                    ("jobDescription", "name", _) => description.0 = value,
                    ("jobDescription", "value", _) => description.1 = value,
                    ("jobDescriptions", "jobDescription", Some(p)) => p.descriptions.push(std::mem::take(&mut description)),
                    _ => {}
                }
            }
            Event::Text(t) => {
                text.push_str(&t.xml10_content());
            }
            Event::CData(c) => {
                text.push_str(&c.xml10_content());
            }
            Event::GeneralRef(r) => {
                if let Ok(Some(ch)) = r.resolve_char_ref() {
                    text.push(ch);
                } else {
                    let name = r.xml10_content();
                    text.push_str(match name.as_ref() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => " ",
                    });
                }
            }
            _ => {}
        }
    }
    if !seen_root {
        bail!("the Personio answer is not a job feed");
    }
    Ok(positions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::FakeHttp;
    use crate::health::{classify, Problem};

    const FEED: &str = include_str!("../fixtures/personio.xml");
    const URL: &str = "https://acme.jobs.personio.de/xml";

    fn adapter(http: Arc<FakeHttp>) -> Personio {
        Personio::new(http, "https://acme.jobs.personio.de/", "acme (source name)")
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    #[tokio::test]
    async fn lists_positions_with_urls_companies_and_offices() {
        let http = FakeHttp::with([(URL, FEED)]);
        let page = adapter(http.clone()).search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Senior Backend Engineer (d/f/m)", "Business Development Representative (d/f/m)", "Data Engineer"], "a bad id: dropped");
        let [backend, sales, data] = &page.candidates[..] else { panic!("three positions") };
        assert_eq!(backend.url, "https://acme.jobs.personio.de/job/2885001");
        assert_eq!((backend.company.as_str(), backend.location.as_str()), ("Acme GmbH", "Berlin, Munich"));
        assert_eq!(backend.snippet, "Product & Tech, experienced, full-time");
        assert_eq!((sales.company.as_str(), sales.location.as_str()), ("acme (source name)", "Berlin"));
        assert_eq!(data.url, "https://acme.jobs.personio.de/job/2885003");
        assert_eq!(page.next, None);
        assert_eq!(http.requested(), [URL]);
    }

    #[tokio::test]
    async fn roles_thin_the_list_and_pages_follow_the_cursor() {
        let http = FakeHttp::with([(URL, FEED)]);
        let pe = adapter(http.clone());
        let prefs = Preferences { roles: vec!["Backend Engineer".into()], ..Default::default() };
        let first = pe.search(&prefs, &ctx(None, 1)).await.unwrap();
        assert_eq!(first.candidates[0].title, "Senior Backend Engineer (d/f/m)");
        assert_eq!(first.next.as_deref(), Some("1"));
        let second = pe.search(&prefs, &ctx(Some("1"), 1)).await.unwrap();
        assert_eq!(second, pe.search(&prefs, &ctx(Some("1"), 1)).await.unwrap(), "the same cursor gives the same page");
        assert_eq!(second.candidates[0].title, "Data Engineer", "no sales in an engineering search");
        assert_eq!(second.next, None);
        assert_eq!(http.requested().len(), 1, "one feed request serves every page");
    }

    #[tokio::test]
    async fn a_posting_has_the_feed_text_or_none_when_the_feed_has_no_description() {
        let http = FakeHttp::with([(URL, FEED)]);
        let pe = adapter(http.clone());
        let page = pe.search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        let text = pe.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.starts_with("# Senior Backend Engineer (d/f/m)\n\nCompany: Acme GmbH\nLocation: Berlin, Munich\n"), "{text}");
        assert!(text.contains("Department: Product & Tech") && text.contains("Employment: permanent"), "{text}");
        assert!(text.contains("## Your mission") && text.contains("ordering platform in") && text.contains("## Your profile"), "{text}");
        assert!(text.contains("PostgreSQL & Kafka") && !text.contains("<li>"), "markup is turned into text: {text}");
        assert_eq!(pe.posting(&page.candidates[1]).await.unwrap(), None, "empty descriptions: the runner loads the page");
        assert_eq!(pe.posting(&page.candidates[2]).await.unwrap(), None, "an empty element too");
        let other = Candidate { url: "https://acme.jobs.personio.de/job/999".into(), ..Default::default() };
        assert_eq!(pe.posting(&other).await.unwrap(), None);
        assert_eq!(http.requested().len(), 1);
    }

    #[test]
    fn markup_outside_cdata_is_read_as_text() {
        let xml = "<workzag-jobs><position><id>1</id><name>Dev</name><jobDescriptions><jobDescription><name>Role</name>\
                   <value><p>Write <b>Rust</b> &amp; Go</p></value></jobDescription></jobDescriptions></position></workzag-jobs>";
        let positions = parse_positions(xml).unwrap();
        let (heading, html) = &positions[0].descriptions[0];
        assert_eq!(heading, "Role");
        assert!(html.contains("Rust") && html.contains("& Go"), "{html}");
    }

    #[test]
    fn deep_nesting_is_refused_quickly() {
        let started = std::time::Instant::now();
        for inner in ["<value>", "<x>"] {
            let deep = format!("<workzag-jobs><position><jobDescriptions><jobDescription>{}", inner.repeat(1_000_000));
            let err = format!("{:#}", parse_positions(&deep).err().unwrap());
            assert!(err.contains("Personio answer is not a job feed") && err.contains("too deep"), "{err}");
        }
        let ok = format!("<workzag-jobs><position><id>1</id><name>Dev</name><jobDescriptions><jobDescription><name>R</name><value>{}a{}</value></jobDescription></jobDescriptions></position></workzag-jobs>", "<b>".repeat(50), "</b>".repeat(50));
        assert!(parse_positions(&ok).unwrap()[0].descriptions[0].1.contains('a'), "nesting below the limit is read");
        assert!(started.elapsed() < std::time::Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[tokio::test]
    async fn an_unknown_shape_is_an_error_naming_the_source() {
        for body in [r#"{"jobs": []}"#, "<html><body>Jobs</body></html>", "<workzag-jobs><position>"] {
            let http = FakeHttp::with([(URL, body)]);
            let err = format!("{:#}", adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
            assert!(err.contains("Personio answer is not"), "{body}: {err}");
        }
        let empty = FakeHttp::with([(URL, "<?xml version=\"1.0\"?><workzag-jobs></workzag-jobs>")]);
        assert!(adapter(empty).search(&Preferences::default(), &ctx(None, 5)).await.unwrap().candidates.is_empty(), "no openings is not an error");
    }

    #[tokio::test]
    async fn a_failed_request_is_a_network_error() {
        let http = FakeHttp::with_statuses([], [(URL, 502)]);
        let err = format!("{:#}", adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
        assert!(err.contains("reading the Personio job feed") && err.contains("HTTP 502"), "{err}");
        assert_eq!(classify(&err), Problem::Network);
    }
}
