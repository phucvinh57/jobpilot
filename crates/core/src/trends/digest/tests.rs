//! The digest end to end (against the fixture server of P13a and a scripted model: nothing leaves
//! the machine) and its checks, node by node.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Duration as Span, Utc};
use serde_json::{json, Value};

use super::*;
use crate::graph::nodes::ChallengeKind;
use crate::graph::{Outcome, WaitReason};
use crate::health::Problem;
use crate::llm::fake::FakeLlm;
use crate::llm::LlmFuture;
use crate::prefs::Preferences;
use crate::store::{NewLead, NewTrendItem};
use crate::trends::collect::{SourceStatus, TrendRun};
use crate::trends::source::SourceKind;
use crate::trends::tests::{add_all, config, core, fixtures, status_of};
use crate::{Core, Secrets};

/// This week: items are fetched now, so they are inside it whatever the clock says.
fn period() -> Period {
    Period::current_week()
}

/// A model that reads the prompts it gets: extraction answers quote each item's own words, the
/// digest is whatever `synth` makes of the evidence listing.
struct Scripted {
    calls: Mutex<Vec<(String, String)>>,
    /// Extraction calls that fail before any succeeds.
    fail_extract: AtomicUsize,
    synth: Box<dyn Fn(&str) -> Value + Send + Sync>,
}

impl Scripted {
    fn new(synth: impl Fn(&str) -> Value + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { calls: Mutex::default(), fail_extract: AtomicUsize::new(0), synth: Box::new(synth) })
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }

    fn extract_calls(&self) -> usize {
        self.calls().iter().filter(|(_, user)| user.contains("<item id=")).count()
    }

    fn synth_calls(&self) -> usize {
        self.calls().iter().filter(|(_, user)| user.contains("<evidence>")).count()
    }
}

