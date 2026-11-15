//! The `trends` graph and the `Core` helpers, end to end against a local fixture server (the
//! adapters have their own tests next to them).

use std::path::Path;
use std::time::{Duration, Instant};

use super::testutil::{Fixtures, Hit};
use super::*;
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::graph::WaitReason;
use crate::engine::pipeline::ingest::tests::JOB_HTML;
use crate::engine::prefs::Preferences;
use crate::engine::store::{NewSource, SourcePurpose};

fn period() -> Period {
    Period::parse("2026-W46").unwrap()
}

pub(super) fn core() -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
    (dir, core)
}

pub(super) fn config(fx: &Fixtures) -> TrendConfig {
    TrendConfig {
        endpoints: Endpoints {
            hackernews: format!("{}/hn", fx.base()),
            yc: format!("{}/yc", fx.base()),
            brave: fx.base().to_string(),
            tavily: fx.base().to_string(),
        },
        limits: CollectLimits {
            host_delay: Duration::ZERO,
            allow_private_hosts: true,
            source_timeout: Duration::from_secs(10),
            collector_budget: Duration::from_secs(20),
            ..CollectLimits::default()
        },
    }
}

pub(super) const HN_FRONT: &str = r#"{"hits": [{"objectID": "1", "title": "Rust 2.0 released", "url": "https://blog.example.com/rust-2", "points": 450, "num_comments": 210, "created_at_i": 1794000000}]}"#;
const HN_BY_DATE: &str = r#"{"hits": [{"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297, "points": 300, "num_comments": 2}]}"#;
const HN_THREAD: &str = r#"{"id": 902, "children": [
    {"id": 7001, "created_at_i": 1793635303, "text": "Acme | Senior Rust Engineer | REMOTE | We build robots in Rust and Go and need help."},
    {"id": 7005, "created_at_i": 1793635600, "text": "Globex | Data Engineer | Berlin, ONSITE | Python, Spark, Kafka, dbt, Airflow, Snowflake"}]}"#;
const YC_META: &str = r#"{"batches": {"fall-2026": {"count": 231}}}"#;
const YC_BATCH: &str = r#"[
    {"name": "Hirey", "slug": "hirey", "one_liner": "Hiring tools", "batch": "Fall 2026", "isHiring": true},
    {"name": "Tire Swing", "slug": "tire-swing", "one_liner": "Housing AI", "batch": "Fall 2026"}]"#;

fn feed(base: &str) -> String {
    let body = "<p>We moved the platform to Postgres 19 and Kafka, and latency dropped by forty percent across every region. The migration took three quarters, covered forty services, and taught the team about backpressure, schema evolution and the cost of dual writes.</p>";
    format!(
        r#"<?xml version="1.0"?><rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/"><channel><title>Blog</title>
        <item><title>Scaling</title><link>{base}/posts/scaling</link><pubDate>Tue, 10 Nov 2026 12:00:00 GMT</pubDate><content:encoded><![CDATA[{body}]]></content:encoded></item>
        <item><title>Shared</title><link>{base}/posts/shared</link><pubDate>Mon, 09 Nov 2026 12:00:00 GMT</pubDate><content:encoded><![CDATA[{body}]]></content:encoded></item>
        </channel></rss>"#
    )
}

/// Every route the full run needs.
pub(super) async fn fixtures() -> Fixtures {
    let fx = Fixtures::start().await;
    fx.route("/hn/search", 200, "application/json", HN_FRONT);
    fx.route("/hn/search_by_date", 200, "application/json", HN_BY_DATE);
    fx.route("/hn/items/902", 200, "application/json", HN_THREAD);
    fx.route("/yc/meta.json", 200, "application/json", YC_META);
    fx.route("/yc/batches/fall-2026.json", 200, "application/json", YC_BATCH);
    fx.route("/feed.xml", 200, "application/rss+xml", feed(fx.base()));
    fx.route("/posts/shared", 200, "text/html", JOB_HTML);
    fx.route("/job", 200, "text/html", JOB_HTML);
    fx
}

pub(super) fn add_all(core: &Core, fx: &Fixtures) {
    core.add_trend_source(SourceKind::HackerNews, "", "").unwrap();
    core.add_trend_source(SourceKind::Yc, "", "").unwrap();
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();
    core.add_trend_source(SourceKind::Page, "Job page", &format!("{}/job", fx.base())).unwrap();
}

