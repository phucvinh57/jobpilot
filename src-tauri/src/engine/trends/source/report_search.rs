//! Annual and quarterly reports found through a search API (decision 6 of the plans: an API key
//! instead of driving a search engine in the browser, which triggers captchas).
//!
//! Enabled only while a key is saved in the OS keyring (see [`SearchProvider::secret_name`]). The
//! queries are built here from the market frame (the user's background and place, never their
//! wishes, see [`MarketFrame`]) and the period; the API's results are data: only
//! http(s) addresses on public hosts are read, with GET, and what a page says never leads to
//! another request. PDFs are read through `pdf-extract`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::page::{fetch_text, make_item};
use super::{join, normalize_url, Ctx, Item};
use crate::engine::trends::frame::MarketFrame;
use crate::engine::trends::http::public_http_url;
use crate::engine::trends::period::Period;
use crate::engine::trends::text::truncate;

/// Which search API the saved key belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchProvider {
    Brave,
    Tavily,
}

impl SearchProvider {
    pub const ALL: [SearchProvider; 2] = [SearchProvider::Brave, SearchProvider::Tavily];

    /// The keyring entry (service `jobpilot`) that holds the API key.
    pub fn secret_name(self) -> &'static str {
        match self {
            SearchProvider::Brave => "trends:brave_api_key",
            SearchProvider::Tavily => "trends:tavily_api_key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SearchProvider::Brave => "Brave Search",
            SearchProvider::Tavily => "Tavily",
        }
    }
}

/// One search result.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub title: String,
    pub url: String,
}

/// Locations and role phrases of the frame that get queries.
const MAX_SUBJECTS: usize = 2;

/// Search queries for annual and quarterly reports on the user's market: where they want to work
/// and what their background makes them. Per location "<location> technology job market report
/// <year>" and "<location> <first role> salary survey <year>" ("<location> tech salary survey
/// <year>" without a role); per role phrase of the profile "<role> hiring trends <year> report" and
/// "<role> skills demand <year> <quarter>". At most two locations and two roles. Without either,
/// general technology queries. Wished skills, wished domains and target roles are not in the frame,
/// so they never steer what is searched. Deduplicated (ignoring case), at most `max`, interleaved
/// so that every subject gets its first query before any gets a second.
pub fn build_queries(frame: &MarketFrame, period: &Period, max: usize) -> Vec<String> {
    let (year, quarter) = (period.year(), period.quarter_label());
    let subjects = |list: &[String]| -> Vec<String> { list.iter().map(|s| subject(s)).filter(|s| !s.is_empty()).take(MAX_SUBJECTS).collect() };
    let locations = subjects(&frame.locations);
    let roles = subjects(&frame.roles);

    let mut queries = Vec::new();
    if locations.is_empty() && roles.is_empty() {
        queries.push(format!("software engineering hiring trends report {year}"));
        queries.push(format!("technology industry outlook {year} {quarter} report"));
        queries.push(format!("developer survey results {year}"));
    }
    for round in 0..2 {
        for location in &locations {
            queries.push(match (round, roles.first()) {
                (0, _) => format!("{location} technology job market report {year}"),
                (_, Some(role)) => format!("{location} {role} salary survey {year}"),
                (_, None) => format!("{location} tech salary survey {year}"),
            });
        }
        for role in &roles {
            queries.push(match round {
                0 => format!("{role} hiring trends {year} report"),
                _ => format!("{role} skills demand {year} {quarter}"),
            });
        }
    }
    let mut seen = std::collections::HashSet::new();
    queries.retain(|q| seen.insert(q.to_lowercase()));
    queries.truncate(max);
    queries
}

/// One location or role as a query term: single spaces, no control characters, bounded length.
fn subject(raw: &str) -> String {
    let flat: String = raw.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    truncate(&flat.split_whitespace().collect::<Vec<_>>().join(" "), 60)
}

