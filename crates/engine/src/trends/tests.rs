//! The `trends` graph and the `Core` helpers, end to end against a local fixture server (the
//! adapters have their own tests next to them).

use std::time::{Duration, Instant};

use super::testutil::Fixtures;
use super::*;
use crate::graph::nodes::ChallengeKind;
use crate::graph::WaitReason;
use crate::pipeline::ingest::tests::JOB_HTML;
use crate::store::{NewSource, SourcePurpose};
use crate::Secrets;

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

#[tokio::test]
async fn a_run_collects_every_source_and_dedups() {
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

    assert_eq!(state.outcomes.len(), 4, "one outcome per source: {:?}", state.outcomes);

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
    let walled: anyhow::Error = anyhow::Error::new(crate::trends::http::FetchError::Status(401)).context("reading the page");
    assert_eq!(detect_challenge("page", &walled), Some(ChallengeKind::Login));
    assert_eq!(detect_challenge("rss", &walled), None);
    assert_eq!(detect_challenge("hackernews", &walled), None);
    assert_eq!(detect_challenge("page", &anyhow::anyhow!("boom")), None);
    assert_eq!(detect_challenge("page", &anyhow::Error::new(crate::trends::http::FetchError::Status(404))), None);

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
fn the_old_search_keys_are_removed_from_the_keyring() {
    let (_dir, core) = core();
    core.secrets.set("trends:brave_api_key", "BSA-key-0123456789").unwrap();
    core.secrets.set("trends:tavily_api_key", "tvly-key-0123456789").unwrap();
    core.forget_old_search_keys();
    assert_eq!(core.secrets.get("trends:brave_api_key").unwrap(), None);
    assert_eq!(core.secrets.get("trends:tavily_api_key").unwrap(), None);
    // Nothing saved: still fine.
    core.forget_old_search_keys();
}

#[test]
fn the_state_round_trips_and_the_thread_is_named_by_period() {
    assert_eq!(thread_id(&period()), "trends:2026-W46");
    assert_eq!(thread_id(&Period::parse("2026-Q4").unwrap()), "trends:2026-Q4");
    let mut state = TrendRun::new(period());
    state.hn_settled.push("903@1".into());
    state.outcomes.push(SourceOutcome { source_id: Some(3), kind: "page".into(), status: SourceStatus::NeedsHuman { kind: ChallengeKind::Login }, items: 0 });
    let json = serde_json::to_string(&state).unwrap();
    assert_eq!(serde_json::from_str::<TrendRun>(&json).unwrap(), state);
    // A state saved before a field existed still loads.
    let old = serde_json::from_str::<TrendRun>(r#"{"period": {"week": {"year": 2026, "week": 46}}}"#).unwrap();
    assert_eq!(old, TrendRun::new(period()));
    // A checkpoint saved before `hn_settled` existed.
    let before = serde_json::from_str::<TrendRun>(
        r#"{"period": {"week": {"year": 2026, "week": 46}}, "items": [], "outcomes": [], "stored": {"new": 2, "duplicates": 0}}"#,
    )
    .unwrap();
    assert!(before.hn_settled.is_empty());
    assert_eq!(before.stored.new, 2);
    // A run of a version that had report search: its plan and its outcome still load.
    let searched = serde_json::from_str::<TrendRun>(
        r#"{"period": {"week": {"year": 2026, "week": 46}},
            "plan": {"hackernews": [1], "search": {"provider": "brave", "queries": ["q"]}, "search_skip": null},
            "outcomes": [{"source_id": null, "kind": "report_search", "status": {"status": "skipped", "reason": "no search API key saved"}, "items": 0}]}"#,
    )
    .unwrap();
    assert_eq!((searched.plan.hackernews.as_slice(), searched.outcomes.len()), (&[1][..], 1));
}

/// Two hiring threads: November (the newest) and October. `/hn/search_by_date` also serves the
/// period's stories, so both threads are stories too.
const HN_TWO_THREADS: &str = r#"{"hits": [
    {"objectID": "902", "title": "Ask HN: Who is hiring? (November 2026)", "created_at_i": 1793635297, "points": 300, "num_comments": 2},
    {"objectID": "903", "title": "Ask HN: Who is hiring? (October 2026)", "created_at_i": 1790866927, "points": 250, "num_comments": 1}]}"#;
const HN_THREAD_OCT: &str = r#"{"id": 903, "children": [
    {"id": 8001, "created_at_i": 1790867000, "text": "Initech | Platform Engineer | Singapore, ONSITE | Kubernetes, Terraform, Go"}]}"#;

/// [`fixtures`] with two hiring threads, and only the Hacker News source.
async fn two_threads(core: &Core) -> Fixtures {
    let fx = fixtures().await;
    fx.route("/hn/search_by_date", 200, "application/json", HN_TWO_THREADS);
    fx.route("/hn/items/903", 200, "application/json", HN_THREAD_OCT);
    core.add_trend_source(SourceKind::HackerNews, "", "").unwrap();
    fx
}

/// The settled hiring threads in kv.
fn settled_kv(core: &Core) -> Option<Vec<String>> {
    core.store.kv_get(collect::HN_SETTLED_KEY).unwrap().map(|json| serde_json::from_str(&json).unwrap())
}