async fn run(core: &Core, fx: &Fixtures) -> TrendRun {
    match core.collect_trends_with(period(), config(fx)).await.unwrap() {
        Outcome::Done(state) => state,
        other => panic!("expected Done, got {other:?}"),
    }
}

pub(super) fn status_of<'a>(state: &'a TrendRun, kind: &'a str) -> &'a SourceStatus {
    &state.outcomes_of(kind).next().unwrap_or_else(|| panic!("no outcome for {kind}: {:?}", state.outcomes)).status
}

fn read_all(dir: &Path, found: &mut Vec<(std::path::PathBuf, Vec<u8>)>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            read_all(&path, found);
        } else {
            found.push((path.clone(), std::fs::read(&path).unwrap()));
        }
    }
}

#[tokio::test]
async fn a_run_collects_every_source_dedups_and_skips_report_search_without_a_key() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);

    let state = run(&core, &fx).await;

    // Per source.
    for kind in ["hackernews", "yc", "rss", "page"] {
        assert_eq!(status_of(&state, kind), &SourceStatus::Ok, "{kind}: {:?}", state.outcomes);
    }
    assert_eq!(state.outcomes_of("hackernews").next().unwrap().items, 4, "front-page story, thread-as-story and two postings");
    assert_eq!(state.outcomes_of("yc").next().unwrap().items, 2);
    assert_eq!(state.outcomes_of("rss").next().unwrap().items, 2);
    assert_eq!(state.outcomes_of("page").next().unwrap().items, 1);
    assert_eq!(state.stored, Stored { new: 9, duplicates: 0 });

    // report_search: skipped with a reason, and not one request reached a search API.
    assert_eq!(
        status_of(&state, "report_search"),
        &SourceStatus::Skipped { reason: "no search API key saved".into() }
    );
    assert!(state.plan.search.is_none());
    let paths: Vec<String> = fx.hits().iter().map(|h: &Hit| h.path().to_string()).collect();
    assert!(!paths.iter().any(|p| p == "/res/v1/web/search" || p == "/search" || p.starts_with("/reports")), "{paths:?}");

    // Stored: rows, text files, source status.
    let items = core.store.list_trend_items(None).unwrap();
    assert_eq!(items.len(), 9);
    for item in &items {
        let path = item.text_path.as_ref().expect("text path");
        assert!(path.starts_with(core.paths.trends_dir()), "{}", path.display());
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.starts_with("# "), "{text}");
    }
    let blog = items.iter().find(|i| i.url.ends_with("/posts/scaling")).unwrap();
    assert_eq!(blog.title, "Scaling");
    assert_eq!(blog.published_at.unwrap().timestamp(), 1794312000);
    assert!(blog.source_id.is_some());
    for source in core.trend_sources().unwrap() {
        assert_eq!(source.last_status.as_deref(), Some("ok"), "{}", source.kind);
        assert!(source.last_run_at.is_some());
    }

    // The same period again: everything is known, nothing is fetched twice, nothing is added.
    let before = fx.hits().len();
    let again = run(&core, &fx).await;
    assert_eq!(again.stored, Stored { new: 0, duplicates: 0 });
    assert_eq!(core.store.list_trend_items(None).unwrap().len(), 9);
    let second: Vec<String> = fx.hits().iter().skip(before).map(|h| h.path().to_string()).collect();
    assert!(!second.contains(&"/job".to_string()), "a known page is not fetched again: {second:?}");
}

#[tokio::test]
async fn the_same_address_from_two_collectors_is_stored_once() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    // The feed's second entry and a page source name the same address.
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();
    core.add_trend_source(SourceKind::Page, "Shared", &format!("{}/posts/shared", fx.base())).unwrap();

    let state = run(&core, &fx).await;
    assert_eq!(state.items.len(), 3, "both collectors read it");
    assert_eq!(state.stored, Stored { new: 2, duplicates: 1 });
    let items = core.store.list_trend_items(None).unwrap();
    assert_eq!(items.iter().filter(|i| i.url.ends_with("/posts/shared")).count(), 1);
}