/// Run `queries` against `provider` with `key` and read the reports the results point at.
///
/// A query that fails or a result that cannot be read is skipped; the adapter fails only when
/// every query did. `key` is used for the request header and nowhere else.
pub async fn collect(ctx: &Ctx, provider: SearchProvider, key: &str, queries: &[String]) -> Result<Vec<Item>> {
    let l = ctx.limits();
    let mut items: Vec<Item> = Vec::new();
    let mut searched = 0;
    let mut first_error = None;
    for query in queries.iter().take(l.max_queries) {
        if items.len() >= l.max_items {
            break;
        }
        let found = match search(ctx, provider, key, query).await {
            Ok(found) => {
                searched += 1;
                found
            }
            Err(e) => {
                tracing::warn!(provider = provider.label(), "a report search failed");
                first_error.get_or_insert(e);
                continue;
            }
        };
        for hit in found.into_iter().take(l.results_per_query) {
            if items.len() >= l.max_items {
                break;
            }
            // Search results are third-party data: public http(s) addresses only.
            let Some(url) = public_http_url(&hit.url, l.allow_private_hosts).ok().and_then(|u| normalize_url(u.as_str()).ok()) else {
                continue;
            };
            if ctx.is_known(&url) || items.iter().any(|i| i.url == url) {
                continue;
            }
            match fetch_text(ctx, &url).await {
                Ok(page) => items.push(make_item(url, Some(&hit.title), None, page.text)),
                Err(_) => tracing::debug!("a search result could not be read and is skipped"),
            }
        }
    }
    match first_error {
        Some(e) if searched == 0 => Err(e).context("the report search failed"),
        _ => Ok(items),
    }
}