impl Llm for Scripted {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        self.calls.lock().unwrap().push((system.into(), user.into()));
        let reply = if schema["properties"]["items"].is_object() {
            let failing = self
                .fail_extract
                .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok();
            if failing {
                Err(anyhow::anyhow!("HTTP 400 Bad Request: the model refused"))
            } else {
                Ok(extract_reply(user))
            }
        } else {
            Ok((self.synth)(user))
        };
        Box::pin(std::future::ready(reply))
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

/// For every `<item id="N">` of the prompt: the item's longest line, five words of it, as a claim.
fn extract_reply(user: &str) -> Value {
    let mut items = Vec::new();
    for block in user.split("<item id=\"").skip(1) {
        let (id, rest) = block.split_once("\">").unwrap();
        let body = rest.split("</item>").next().unwrap();
        let text = body.split("Text:\n").nth(1).unwrap_or_default();
        let line = text.lines().filter(|l| !l.trim().starts_with('#')).max_by_key(|l| l.len()).unwrap_or_default();
        let quote = line.split_whitespace().take(5).collect::<Vec<_>>().join(" ");
        items.push(json!({"id": id.parse::<i64>().unwrap(), "topics": ["rust"], "claims": [{"text": format!("says {quote}"), "quote": quote}]}));
    }
    json!({"items": items})
}

/// The id of the evidence item whose line starts with `[id] <title>`.
fn id_of(user: &str, title: &str) -> i64 {
    let line = user.lines().find(|l| l.starts_with('[') && l.contains(&format!("] {title}"))).unwrap_or_else(|| panic!("no evidence line for {title}"));
    line[1..line.find(']').unwrap()].parse().unwrap()
}

/// A digest with two good points and three that must not survive.
fn mixed_points(user: &str) -> Value {
    let rust = id_of(user, "Rust 2.0 released");
    let post = id_of(user, "Acme | Senior Rust Engineer");
    json!({"points": [
        {"section": "rising_skills", "topic": "Rust", "text": "Rust is asked for more.", "quote": "Rust 2.0 released", "citations": [rust, post]},
        {"section": "hiring", "topic": "", "text": "Acme hires remote Rust engineers.", "quote": "Senior Rust Engineer", "citations": [post]},
        {"section": "domains", "topic": "robotics", "text": "Robotics is active (no source).", "quote": "", "citations": []},
        {"section": "funding", "topic": "", "text": "Cites an item that does not exist.", "quote": "", "citations": [987654]},
        {"section": "funding", "topic": "", "text": "Acme raised a lot.", "quote": "Acme raised ten billion dollars last week", "citations": [post]},
        {"section": "for_you", "topic": "", "text": "Matches your Rust preference.", "quote": "", "citations": [post, rust, post]}
    ]})
}

/// One point citing the first evidence item, no quote.
fn one_point(user: &str) -> Value {
    let line = user.lines().find(|l| l.starts_with('[')).unwrap();
    let id: i64 = line[1..line.find(']').unwrap()].parse().unwrap();
    json!({"points": [{"section": "hiring", "topic": "", "text": "Someone is hiring.", "quote": "", "citations": [id]}]})
}

fn digest_config(llm: &Arc<Scripted>) -> DigestConfig {
    DigestConfig { llm: Some(llm.clone()), limits: DigestLimits { batch_items: 100, ..DigestLimits::default() } }
}

fn done(outcome: Outcome<TrendRun>) -> TrendRun {
    match outcome {
        Outcome::Done(state) => state,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn digest_of(core: &Core, state: &TrendRun) -> Digest {
    let id = state.digest.report_id.expect("a saved report");
    core.trend_digest(id).unwrap().expect("the report is stored")
}

#[tokio::test]
async fn a_run_with_the_fake_model_saves_a_report_that_cites_its_source() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    // One source, one item: its id is 1, so the fake model's replies can name it.
    core.add_trend_source(SourceKind::Page, "Job page", &format!("{}/job", fx.base())).unwrap();
    let llm = Arc::new(FakeLlm::with_replies([
        json!({"items": [{"id": 1, "topics": ["Rust", "Go"], "claims": [
            {"text": "Acme hires a Senior Rust Engineer", "quote": "Design and build backend services in Rust and Go"},
            {"text": "Made up", "quote": "Acme was acquired by a giant"}]}]}),
        json!({"points": [
            {"section": "hiring", "topic": "", "text": "Acme Robotics is hiring a Senior Rust Engineer.", "quote": "Design and build backend services in Rust and Go", "citations": [1]},
            {"section": "domains", "topic": "robotics", "text": "Robotics is active.", "quote": "", "citations": []}
        ]}),
    ]));

    let digest = DigestConfig { llm: Some(llm.clone()), ..DigestConfig::default() };
    let state = done(core.collect_trends_and_digest_with(period(), config(&fx), digest).await.unwrap());

    // The model saw the item as untrusted data, and only the verified claim reached the synthesis.
    let calls = llm.calls();
    assert_eq!(calls.len(), 2, "one extraction batch, one synthesis");
    assert!(calls[0].user.contains("<item id=\"1\">") && calls[0].system.contains("UNTRUSTED"));
    assert!(calls[1].system.contains("UNTRUSTED") && calls[1].user.contains("<evidence>"), "{}", calls[1].user);
    assert!(calls[1].user.contains("Design and build backend services in Rust and Go"));
    assert!(!calls[1].user.contains("acquired by a giant"), "a claim with a fake quote is dropped before the synthesis");

    // The saved report.
    let digest = digest_of(&core, &state);
    assert_eq!((digest.report.kind.as_str(), digest.report.period.as_str()), ("weekly", period().label().as_str()));
    assert_eq!(digest.report.title, format!("Weekly digest {}", period().label()));
    let data = digest.data.expect("readable data");
    assert_eq!(data.points.len(), 1, "the uncited point is gone: {:?}", data.points);
    assert_eq!((data.dropped, data.items_read), (1, 1));
    assert_eq!(data.points[0].citations, [1]);
    assert_eq!(data.items.len(), 1);
    assert_eq!(data.items[0].url, format!("{}/job", fx.base()));
    assert_eq!(data.counts.months.len(), 6);
    assert!(digest.report.body_md.contains("## Hiring signals") && digest.report.body_md.contains(&format!("[1]({}/job)", fx.base())), "{}", digest.report.body_md);
    assert!(digest.report.body_md.contains("point(s) were left out"));
    assert_eq!(core.latest_trend_digest().unwrap().unwrap().report.id, digest.report.id);

    // report_search was skipped (no key), and no search API was called.
    assert_eq!(status_of(&state, "report_search"), &SourceStatus::Skipped { reason: "no search API key saved".into() });
    assert!(fx.hits().iter().all(|h| h.path() != "/res/v1/web/search" && h.path() != "/search"));
}

#[tokio::test]
async fn uncited_points_unknown_ids_and_fake_quotes_are_removed_from_a_full_digest() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    let llm = Scripted::new(mixed_points);

    let state = done(core.collect_trends_and_digest_with(period(), config(&fx), digest_config(&llm)).await.unwrap());

    let data = digest_of(&core, &state).data.unwrap();
    let kept: Vec<(Section, &str)> = data.points.iter().map(|p| (p.section, p.text.as_str())).collect();
    assert_eq!(
        kept,
        [
            (Section::RisingSkills, "Rust is asked for more."),
            (Section::Hiring, "Acme hires remote Rust engineers."),
            (Section::ForYou, "Matches your Rust preference.")
        ],
        "kept in section order"
    );
    assert_eq!(data.dropped, 3, "no citation, an unknown id, a quote that is not in the source");
    assert_eq!(data.items_read, 9);
    // Every citation resolves to a stored item, once each.
    for point in &data.points {
        let mut ids = point.citations.clone();
        ids.dedup();
        assert_eq!(ids.len(), point.citations.len(), "{point:?}");
        for id in &point.citations {
            assert!(core.store.get_trend_item(*id).unwrap().is_some());
            assert!(data.items.iter().any(|i| i.id == *id));
        }
    }
    assert_eq!(data.points[2].citations.len(), 2, "the repeated id is cited once");
    assert_eq!(state.digest.failed_batches, 0);
    assert_eq!(llm.extract_calls(), 1);
    assert_eq!(llm.synth_calls(), 1);
}

#[tokio::test]
async fn a_collector_that_times_out_does_not_stop_the_digest() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    fx.route_after("/hn/search", Duration::from_secs(5), 200, "application/json", crate::trends::tests::HN_FRONT);
    let mut cfg = config(&fx);
    cfg.limits.source_timeout = Duration::from_millis(300);
    let llm = Scripted::new(one_point);