#[tokio::test]
async fn report_search_runs_with_a_key_and_the_key_stays_in_the_keyring() {
    let (dir, core) = core();
    let fx = fixtures().await;
    let key = "BSAtest-brave-key-0123456789";
    core.set_search_key(SearchProvider::Brave, key).unwrap();
    // The queries come from the profile's roles and the preferred locations; the wishes (domains,
    // target roles, skills) are saved too and must not show up.
    crate::engine::profile::set_test_profile(
        &core,
        "basics:\n  name: Test\n  headline: Backend Engineer\nexperience:\n  - company: Acme\n    position: Platform Engineer\n",
    );
    core.save_preferences(Preferences {
        locations: vec!["Singapore".into()],
        domains: vec!["fintech".into()],
        roles: vec!["Product Manager".into()],
        must_skills: vec!["Kotlin".into()],
        ..Preferences::default()
    })
    .unwrap();
    let report = "<html><head><title>Fintech report</title></head><body><article><h1>Fintech report</h1>".to_string()
        + &"<p>Payments companies hired more Rust engineers this year while funding rounds shrank.</p>".repeat(6)
        + "</article></body></html>";
    fx.route("/reports/fintech", 200, "text/html", report);
    fx.route(
        "/res/v1/web/search",
        200,
        "application/json",
        format!(r#"{{"web": {{"results": [{{"title": "Fintech 2026", "url": "{}/reports/fintech"}}]}}}}"#, fx.base()),
    );

    let state = run(&core, &fx).await;

    let search = state.plan.search.clone().expect("search planned");
    assert_eq!(search.provider, SearchProvider::Brave);
    assert_eq!(
        search.queries,
        [
            "Singapore technology job market report 2026",
            "Platform Engineer hiring trends 2026 report",
            "Backend Engineer hiring trends 2026 report",
            "Singapore Platform Engineer salary survey 2026",
            "Platform Engineer skills demand 2026 Q4",
            "Backend Engineer skills demand 2026 Q4",
        ],
        "background and place, capped at max_queries"
    );
    let all = search.queries.join(" ").to_lowercase();
    assert!(!all.contains("fintech") && !all.contains("product manager") && !all.contains("kotlin"), "{all}");
    let outcome = state.outcomes_of("report_search").next().unwrap();
    assert_eq!((&outcome.status, outcome.items, outcome.source_id), (&SourceStatus::Ok, 1, None));
    let stored = core.store.list_trend_items(None).unwrap();
    let item = stored.iter().find(|i| i.url.ends_with("/reports/fintech")).unwrap();
    assert_eq!((item.source_id, item.title.as_str()), (None, "Fintech 2026"));
    let searches = fx.hits_on("/res/v1/web/search");
    assert_eq!(searches.len(), search.queries.len(), "one request per query");
    assert_eq!(searches[0].header("x-subscription-token"), Some(key));

    // The key is in the keyring and in no checkpoint, database file or text file.
    assert_eq!(core.secrets.get(SearchProvider::Brave.secret_name()).unwrap().as_deref(), Some(key));
    for row in core.store.graph_checkpoints(&thread_id(&period())).unwrap() {
        assert!(!row.state_json.contains(key), "state of step {}", row.step);
    }
    let mut files = Vec::new();
    read_all(dir.path(), &mut files);
    assert!(files.len() > 3);
    for (path, bytes) in files {
        assert!(!bytes.windows(key.len()).any(|w| w == key.as_bytes()), "{} contains the key", path.display());
    }
}

#[tokio::test]
async fn a_collector_that_times_out_is_left_out_and_the_run_still_finishes() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    // Hacker News answers after 5 s; its source gets 300 ms.
    fx.route_after("/hn/search", Duration::from_secs(5), 200, "application/json", HN_FRONT);
    let mut cfg = config(&fx);
    cfg.limits.source_timeout = Duration::from_millis(300);

    let started = Instant::now();
    let Outcome::Done(state) = core.collect_trends_with(period(), cfg).await.unwrap() else { panic!("the run must finish") };
    assert!(started.elapsed() < Duration::from_secs(4), "did not wait for the slow source: {:?}", started.elapsed());

    assert_eq!(status_of(&state, "hackernews"), &SourceStatus::TimedOut);
    assert_eq!(state.outcomes_of("hackernews").next().unwrap().items, 0);
    for kind in ["yc", "rss", "page"] {
        assert_eq!(status_of(&state, kind), &SourceStatus::Ok, "{kind}");
    }
    assert_eq!(state.stored.new, 5, "yc 2, feed 2, page 1; nothing of Hacker News");
    let hn = core.trend_sources().unwrap().into_iter().find(|s| s.kind == "hackernews").unwrap();
    assert_eq!(hn.last_status.as_deref(), Some("timed out"));
    assert!(hn.last_run_at.is_some());
}