#[tokio::test]
async fn hiring_threads_read_in_full_are_not_downloaded_again() {
    let (_dir, core) = core();
    let fx = two_threads(&core).await;

    let first = run(&core, &fx).await;
    assert_eq!(status_of(&first, "hackernews"), &SourceStatus::Ok);
    assert_eq!((fx.hits_on("/hn/items/902").len(), fx.hits_on("/hn/items/903").len()), (1, 1), "the first run reads every thread");
    assert_eq!(first.hn_settled, ["903@1"]);
    assert_eq!(settled_kv(&core), Some(vec!["903@1".to_string()]), "the older thread, its postings stored; not the newest");

    // A new posting in the newest thread.
    let posting = r#"}, {"id": 7009, "created_at_i": 1793700000, "text": "Vandelay | Rust Engineer | Singapore | Rust, Tokio, Postgres"}]}"#;
    fx.route("/hn/items/902", 200, "application/json", HN_THREAD.replace("}]}", posting));
    let second = run(&core, &fx).await;
    assert_eq!(fx.hits_on("/hn/items/903").len(), 1, "October is not downloaded again");
    assert_eq!(fx.hits_on("/hn/items/902").len(), 2, "the newest thread is read every run");
    assert_eq!(second.stored, Stored { new: 1, duplicates: 0 });
    let stored = core.store.list_trend_items(None).unwrap();
    assert!(stored.iter().any(|i| i.url == "https://news.ycombinator.com/item?id=7009"), "the new posting is stored");
    assert!(second.hn_settled.is_empty());
    assert_eq!(settled_kv(&core), Some(vec!["903@1".to_string()]));
}

#[tokio::test]
async fn the_settled_threads_keep_the_newest_ids_and_a_bad_value_is_ignored() {
    let (_dir, core) = core();
    let fx = two_threads(&core).await;

    // Not a list: ignored, every thread is read, and the value is replaced.
    core.store.kv_set(collect::HN_SETTLED_KEY, "not json").unwrap();
    run(&core, &fx).await;
    assert_eq!(fx.hits_on("/hn/items/903").len(), 1);
    assert_eq!(settled_kv(&core), Some(vec!["903@1".to_string()]));

    // 30 older threads, October with an old count, and keys that are not settled keys: one key per
    // thread (the new count), the 24 newest threads, newest first.
    let mut saved: Vec<String> = (1..=30).map(|n| format!("{n}@5")).collect();
    saved.extend(["903@0".into(), "abc@1".into(), "17".into()]);
    core.store.kv_set(collect::HN_SETTLED_KEY, &serde_json::to_string(&saved).unwrap()).unwrap();
    run(&core, &fx).await;
    assert_eq!(fx.hits_on("/hn/items/903").len(), 2, "October's count changed: read again");
    let kept = settled_kv(&core).unwrap();
    let expected: Vec<String> = std::iter::once("903@1".to_string()).chain((8..=30).rev().map(|n| format!("{n}@5"))).collect();
    assert_eq!(kept, expected);

    // Nothing new settles: October is skipped, and kv is not rewritten (a rewrite would sort it).
    let reversed: Vec<String> = kept.iter().rev().cloned().collect();
    core.store.kv_set(collect::HN_SETTLED_KEY, &serde_json::to_string(&reversed).unwrap()).unwrap();
    run(&core, &fx).await;
    assert_eq!(fx.hits_on("/hn/items/903").len(), 2, "October is not read again");
    assert_eq!(settled_kv(&core).unwrap(), reversed);
    // A key that is already saved changes nothing either.
    collect::record_settled(&core.store, &["903@1".to_string()]).unwrap();
    assert_eq!(settled_kv(&core).unwrap(), reversed);
}

#[tokio::test]
async fn a_run_that_fails_before_its_items_are_stored_records_no_thread() {
    let (_dir, core) = core();
    let fx = two_threads(&core).await;
    // `store_items` fails: every insert into trend_items is refused.
    let refuse = "CREATE TRIGGER refuse_items BEFORE INSERT ON trend_items BEGIN SELECT RAISE(ABORT, 'disk full'); END;";
    core.store.with_conn(|c| Ok(c.execute_batch(refuse)?)).unwrap();

    let outcome = core.collect_trends_with(period(), config(&fx)).await.unwrap();
    assert!(matches!(outcome, Outcome::Failed(_)), "{outcome:?}");
    assert_eq!(fx.hits_on("/hn/items/903").len(), 1, "the collector read October");
    assert_eq!(settled_kv(&core), None, "nothing is recorded before the postings are stored");

    // Retry: the items are stored, then the thread is recorded.
    core.store.with_conn(|c| Ok(c.execute_batch("DROP TRIGGER refuse_items;")?)).unwrap();
    let Outcome::Done(state) = core.resume_trends_collection(period(), config(&fx)).await.unwrap() else { panic!("the retry finishes") };
    assert!(state.stored.new > 0);
    assert_eq!(settled_kv(&core), Some(vec!["903@1".to_string()]));
}
