//! Recruitee: the public offers API of a company's board (`<company>.recruitee.com/api/offers/`).
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
use crate::net::http::public_http_url;
use crate::prefs::Preferences;

/// The board hosts: `<company>.recruitee.com`.
pub(super) const HOSTS: &[&str] = &["recruitee.com"];

pub struct Recruitee {
    http: Arc<dyn Http>,
    origin: String,
    company: String,
    listing: Cached<Vec<Posting>>,
}

struct Posting {
    slug: String,
    candidate: Candidate,
    /// `None` when the offer has no description: the runner loads its page instead.
    text: Option<String>,
}

impl Recruitee {
    /// `origin`: the company's board (`https://acme.recruitee.com`), built by code from a name
    /// `board_host` checked. `company` names postings that do not carry a company name.
    pub fn new(http: Arc<dyn Http>, origin: &str, company: &str) -> Self {
        Self { http, origin: origin.trim_end_matches('/').to_string(), company: company.to_string(), listing: Cached::new(LISTING_TTL) }
    }

    async fn postings(&self) -> Result<Arc<Vec<Posting>>> {
        self.listing
            .get_or_load(|| async {
                let url = format!("{}/api/offers/", self.origin);
                let body = self.http.get_json(&url).await.context("reading the Recruitee offers")?;
                let offers = body.get("offers").and_then(Value::as_array).context("the Recruitee answer has no offer list")?;
                let postings: Vec<Posting> = offers.iter().filter_map(|o| self.posting_of(o)).collect();
                tracing::info!(offers = offers.len(), kept = postings.len(), "Recruitee offers read");
                Ok(postings)
            })
            .await
    }

    fn posting_of(&self, o: &Value) -> Option<Posting> {
        let slug = text(o.get("slug"));
        let title = text(o.get("title"));
        if !valid_slug(&slug) || title.is_empty() {
            return None;
        }
        let url = posting_url(&text(o.get("careers_url")), &slug).unwrap_or_else(|| format!("{}/o/{slug}", self.origin));
        let company = Some(text(o.get("company_name"))).filter(|c| !c.is_empty()).unwrap_or_else(|| self.company.clone());
        let location = location_of(o);
        let description = html_to_text(&text(o.get("description")));
        let requirements = html_to_text(&text(o.get("requirements")));
        let mut full = format!("# {title}\n\nCompany: {company}\nLocation: {location}\n");
        let department = text(o.get("department"));
        if !department.is_empty() {
            full.push_str(&format!("Department: {department}\n"));
        }
        if let Some(salary) = salary_of(o.get("salary")) {
            full.push_str(&format!("Salary: {salary}\n"));
        }
        full.push_str(&format!("\n{description}"));
        if !requirements.is_empty() {
            full.push_str(&format!("\n\n## Requirements\n\n{requirements}"));
        }
        let snippet = one_line(&description, SNIPPET_CHARS);
        let text = (!description.is_empty() || !requirements.is_empty()).then(|| full.trim_end().to_string());
        Some(Posting { slug, candidate: Candidate { url, title, company, location, snippet }, text })
    }
}

#[async_trait]
impl JobSource for Recruitee {
    async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        let fitting: Vec<Candidate> =
            self.postings().await?.iter().filter(|p| title_fits_roles(&p.candidate.title, &prefs.roles)).map(|p| p.candidate.clone()).collect();
        let (candidates, next) = page_of(&fitting, ctx)?;
        Ok(SearchPage { candidates, next })
    }

    async fn posting(&self, candidate: &Candidate) -> Result<Option<String>> {
        let wanted = last_segment(&candidate.url);
        let postings = self.postings().await?;
        Ok(postings.iter().find(|p| p.candidate.url == candidate.url || Some(&p.slug) == wanted.as_ref()).and_then(|p| p.text.clone()))
    }
}

fn text(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).map(str::trim).unwrap_or_default().to_string()
}

fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && slug.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
}