    let started = Instant::now();
    let state = done(core.collect_trends_and_digest_with(period(), cfg, digest_config(&llm)).await.unwrap());
    assert!(started.elapsed() < Duration::from_secs(4), "did not wait for the slow source");

    assert_eq!(status_of(&state, "hackernews"), &SourceStatus::TimedOut);
    let data = digest_of(&core, &state).data.unwrap();
    assert_eq!(data.items_read, 5, "yc 2, feed 2, page 1: the digest is about what was collected");
    assert_eq!(data.points.len(), 1);
}

#[tokio::test]
async fn a_source_at_a_challenge_pauses_the_run_but_not_the_digest() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();
    let wall = core.add_trend_source(SourceKind::Page, "Walled", &format!("{}/walled", fx.base())).unwrap();
    fx.route("/walled", 403, "text/html", "bot check");
    let llm = Scripted::new(one_point);

    let outcome = core.collect_trends_and_digest_with(period(), config(&fx), digest_config(&llm)).await.unwrap();
    let url = format!("{}/walled", fx.base());
    assert_eq!(outcome, Outcome::Waiting(WaitReason::Blocked { source_id: wall.id, url: url.clone() }));
    assert!(core.latest_trend_digest().unwrap().is_none(), "the paused run has not built it yet");
    assert_eq!(llm.calls().len(), 0);
    let waiting = core.trend_challenges().unwrap();
    assert_eq!(
        waiting,
        [TrendChallenge { period: period(), source_id: wall.id, source_name: "Walled".into(), url, kind: ChallengeKind::Blocked }]
    );

    // Build it now from what the other collectors stored; the collection stays paused.
    let state = done(core.build_trends_digest_with(period(), digest_config(&llm)).await.unwrap());
    let first = digest_of(&core, &state);
    assert_eq!(first.data.as_ref().unwrap().items_read, 2, "the feed's two items");
    assert_eq!(core.trend_challenges().unwrap().len(), 1, "still waiting for the user");
    assert_eq!(llm.extract_calls(), 1);

    // Continue: the paused run goes on to the digest; what was read before is not read again.
    let state = done(core.resume_trends_collection_with(period(), config(&fx), digest_config(&llm)).await.unwrap());
    assert!(core.trend_challenges().unwrap().is_empty());
    let second = digest_of(&core, &state);
    assert_ne!(first.report.id, second.report.id, "a new row; the newest wins");
    assert_eq!(core.latest_trend_digest().unwrap().unwrap().report.id, second.report.id);
    assert_eq!(core.trend_reports().unwrap().len(), 2);
    assert_eq!(llm.extract_calls(), 1, "the extraction was cached");
    assert_eq!(llm.synth_calls(), 2);
}