#[tokio::test]
async fn a_spent_collector_budget_leaves_the_remaining_sources_out() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    let mut cfg = config(&fx);
    cfg.limits.collector_budget = Duration::ZERO;
    let Outcome::Done(state) = core.collect_trends_with(period(), cfg).await.unwrap() else { panic!("the run must finish") };
    for kind in ["hackernews", "yc", "rss", "page"] {
        assert_eq!(status_of(&state, kind), &SourceStatus::TimedOut, "{kind}");
    }
    assert_eq!(state.stored, Stored::default());
    assert!(core.store.list_trend_items(None).unwrap().is_empty());
}

#[tokio::test]
async fn a_failing_source_is_reported_without_its_address_and_the_others_are_kept() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    core.add_trend_source(SourceKind::Yc, "", "").unwrap();
    core.add_trend_source(SourceKind::Rss, "Private feed", &format!("{}/gone.xml?token=hunter2hunter2", fx.base())).unwrap();
    fx.route("/gone.xml", 404, "text/plain", "gone");

    let state = run(&core, &fx).await;
    assert_eq!(status_of(&state, "yc"), &SourceStatus::Ok);
    assert_eq!(status_of(&state, "rss"), &SourceStatus::Failed { error: "HTTP 404".into() });
    let feed = core.trend_sources().unwrap().into_iter().find(|s| s.kind == "rss").unwrap();
    assert_eq!(feed.last_status.as_deref(), Some("failed: HTTP 404"));
    assert!(!format!("{:?}", state.outcomes).contains("hunter2"));
    assert_eq!(state.stored.new, 2);
}

#[tokio::test]
async fn a_login_or_bot_wall_on_a_page_pauses_after_the_other_items_are_stored() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();
    let wall = core.add_trend_source(SourceKind::Page, "Walled", &format!("{}/walled", fx.base())).unwrap();
    fx.route("/walled", 403, "text/html", "bot check");

    let outcome = core.collect_trends_with(period(), config(&fx)).await.unwrap();
    let url = format!("{}/walled", fx.base());
    assert_eq!(outcome, Outcome::Waiting(WaitReason::Blocked { source_id: wall.id, url: url.clone() }));
    assert_eq!(core.store.list_trend_items(None).unwrap().len(), 2, "the feed's items are stored before the pause");
    assert_eq!(core.store.get_source(wall.id).unwrap().unwrap().last_status.as_deref(), Some("needs_human"));
    let waiting = core.graphs.waiting().unwrap();
    assert_eq!((waiting.len(), waiting[0].thread.as_str()), (1, "trends:2026-W46"));

    // Asking again while it waits starts over (and meets the wall again); "Continue" carries on.
    let again = core.collect_trends_with(period(), config(&fx)).await.unwrap();
    assert_eq!(again, Outcome::Waiting(WaitReason::Blocked { source_id: wall.id, url }));
    let Outcome::Done(state) = core.resume_trends_collection(period(), config(&fx)).await.unwrap() else { panic!("continues to the end") };
    assert!(state.challenge.is_none());
    assert_eq!(status_of(&state, "page"), &SourceStatus::NeedsHuman { kind: ChallengeKind::Blocked });
    assert!(core.graphs.waiting().unwrap().is_empty());

    // A login wall is a login; an API source never pauses the run.
    let walled: anyhow::Error = anyhow::Error::new(crate::engine::trends::http::FetchError::Status(401)).context("reading the page");
    assert_eq!(detect_challenge("page", &walled), Some(ChallengeKind::Login));
    assert_eq!(detect_challenge("rss", &walled), None);
    assert_eq!(detect_challenge("hackernews", &walled), None);
    assert_eq!(detect_challenge("page", &anyhow::anyhow!("boom")), None);
    assert_eq!(detect_challenge("page", &anyhow::Error::new(crate::engine::trends::http::FetchError::Status(404))), None);

    fx.route("/hn/search", 403, "text/plain", "blocked");
    fx.route("/hn/search_by_date", 403, "text/plain", "blocked");
    core.add_trend_source(SourceKind::HackerNews, "", "").unwrap();
    fx.route("/walled", 200, "text/html", JOB_HTML);
    let state = run(&core, &fx).await;
    assert!(matches!(status_of(&state, "hackernews"), SourceStatus::Failed { .. }), "{:?}", state.outcomes);
}