/// The offer's own `careers_url` when it is an https address of this offer (its path ends in
/// `/o/<slug>`) on a public host name; it may be on the company's own domain. An IP address, this
/// machine or a private network name, or anything else is not used.
fn posting_url(careers_url: &str, slug: &str) -> Option<String> {
    let url = public_http_url(careers_url, false).ok()?;
    let fits = url.scheme() == "https"
        && matches!(url.host(), Some(url::Host::Domain(_)))
        && url.path().trim_end_matches('/').ends_with(&format!("/o/{slug}"));
    fits.then(|| url.to_string())
}

/// `location` (or city, country), with "(Remote)" or "(Hybrid)" when the offer says so.
fn location_of(o: &Value) -> String {
    let mut location = text(o.get("location"));
    if location.is_empty() {
        location = ["city", "country"].iter().map(|k| text(o.get(*k))).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", ");
    }
    let flag = |k: &str| o.get(k).and_then(Value::as_bool) == Some(true);
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

/// `60000-80000 EUR per year`, when the offer publishes a range.
fn salary_of(salary: Option<&Value>) -> Option<String> {
    let s = salary?;
    let field = |k: &str| match s.get(k) {
        Some(Value::String(v)) => v.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    let range = match (field("min"), field("max")) {
        (min, max) if min.is_empty() && max.is_empty() => return None,
        (min, max) if max.is_empty() || min == max => min,
        (min, max) if min.is_empty() => max,
        (min, max) => format!("{min}-{max}"),
    };
    let mut line = range;
    for (k, prefix) in [("currency", " "), ("period", " per ")] {
        let v = field(k);
        if !v.is_empty() {
            line.push_str(&format!("{prefix}{v}"));
        }
    }
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::source::FakeHttp;
    use crate::health::{classify, Problem};

    const LIST: &str = include_str!("../fixtures/recruitee_offers.json");
    const URL: &str = "https://acme.recruitee.com/api/offers/";

    fn adapter(http: Arc<FakeHttp>) -> Recruitee {
        Recruitee::new(http, "https://acme.recruitee.com/", "acme (source name)")
    }

    fn ctx(cursor: Option<&str>, page_size: usize) -> SearchCtx {
        SearchCtx { cursor: cursor.map(str::to_string), page_size }
    }

    #[tokio::test]
    async fn lists_offers_with_urls_companies_and_locations() {
        let http = FakeHttp::with([(URL, LIST)]);
        let page = adapter(http.clone()).search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        let titles: Vec<_> = page.candidates.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Backend Engineer", "Account Manager", "Platform Engineer", "Data Analyst"], "no slug: dropped");
        let [backend, sales, platform, _] = &page.candidates[..] else { panic!("four offers") };
        assert_eq!(backend.url, "https://careers.acme.test/o/backend-engineer", "the offer's own careers page");
        assert_eq!((backend.company.as_str(), backend.location.as_str()), ("Acme", "Amsterdam, Noord-Holland, Netherlands (Hybrid)"));
        assert!(backend.snippet.starts_with("At Acme we make anvils easy to order."), "{}", backend.snippet);
        assert_eq!(sales.url, "https://acme.recruitee.com/o/account-manager", "a careers_url of another offer is not used");
        assert_eq!((sales.company.as_str(), sales.location.as_str()), ("acme (source name)", "Utrecht, Netherlands"));
        assert_eq!(platform.url, "https://acme.recruitee.com/o/platform-engineer", "only http(s)");
        assert_eq!(platform.location, "Remote");
        assert_eq!(page.next, None);
        assert_eq!(http.requested(), [URL]);
    }

    #[tokio::test]
    async fn roles_thin_the_list_and_pages_follow_the_cursor() {
        let http = FakeHttp::with([(URL, LIST)]);
        let rc = adapter(http.clone());
        let prefs = Preferences { roles: vec!["Software Engineer".into()], ..Default::default() };
        let first = rc.search(&prefs, &ctx(None, 1)).await.unwrap();
        assert_eq!(first.candidates[0].title, "Backend Engineer");
        assert_eq!(first.next.as_deref(), Some("1"));
        let second = rc.search(&prefs, &ctx(Some("1"), 1)).await.unwrap();
        assert_eq!(second, rc.search(&prefs, &ctx(Some("1"), 1)).await.unwrap(), "the same cursor gives the same page");
        assert_eq!(second.candidates[0].title, "Platform Engineer", "no sales in an engineering search");
        assert_eq!(second.next, None);
        assert_eq!(http.requested().len(), 1, "one listing request serves every page");
    }

    #[tokio::test]
    async fn postings_are_read_from_the_cached_listing() {
        let http = FakeHttp::with([(URL, LIST)]);
        let rc = adapter(http.clone());
        let page = rc.search(&Preferences::default(), &ctx(None, 10)).await.unwrap();
        let text = rc.posting(&page.candidates[0]).await.unwrap().unwrap();
        assert!(text.starts_with("# Backend Engineer\n\nCompany: Acme\nLocation: Amsterdam"), "{text}");
        assert!(text.contains("Department: Technology") && text.contains("Salary: 60000-80000 EUR per year"), "{text}");
        assert!(text.contains("order API in Go") && text.contains("## Requirements") && text.contains("You know PostgreSQL"), "{text}");
        assert!(!text.contains("<p") && !text.contains("<li>"), "markup is turned into text: {text}");
        let sales = rc.posting(&page.candidates[1]).await.unwrap().unwrap();
        assert!(!sales.contains("Salary") && !sales.contains("## Requirements"), "{sales}");
        let by_slug = Candidate { url: "https://acme.recruitee.com/o/platform-engineer".into(), ..Default::default() };
        assert!(rc.posting(&by_slug).await.unwrap().unwrap().contains("Kubernetes"));
        let unknown = Candidate { url: "https://acme.recruitee.com/o/zzz".into(), ..Default::default() };
        let empty = rc.search(&Preferences::default(), &ctx(None, 10)).await.unwrap().candidates.into_iter().find(|c| c.title == "Data Analyst").unwrap();
        assert_eq!(empty.url, "https://acme.recruitee.com/o/data-analyst", "a careers_url on a private host is not used");
        assert_eq!(rc.posting(&empty).await.unwrap(), None, "no description: the runner loads the page");
        assert_eq!(rc.posting(&unknown).await.unwrap(), None);
        assert_eq!(http.requested().len(), 1);
    }

    #[tokio::test]
    async fn an_unknown_shape_is_an_error_naming_the_source() {
        let http = FakeHttp::with([(URL, r#"[{"title": "x"}]"#)]);
        let err = format!("{:#}", adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
        assert!(err.contains("Recruitee answer has no offer list"), "{err}");
    }

    #[tokio::test]
    async fn a_failed_request_is_a_network_error() {
        let http = FakeHttp::with_statuses([], [(URL, 429)]);
        let err = format!("{:#}", adapter(http).search(&Preferences::default(), &ctx(None, 5)).await.unwrap_err());
        assert!(err.contains("reading the Recruitee offers") && err.contains("HTTP 429"), "{err}");
        assert_eq!(classify(&err), Problem::Network);
    }

    #[test]
    fn careers_urls_and_salaries() {
        assert_eq!(posting_url("https://careers.acme.test/o/dev/", "dev").as_deref(), Some("https://careers.acme.test/o/dev/"));
        assert_eq!(posting_url("https://careers.acme.test/o/dev-2", "dev"), None, "another offer");
        assert_eq!(posting_url("javascript:alert(1)//o/dev", "dev"), None);
        assert_eq!(posting_url("", "dev"), None);
        for private in [
            "http://careers.acme.test/o/dev",
            "https://localhost/o/dev",
            "https://jobs.localhost/o/dev",
            "https://intranet/o/dev",
            "https://careers.corp.internal/o/dev",
            "https://127.0.0.1/o/dev",
            "https://10.0.0.5/o/dev",
            "https://8.8.8.8/o/dev",
            "https://[::1]/o/dev",
        ] {
            assert_eq!(posting_url(private, "dev"), None, "{private}");
        }
        let salary = |v: serde_json::Value| salary_of(Some(&v));
        assert_eq!(salary(serde_json::json!({"min": 5000, "max": 5000, "currency": "EUR", "period": "month"})).as_deref(), Some("5000 EUR per month"));
        assert_eq!(salary(serde_json::json!({"min": null, "max": "90000"})).as_deref(), Some("90000"));
        assert_eq!(salary(serde_json::json!({"min": null, "max": null, "currency": "EUR"})), None);
    }
}