#[tokio::test]
async fn regenerating_asks_the_model_only_about_new_items() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();
    let llm = Scripted::new(one_point);
    done(core.collect_trends_and_digest_with(period(), config(&fx), digest_config(&llm)).await.unwrap());
    assert_eq!((llm.extract_calls(), llm.synth_calls()), (1, 1));

    // A new item arrives; regenerate from what is stored.
    let text = core.paths.trends_dir().join("extra.txt");
    std::fs::write(&text, "# A new post\n\nWe adopted Zig for the storage engine and cut tail latency in half.").unwrap();
    core.store
        .add_trend_item(&NewTrendItem { url: "https://extra.test/post".into(), title: "A new post".into(), text_path: Some(text), ..Default::default() })
        .unwrap();
    let state = done(core.build_trends_digest_with(period(), digest_config(&llm)).await.unwrap());

    let calls = llm.calls();
    assert_eq!(llm.extract_calls(), 2);
    let second_extract = calls.iter().filter(|(_, u)| u.contains("<item id=")).nth(1).unwrap();
    assert_eq!(second_extract.1.matches("<item id=").count(), 1, "only the new item is read: {}", second_extract.1);
    assert!(second_extract.1.contains("A new post"));
    assert_eq!(llm.synth_calls(), 2);
    assert_eq!(digest_of(&core, &state).data.unwrap().items_read, 3);
    assert_eq!(core.trend_reports().unwrap().len(), 2);
}