/// The API's results for one query (dates are not kept: search APIs rarely have a reliable one).
pub async fn search(ctx: &Ctx, provider: SearchProvider, key: &str, query: &str) -> Result<Vec<Found>> {
    let n = ctx.limits().results_per_query.clamp(1, 20);
    match provider {
        SearchProvider::Brave => {
            let mut url = url::Url::parse(&join(&ctx.endpoints().brave, "res/v1/web/search")).context("the Brave address is not a URL")?;
            url.query_pairs_mut().append_pair("q", query).append_pair("count", &n.to_string());
            let resp = ctx.http.get(url.as_str(), &[("Accept", "application/json"), ("X-Subscription-Token", key)]).await?;
            let body: BraveResponse = resp.json("Brave Search")?;
            Ok(body.web.results.into_iter().map(|r| Found { title: r.title.unwrap_or_default(), url: r.url.unwrap_or_default() }).collect())
        }
        SearchProvider::Tavily => {
            let url = join(&ctx.endpoints().tavily, "search");
            let body = serde_json::json!({
                "query": query,
                "max_results": n,
                "search_depth": "basic",
                "include_answer": false,
                "include_raw_content": false,
            });
            let auth = format!("Bearer {key}");
            let resp = ctx.http.post_json(&url, &[("Accept", "application/json"), ("Authorization", &auth)], &body).await?;
            let body: TavilyResponse = resp.json("Tavily")?;
            Ok(body.results.into_iter().map(|r| Found { title: r.title.unwrap_or_default(), url: r.url.unwrap_or_default() }).collect())
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BraveResponse {
    web: BraveWeb,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BraveWeb {
    results: Vec<BraveResult>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct BraveResult {
    title: Option<String>,
    url: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TavilyResponse {
    results: Vec<TavilyResult>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TavilyResult {
    title: Option<String>,
    url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::trends::testutil::{ctx, typst_pdf, Fixtures};

    use crate::engine::pipeline::cv::{Cv, Experience};
    use crate::engine::prefs::{Preferences, Remote};

    fn w46() -> Period {
        Period::parse("2026-W46").unwrap()
    }

    /// A frame with these locations and role phrases (as the profile would give them).
    fn frame(locations: &[&str], roles: &[&str]) -> MarketFrame {
        MarketFrame {
            locations: locations.iter().map(|s| s.to_string()).collect(),
            roles: roles.iter().map(|s| s.to_string()).collect(),
            ..MarketFrame::default()
        }
    }

    fn cv() -> Cv {
        let mut cv = Cv::default();
        cv.basics.headline = "Backend engineer".into();
        cv.experience = vec![Experience { position: "Software Engineer".into(), company: "Acme".into(), ..Experience::default() }];
        cv
    }

    #[test]
    fn queries_come_from_locations_roles_year_and_quarter() {
        let q = build_queries(&frame(&["Ho Chi Minh City", "Singapore"], &["Software Engineer", "Backend engineer"]), &w46(), 10);
        assert_eq!(
            q,
            [
                "Ho Chi Minh City technology job market report 2026",
                "Singapore technology job market report 2026",
                "Software Engineer hiring trends 2026 report",
                "Backend engineer hiring trends 2026 report",
                "Ho Chi Minh City Software Engineer salary survey 2026",
                "Singapore Software Engineer salary survey 2026",
                "Software Engineer skills demand 2026 Q4",
                "Backend engineer skills demand 2026 Q4",
            ]
        );
        let q2 = build_queries(&frame(&[], &["Data Analyst"]), &Period::parse("2026-Q2").unwrap(), 10);
        assert_eq!(q2, ["Data Analyst hiring trends 2026 report", "Data Analyst skills demand 2026 Q2"]);
        let no_role = build_queries(&frame(&["Berlin"], &[]), &w46(), 10);
        assert_eq!(no_role, ["Berlin technology job market report 2026", "Berlin tech salary survey 2026"]);
    }

    #[test]
    fn at_most_two_subjects_of_each_kind_and_every_subject_first_under_the_cap() {
        let big = frame(&["Hanoi", "Singapore", "Tokyo"], &["SRE", "Data Analyst", "Backend Engineer"]);
        let all = build_queries(&big, &w46(), 20);
        assert_eq!(all.len(), 8, "two locations and two roles, two queries each: {all:?}");
        assert!(!all.iter().any(|q| q.contains("Tokyo") || q.contains("Backend Engineer")), "{all:?}");
        assert_eq!(build_queries(&big, &w46(), 4), [
            "Hanoi technology job market report 2026",
            "Singapore technology job market report 2026",
            "SRE hiring trends 2026 report",
            "Data Analyst hiring trends 2026 report",
        ]);
        assert!(build_queries(&big, &w46(), 0).is_empty());
    }

    #[test]
    fn without_a_place_or_a_background_the_queries_are_general() {
        let generic = build_queries(&MarketFrame::default(), &w46(), 10);
        assert_eq!(generic.len(), 3);
        assert!(generic[0].contains("hiring trends report 2026"), "{generic:?}");
        assert!(generic[1].contains("2026 Q4"), "{generic:?}");
        assert_eq!(build_queries(&MarketFrame::default(), &w46(), 2).len(), 2);
    }

    #[test]
    fn wishes_never_change_the_queries() {
        let place = Preferences { locations: vec!["Singapore".into()], remote: Remote::Hybrid, ..Preferences::default() };
        let wishes = Preferences {
            must_skills: vec!["Kotlin".into()],
            nice_skills: vec!["Elixir".into()],
            avoid_skills: vec!["PHP".into()],
            domains: vec!["fintech".into(), "robotics".into()],
            avoid_domains: vec!["gambling".into()],
            roles: vec!["Product Manager".into()],
            salary_min: Some(150_000),
            currency: Some("SGD".into()),
            seniority: vec!["Staff".into()],
            ..place.clone()
        };
        let plain = build_queries(&MarketFrame::new(Some(&cv()), &place), &w46(), 20);
        let wished = build_queries(&MarketFrame::new(Some(&cv()), &wishes), &w46(), 20);
        assert_eq!(plain, wished);
        assert_eq!(plain[0], "Singapore technology job market report 2026");
        assert!(plain.contains(&"Software Engineer hiring trends 2026 report".to_string()), "{plain:?}");
        let all = plain.join(" ").to_lowercase();
        for wish in ["kotlin", "elixir", "php", "fintech", "robotics", "gambling", "product manager", "150", "sgd", "staff"] {
            assert!(!all.contains(wish), "{wish} in {plain:?}");
        }
    }

    #[test]
    fn subjects_are_sanitised_and_deduplicated() {
        let q = build_queries(&frame(&["  Ho Chi\tMinh\n", "", &"x".repeat(200)], &[]), &Period::parse("2026-Q1").unwrap(), 20);
        assert_eq!(q[0], "Ho Chi Minh technology job market report 2026");
        assert!(q.iter().all(|s| s.chars().count() < 120 && !s.contains('\n') && !s.contains('\t')), "{q:?}");
        let dup = build_queries(&frame(&["Singapore", "singapore"], &[]), &Period::parse("2026-Q1").unwrap(), 20);
        assert_eq!(dup.len(), 2, "case-insensitive duplicates collapse: {dup:?}");
    }

    const BRAVE: &str = r#"{"web": {"results": [
        {"title": "Fintech Annual Report 2026", "url": "BASE/reports/fintech", "description": "x"},
        {"title": "Not http", "url": "ftp://files.example/report.pdf"},
        {"title": "A PDF report", "url": "BASE/reports/outlook.pdf"},
        {"title": "Broken", "url": "BASE/reports/broken"},
        {"title": "Fintech Annual Report 2026 again", "url": "BASE/reports/fintech#page=2"},
        {"title": "No url"}
    ]}}"#;

    const TAVILY: &str = r#"{"query": "q", "results": [
        {"title": "Tavily hit", "url": "BASE/reports/fintech", "content": "snippet", "score": 0.9}
    ]}"#;

    fn report_page() -> String {
        "<html><head><title>Fintech Annual Report 2026</title></head><body><article><h1>Fintech Annual Report 2026</h1>".to_string()
            + &"<p>Payments companies hired more Rust and Go engineers this year while funding rounds shrank in the second quarter.</p>".repeat(5)
            + "</article></body></html>"
    }

    async fn fixtures() -> Fixtures {
        let fx = Fixtures::start().await;
        fx.route("/res/v1/web/search", 200, "application/json", BRAVE.replace("BASE", fx.base()));
        fx.route("/search", 200, "application/json", TAVILY.replace("BASE", fx.base()));
        fx.route("/reports/fintech", 200, "text/html", report_page());
        fx.route("/reports/broken", 500, "text/plain", "boom");
        let report = "= Outlook 2026\n".to_string() + &"Engineering headcount at large companies grew by four percent over the year. ".repeat(6);
        fx.route("/reports/outlook.pdf", 200, "application/pdf", typst_pdf(&report));
        fx
    }

    fn query() -> Vec<String> {
        vec!["fintech industry annual report 2026".to_string()]
    }

    #[tokio::test]
    async fn brave_results_are_filtered_and_read() {
        let fx = fixtures().await;
        let ctx = ctx(&fx, &[]);
        let items = collect(&ctx, SearchProvider::Brave, "brave-test-key-123", &query()).await.unwrap();

        let urls: Vec<_> = items.iter().map(|i| i.url.clone()).collect();
        assert_eq!(
            urls,
            [format!("{}/reports/fintech", fx.base()), format!("{}/reports/outlook.pdf", fx.base())],
            "ftp, a result without a URL, an unreadable page and the fragment duplicate are dropped"
        );
        let req = &fx.hits_on("/res/v1/web/search")[0];
        assert_eq!(req.method, "GET");
        assert!(req.target.contains("q=fintech+industry+annual+report+2026") && req.target.contains("count=5"), "{}", req.target);
        assert_eq!(req.header("x-subscription-token"), Some("brave-test-key-123"));
        assert_eq!(items[0].title, "Fintech Annual Report 2026");
        assert!(items[0].text.starts_with("# Fintech Annual Report 2026\n"), "{}", items[0].text);
        assert_eq!(items[1].title, "A PDF report");
        let flat = items[1].text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains("Engineering headcount at large companies grew"), "PDF text: {flat}");
    }

    #[tokio::test]
    async fn tavily_is_a_json_post_with_a_bearer_key() {
        let fx = fixtures().await;
        let items = collect(&ctx(&fx, &[]), SearchProvider::Tavily, "tvly-test-key-456", &query()).await.unwrap();
        assert_eq!(items.len(), 1);
        let req = &fx.hits_on("/search")[0];
        assert_eq!(req.method, "POST");
        assert_eq!(req.header("authorization"), Some("Bearer tvly-test-key-456"));
        let body: serde_json::Value = serde_json::from_str(&req.body).unwrap();
        assert_eq!((body["query"].as_str(), body["max_results"].as_u64()), (Some("fintech industry annual report 2026"), Some(5)));
        assert!(!req.body.contains("tvly-test-key-456"), "the key is a header, not part of the body");
    }

    #[tokio::test]
    async fn the_api_key_is_not_sent_to_another_host_when_the_api_redirects() {
        let fx = fixtures().await;
        let other = Fixtures::start().await;
        other.route("/steal", 200, "application/json", r#"{"web": {"results": []}}"#);
        fx.redirect("/res/v1/web/search", 302, &format!("{}/steal", other.base()));
        fx.redirect("/search", 307, &format!("{}/steal", other.base()));
        let ctx = ctx(&fx, &[]);

        for (provider, key) in [(SearchProvider::Brave, "brave-test-key-123"), (SearchProvider::Tavily, "tvly-test-key-456")] {
            let err = format!("{:#}", collect(&ctx, provider, key, &query()).await.unwrap_err());
            assert!(err.contains("not followed") && !err.contains(key), "{err}");
        }
        assert!(other.hits().is_empty(), "no key, no query reached the other host: {:?}", other.hits());
    }

    #[tokio::test]
    async fn results_on_this_machine_or_a_private_network_are_not_read() {
        let fx = fixtures().await;
        let mut c = ctx(&fx, &[]);
        let mut config = (*c.config).clone();
        config.limits.allow_private_hosts = false;
        c.config = std::sync::Arc::new(config);
        // The API itself is the fixture on 127.0.0.1 (the user's endpoint, not a result), but every
        // result points at it too and is refused.
        let items = collect(&c, SearchProvider::Brave, "brave-test-key-123", &query()).await.unwrap();
        assert!(items.is_empty());
        assert!(fx.hits().iter().all(|h| !h.path().starts_with("/reports")), "no result was requested: {:?}", fx.hits());
    }

    #[tokio::test]
    async fn known_results_are_not_fetched_and_a_dead_api_is_an_error() {
        let fx = fixtures().await;
        let known = [format!("{}/reports/fintech", fx.base()), format!("{}/reports/outlook.pdf", fx.base())];
        let c = ctx(&fx, &[known[0].as_str(), known[1].as_str()]);
        assert!(collect(&c, SearchProvider::Brave, "brave-test-key-123", &query()).await.unwrap().is_empty());
        assert!(fx.hits_on("/reports/fintech").is_empty() && fx.hits_on("/reports/outlook.pdf").is_empty());

        fx.route("/res/v1/web/search", 401, "application/json", r#"{"error": "bad key"}"#);
        let err = format!("{:#}", collect(&ctx(&fx, &[]), SearchProvider::Brave, "brave-test-key-123", &query()).await.unwrap_err());
        assert!(err.contains("HTTP 401") && !err.contains("brave-test-key-123"), "{err}");
    }
}