#[tokio::test]
async fn disabled_and_removed_sources_are_not_collected() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    let sources = core.trend_sources().unwrap();
    let id = |kind: &str| sources.iter().find(|s| s.kind == kind).unwrap().id;
    core.set_trend_source_enabled(id("hackernews"), false).unwrap();
    core.remove_trend_source(id("yc")).unwrap();

    let state = run(&core, &fx).await;
    assert_eq!((state.plan.hackernews.len(), state.plan.yc.len(), state.plan.feeds.len()), (0, 0, 2));
    assert_eq!(state.outcomes_of("hackernews").count() + state.outcomes_of("yc").count(), 0);
    assert!(fx.hits().iter().all(|h| !h.path().starts_with("/hn") && !h.path().starts_with("/yc")));
    assert_eq!(state.stored.new, 3);
}

#[test]
fn trend_sources_are_validated_normalised_and_scoped_to_trends() {
    let (_dir, core) = core();
    let hn = core.add_trend_source(SourceKind::HackerNews, "", "ignored").unwrap();
    assert_eq!((hn.kind.as_str(), hn.name.as_str(), hn.url.as_str(), hn.purpose), ("hackernews", "Hacker News", "", SourcePurpose::Trends));
    assert!(core.add_trend_source(SourceKind::HackerNews, "again", "").unwrap_err().to_string().contains("already"));

    let feed = core.add_trend_source(SourceKind::Rss, "  ", " https://blog.example.com/feed.xml#top ").unwrap();
    assert_eq!((feed.name.as_str(), feed.url.as_str()), ("blog.example.com", "https://blog.example.com/feed.xml"));
    assert!(core.add_trend_source(SourceKind::Rss, "dup", "https://blog.example.com/feed.xml").unwrap_err().to_string().contains("already"));
    for bad in ["", "not a url", "ftp://x.test/feed", "file:///etc/passwd", "javascript:alert(1)"] {
        assert!(core.add_trend_source(SourceKind::Page, "x", bad).is_err(), "{bad}");
    }

    // A job source is not a trend source.
    let job = core.store.add_source(&NewSource { kind: "greenhouse".into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap();
    assert!(core.remove_trend_source(job.id).unwrap_err().to_string().contains("not a trend source"));
    assert!(core.set_trend_source_enabled(job.id, false).is_err());
    assert!(core.remove_trend_source(9999).unwrap_err().to_string().contains("not found"));
    assert_eq!(core.trend_sources().unwrap().len(), 2);

    core.remove_trend_source(feed.id).unwrap();
    assert_eq!(core.trend_sources().unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), [hn.id]);
}

#[test]
fn search_keys_are_saved_through_the_secrets_and_brave_wins() {
    let (_dir, core) = core();
    assert_eq!(core.search_provider().unwrap(), None);
    assert!(core.set_search_key(SearchProvider::Tavily, "   ").is_err());
    core.set_search_key(SearchProvider::Tavily, " tvly-key-0123456789 ").unwrap();
    assert_eq!(core.search_provider().unwrap(), Some(SearchProvider::Tavily));
    assert_eq!(core.secrets.get("trends:tavily_api_key").unwrap().as_deref(), Some("tvly-key-0123456789"));
    core.set_search_key(SearchProvider::Brave, "BSA-key-0123456789").unwrap();
    assert_eq!(core.search_provider().unwrap(), Some(SearchProvider::Brave));
    core.clear_search_key(SearchProvider::Brave).unwrap();
    core.clear_search_key(SearchProvider::Brave).unwrap();
    assert_eq!(core.search_provider().unwrap(), Some(SearchProvider::Tavily));
    core.clear_search_key(SearchProvider::Tavily).unwrap();
    assert_eq!(core.search_provider().unwrap(), None);
}

#[test]
fn the_state_round_trips_and_the_thread_is_named_by_period() {
    assert_eq!(thread_id(&period()), "trends:2026-W46");
    assert_eq!(thread_id(&Period::parse("2026-Q4").unwrap()), "trends:2026-Q4");
    let mut state = TrendRun::new(period());
    state.outcomes.push(SourceOutcome { source_id: Some(3), kind: "page".into(), status: SourceStatus::NeedsHuman { kind: ChallengeKind::Login }, items: 0 });
    state.outcomes.push(SourceOutcome { source_id: None, kind: "report_search".into(), status: SourceStatus::Skipped { reason: "no search API key saved".into() }, items: 0 });
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(serde_json::from_str::<TrendRun>(&json).unwrap(), state);
    // A state saved before a field existed still loads.
    let old = serde_json::from_str::<TrendRun>(r#"{"period": {"week": {"year": 2026, "week": 46}}}"#).unwrap();
    assert_eq!(old, TrendRun::new(period()));
}