#[tokio::test]
async fn without_a_provider_the_run_fails_with_a_problem_the_ui_can_act_on() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    core.add_trend_source(SourceKind::Rss, "Blog", &format!("{}/feed.xml", fx.base())).unwrap();

    // No model configured and no default provider: collection works, the digest says why not.
    let outcome = core.collect_trends_and_digest_with(period(), config(&fx), DigestConfig::default()).await.unwrap();
    let Outcome::Failed(failure) = outcome else { panic!("expected a failure, got {outcome:?}") };
    assert_eq!((failure.node.as_str(), failure.problem), ("extract", Problem::NoProvider));
    assert_eq!(core.store.list_trend_items(None).unwrap().len(), 2, "the items were stored first");
    assert!(core.latest_trend_digest().unwrap().is_none());

    // A model that is down: the same step fails, as a network problem that offers Retry; the
    // failed thread is picked up where it stopped.
    let down = DigestConfig { llm: Some(Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"))), ..DigestConfig::default() };
    let Outcome::Failed(failure) = core.resume_trends_collection_with(period(), config(&fx), down).await.unwrap() else { panic!("still down") };
    assert_eq!(failure.problem, Problem::Network);
    assert!(failure.problem.retryable());

    let llm = Scripted::new(one_point);
    let state = done(core.resume_trends_collection_with(period(), config(&fx), digest_config(&llm)).await.unwrap());
    assert!(state.digest.report_id.is_some());
}

#[tokio::test]
async fn one_failed_extraction_batch_does_not_fail_the_digest() {
    let (_dir, core) = core();
    let fx = fixtures().await;
    add_all(&core, &fx);
    let llm = Scripted::new(one_point);
    llm.fail_extract.store(1, Ordering::SeqCst);
    let digest = DigestConfig {
        llm: Some(llm.clone()),
        limits: DigestLimits { batch_items: 3, concurrency: 1, ..DigestLimits::default() },
    };

    let state = done(core.collect_trends_and_digest_with(period(), config(&fx), digest).await.unwrap());
    assert_eq!(state.digest.failed_batches, 1);
    assert_eq!(llm.extract_calls(), 3);
    assert_eq!(state.digest.extractions.len(), 6, "the two batches that worked");
    assert!(state.digest.report_id.is_some());
}

#[tokio::test]
async fn an_empty_period_is_an_error_not_an_empty_report() {
    let (_dir, core) = core();
    let llm = Scripted::new(one_point);
    let outcome = core.build_trends_digest_with(period(), digest_config(&llm)).await.unwrap();
    let Outcome::Failed(failure) = outcome else { panic!("expected a failure") };
    assert!(failure.message.contains("no trend items"), "{}", failure.message);
    assert_eq!(llm.calls().len(), 0);
    assert!(core.trend_reports().unwrap().is_empty());
}

// ---------------------------------------------------------------- the nodes on their own

fn env_with(store: Store) -> Arc<DigestEnv> {
    Arc::new(DigestEnv { store, secrets: Secrets::memory(), config: DigestConfig::default() })
}

/// An item with a text file under `dir`.
fn stored_item(store: &Store, dir: &std::path::Path, url: &str, title: &str, text: &str, published: Option<chrono::DateTime<Utc>>) -> i64 {
    let path = dir.join(format!("{}.txt", url.replace(['/', ':', '?', '='], "_")));
    std::fs::write(&path, text).unwrap();
    store
        .add_trend_item(&NewTrendItem { url: url.into(), title: title.into(), published_at: published, text_path: Some(path), ..Default::default() })
        .unwrap()
        .unwrap()
        .id
}

#[tokio::test]
async fn enforce_citations_keeps_only_points_a_reader_can_check() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let a = stored_item(&store, dir.path(), "https://a.test/1", "Acme grows", "# Acme grows\n\nAcme, a fintech startup, raised a $20M Series B \u{2013} \"we are hiring 30 engineers\" said the CEO.", None);
    let b = stored_item(&store, dir.path(), "https://b.test/2", "Other", "# Other\n\nNothing about hiring here, just a long enough sentence.", None);
    let outside = stored_item(&store, dir.path(), "https://c.test/3", "Not in the digest", "# Not in the digest\n\nAcme raised a $20M Series B.", None);
    let mut state = TrendRun::new(period());
    state.digest.items = vec![a, b];
    let point = |text: &str, quote: &str, citations: &[i64]| Point {
        section: Section::Funding,
        topic: String::new(),
        text: text.into(),
        quote: quote.into(),
        citations: citations.to_vec(),
    };
    state.digest.points = vec![
        point("uncited", "", &[]),
        point("cites an item of another digest", "", &[outside]),
        point("cites nothing real", "", &[0, -4, 12345]),
        point("quote from nowhere", "Acme raised $200M from a famous fund", &[a]),
        point("quote from the wrong item", "raised a $20M Series B", &[b]),
        point("good, quote differs in case, quotes and dashes", "RAISED a \"$20M Series B\" - we are hiring 30 engineers", &[a, b, a, 777]),
        point("good, quote is the title", "Acme grows", &[a]),
        point("good, no quote", "", &[b]),
        point("good, no quote", "", &[b]),
        point("   ", "", &[a]),
        point("quote too short to prove anything", "Acme", &[a]),
    ];

    let env = env_with(store);
    let state = cite::enforce_citations(env, state).await.unwrap();

    let kept: Vec<&str> = state.digest.points.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(
        kept,
        ["good, quote differs in case, quotes and dashes", "good, quote is the title", "good, no quote"],
        "duplicates are collapsed"
    );
    assert_eq!(state.digest.dropped, 7, "uncited, outside, unreal ids, two quotes not in the source, empty text, quote too short");
    assert_eq!(state.digest.points[0].citations, [a, b], "invalid and repeated ids are removed from a point that stays");
}

#[tokio::test]
async fn the_digest_counts_hiring_postings_and_leads_from_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now();
    let posting = |text: &str| format!("# Hacker News, Ask HN: Who is hiring? (this month)\n\n{text}");
    stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=1", "Acme | Rust", &posting("Acme | Rust engineer | Kafka"), Some(now));
    stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=2", "Globex | Go", &posting("Globex | Go and Rust, Kubernetes"), Some(now - Span::days(1)));
    // Not postings: a story on HN, and a hiring header on another site's page.
    stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=3", "Rust 2.0", "# Rust 2.0 released\n\nRust Rust Rust", Some(now));
    stored_item(&store, dir.path(), "https://blog.test/hiring", "Blog", &posting("Rust Kafka Kubernetes"), Some(now));
    // A posting from long before the window is not counted either.
    stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=4", "Old", &posting("Rust Rust"), Some(now - Span::days(400)));
    let jd = dir.path().join("lead.md");
    std::fs::write(&jd, "We need strong Rust and Kubernetes skills. Docker too.").unwrap();
    store
        .add_lead(&NewLead { url: "https://jobs.test/1".into(), company: "Initech".into(), role: "Platform Engineer".into(), jd_path: Some(jd), ..Default::default() })
        .unwrap();
    store.save_preferences(&Preferences { nice_skills: vec!["Kafka".into(), "Gleam".into()], ..Preferences::default() }).unwrap();

    let state = count::count(env_with(store), TrendRun::new(Period::week_of(now.date_naive()))).await.unwrap();

    let counts = state.digest.counts.unwrap();
    assert_eq!(counts.months.len(), 6);
    assert_eq!(counts.documents.iter().sum::<u32>(), 3, "two postings and one lead");
    let total = |skill: &str| counts.skills.iter().find(|s| s.skill == skill).map(|s| s.total);
    assert_eq!(total("Rust"), Some(3));
    assert_eq!(total("Kubernetes"), Some(2));
    assert_eq!(total("Kafka"), Some(1));
    assert_eq!(total("Go"), Some(1));
    assert_eq!(total("Docker"), Some(1));
    assert_eq!(total("Gleam"), None, "the user's skills are counted, and absent when nobody asks for them");
}

#[test]
fn items_are_spread_over_the_sources() {
    let at = Utc::now();
    let item = |id: i64, source: Option<i64>, age: i64| TrendItem {
        id,
        source_id: source,
        url: format!("https://x.test/{id}"),
        title: format!("t{id}"),
        published_at: Some(at - Span::minutes(age)),
        text_path: None,
        fetched_at: at,
    };
    // Newest first, as the store lists them: source 1 has many, source 2 and the search have one each.
    let items = vec![item(1, Some(1), 1), item(2, Some(1), 2), item(3, Some(1), 3), item(4, Some(2), 4), item(5, None, 5), item(6, Some(1), 6)];
    let picked: Vec<i64> = spread(items.clone(), 4).iter().map(|i| i.id).collect();
    assert_eq!(picked, [1, 2, 4, 5], "one from each source before a second from any, then newest first");
    assert_eq!(spread(items.clone(), 10).len(), 6);
    assert_eq!(spread(items, 0).len(), 0);
}

#[test]
fn a_report_reads_back_and_an_unreadable_one_still_shows_its_text() {
    let (_dir, core) = core();
    let data = DigestData {
        items: vec![CitedItem { id: 3, title: "T".into(), url: "https://t.test/".into() }],
        points: vec![Point { section: Section::Domains, topic: "fintech".into(), text: "Fintech is active.".into(), quote: "q q q q q q".into(), citations: vec![3] }],
        counts: SkillCounts::default(),
        items_read: 4,
        dropped: 0,
    };
    let saved = core.save_trend_digest(&Period::parse("2026-Q4").unwrap(), &data).unwrap();
    assert_eq!((saved.kind.as_str(), saved.title.as_str()), ("quarterly", "Quarterly summary 2026-Q4"));
    assert!(saved.body_md.contains("- **fintech**: Fintech is active. (sources: [3](https://t.test/))\n  > q q q q q q"), "{}", saved.body_md);
    let back = core.trend_digest(saved.id).unwrap().unwrap();
    assert_eq!(back.data, Some(data));
    assert_eq!(back.period(), Period::parse("2026-Q4"));

    let odd = core.store.add_report(&crate::store::NewReport { kind: "weekly".into(), period: "2026-W45".into(), title: "Old".into(), body_md: "plain".into(), citations_json: "{broken".into() }).unwrap();
    let back = core.trend_digest(odd.id).unwrap().unwrap();
    assert!(back.data.is_none());
    assert_eq!(back.report.body_md, "plain");
    assert!(core.trend_digest(9999).unwrap().is_none());
}

#[test]
fn search_keys_that_are_saved_are_listed() {
    let (_dir, core) = core();
    assert!(core.saved_search_providers().unwrap().is_empty());
    core.set_search_key(crate::trends::SearchProvider::Tavily, "tvly-key-0123456789").unwrap();
    assert_eq!(core.saved_search_providers().unwrap(), [crate::trends::SearchProvider::Tavily]);
    core.set_search_key(crate::trends::SearchProvider::Brave, "BSA-key-0123456789").unwrap();
    assert_eq!(core.saved_search_providers().unwrap().len(), 2);
}
