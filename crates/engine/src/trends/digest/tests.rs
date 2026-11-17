//! The digest end to end (against the collection's fixture server and a scripted model: nothing leaves
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
    DigestConfig { llm: Some(llm.clone()), limits: DigestLimits { batch_items: 100, ..DigestLimits::default() }, profile: None }
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
        profile: None,
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
async fn a_quote_that_is_the_whole_title_proves_even_when_it_is_short() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let a = stored_item(&store, dir.path(), "https://a.test/1", "Rust 2.0", "# Rust 2.0\n\nThe release notes mention async closures and nothing else of note.", None);
    let mut state = TrendRun::new(period());
    state.digest.items = vec![a];
    let point = |text: &str, quote: &str| Point { section: Section::Hiring, topic: String::new(), text: text.into(), quote: quote.into(), citations: vec![a] };
    state.digest.points = vec![
        point("the whole title, shorter than the minimum", "Rust 2.0"),
        point("the whole title, other case and marks", "\u{201c}rust 2.0\u{201d}"),
        point("only part of the title", "Rust"),
        point("a short piece of the text", "async"),
        point("a short quote from nowhere", "Go 3.0"),
    ];

    let state = cite::enforce_citations(env_with(store), state).await.unwrap();

    let kept: Vec<&str> = state.digest.points.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(kept, ["the whole title, shorter than the minimum", "the whole title, other case and marks"]);
    assert_eq!(state.digest.dropped, 3);
}

#[tokio::test]
async fn enforce_citations_applies_the_limits_of_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let ids: Vec<i64> = (0..8).map(|n| stored_item(&store, dir.path(), &format!("https://a.test/{n}"), &format!("Item {n}"), "text", None)).collect();
    let mut state = TrendRun::new(period());
    state.digest.items = ids.clone();
    state.digest.counts = Some(SkillCounts {
        wishes: vec![WishSignal { wish: "Rust".into(), kind: WishKind::Skill, avoided: false, direction: Direction::Unclear, mentions: 0 }],
        ..SkillCounts::default()
    });
    // Five valid points in each section, each citing all eight items.
    state.digest.points = Section::ALL
        .iter()
        .flat_map(|section| {
            let topic = if *section == Section::WishesCheck { "Rust" } else { "" };
            let ids = &ids;
            (0..5).map(move |n| Point { section: *section, topic: topic.into(), text: format!("{} point {n}", section.as_str()), quote: String::new(), citations: ids.clone() })
        })
        .collect();

    let state = cite::enforce_citations(env_with(store), state).await.unwrap();

    let points = &state.digest.points;
    assert_eq!(points.len(), cite::MAX_POINTS, "twenty in all");
    for section in Section::ALL {
        assert!(points.iter().filter(|p| p.section == section).count() <= cite::MAX_POINTS_PER_SECTION, "{section:?}");
    }
    assert!(points.iter().all(|p| p.citations.len() == cite::MAX_CITATIONS), "five citations each, the first five");
    assert_eq!(points[0].citations, ids[..cite::MAX_CITATIONS]);
    assert_eq!(points[0].text, "rising_skills point 0", "the model's own order decides which stay");
    assert_eq!(state.digest.dropped, 0, "points over the limits are not 'left out for a bad source'");
    assert!(SYSTEM_RULES.iter().all(|rule| synthesize::SYSTEM.contains(rule)), "the prompt states the same limits");
}

/// The limits as the prompt words them.
const SYSTEM_RULES: [&str; 3] = ["at most 5", "At most 4 points per section", "at most 20 in total"];

#[test]
fn the_markdown_carries_no_link_or_markup_the_model_wrote() {
    let item = |id, url: &str| CitedItem { id, title: "Title [x](http://evil.test)".into(), url: url.into() };
    let data = DigestData {
        items: vec![
            item(1, "https://t.test/a b(1)?q=)"),
            item(2, "javascript:alert(1)"),
            item(3, "file:///etc/passwd"),
            item(4, "not a url"),
        ],
        points: vec![
            Point {
                section: Section::Hiring,
                topic: "x](http://evil.test) *".into(),
                text: "[click](http://evil.test) <script>alert(1)</script> **bold** ![img](http://evil.test/i.png) www.evil.test mail a@b.test https://evil.test/x `code` &lt;\n# not a heading".into(),
                quote: "[quote](http://evil.test)\n> more".into(),
                citations: vec![1, 2, 3, 4],
            },
            Point { section: Section::ForYou, topic: String::new(), text: "- not a nested list".into(), quote: String::new(), citations: vec![1] },
        ],
        counts: SkillCounts { months: vec!["2026-11".into()], documents: vec![1], skills: vec![SkillCount { skill: "a|b".into(), by_month: vec![1], total: 1, ..SkillCount::default() }], ..SkillCounts::default() },
        items_read: 5,
        dropped: 0,
    };

    let md = cite::render_markdown(&Period::parse("2026-W46").unwrap(), &data);

    // The only links are the stored addresses of cited web items, with the characters that end a link encoded.
    assert_eq!(md.matches("[1](https://t.test/a%20b%281%29?q=%29)").count(), 2, "{md}");
    for plain in ["[2]", "[3]", "[4]"] {
        assert!(md.contains(plain) && !md.contains(&format!("{plain}(")), "{plain} in {md}");
    }
    for bad in ["javascript:", "file:", "](http://", "![", "www.evil", "a@b", "https://evil", "`code`", "**bold**", "\n>"] {
        assert!(!md.contains(bad), "{bad:?} in {md}");
    }
    assert!(md.lines().all(|l| !l.starts_with('#') || l.starts_with("# Weekly digest") || l.starts_with("## ")), "{md}");
    for escaped in [
        r"\[click\](http\://evil.test)",
        r"\<script\>alert(1)\</script\>",
        r"\*\*bold\*\*",
        r"!\[img\](http\://evil.test/i.png)",
        r"www\.evil.test",
        r"a\@b.test",
        r"https\://evil.test/x",
        r"\`code\`",
        r"\&lt;",
        r"**x\](http\://evil.test) \***",
        "  > \\[quote\\](http\\://evil.test) \\> more",
        r"- \- not a nested list",
        r"| a\|b |",
    ] {
        assert!(md.contains(escaped), "{escaped:?} missing in {md}");
    }
}

#[tokio::test]
async fn the_digest_counts_hiring_postings_and_never_mixes_in_the_leads() {
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
    store.save_preferences(&Preferences { nice_skills: vec!["Kafka".into(), "Gleam".into()], ..Preferences::default() }).unwrap();
    let env = env_with(store);
    let period = Period::week_of(now.date_naive());

    let before = count::count(env.clone(), TrendRun::new(period)).await.unwrap().digest.counts.unwrap();
    assert_eq!(before.months.len(), 6);
    assert_eq!(before.documents.iter().sum::<u32>(), 2, "two postings");
    let total = |counts: &SkillCounts, skill: &str| counts.skills.iter().find(|s| s.skill == skill).map(|s| s.total);
    assert_eq!(total(&before, "Rust"), Some(2));
    assert_eq!(total(&before, "Kubernetes"), Some(1));
    assert_eq!(total(&before, "Kafka"), Some(1));
    assert_eq!(total(&before, "Go"), Some(1));
    assert_eq!(total(&before, "Gleam"), None, "a wished skill is measured for the wishes only");
    assert!(before.skills.iter().find(|s| s.skill == "Kafka").is_some_and(|s| s.wished));
    assert!(before.pipeline.is_none());

    // A lead arrives: the market numbers do not move, the lead is counted apart.
    let jd = dir.path().join("lead.md");
    std::fs::write(&jd, "We need strong Rust and Kubernetes skills. Docker too.").unwrap();
    env.store
        .add_lead(&NewLead { url: "https://jobs.test/1".into(), company: "Initech".into(), role: "Platform Engineer".into(), jd_path: Some(jd), ..Default::default() })
        .unwrap();
    let after = count::count(env, TrendRun::new(period)).await.unwrap().digest.counts.unwrap();
    assert_eq!((&after.documents, &after.skills, &after.local), (&before.documents, &before.skills, &before.local));
    assert_eq!((after.gap_scope, &after.gap, &after.wishes), (before.gap_scope, &before.gap, &before.wishes));
    let pipeline = after.pipeline.expect("the lead is in the pipeline");
    assert_eq!(pipeline.documents.iter().sum::<u32>(), 1);
    assert_eq!(total(&SkillCounts { skills: pipeline.skills, ..SkillCounts::default() }, "Docker"), Some(1));
}

/// A backend engineer who knows Rust and Python.
const PROFILE: &str = "basics:
  name: Test Person
  headline: Backend Engineer
  email: test@example.com
experience:
  - company: Acme
    position: Backend Engineer
    start: \"2020\"
    end: Present
    bullets: [Built services]
skills:
  - category: Languages
    items: [Rust, Python]
";

/// Six months of hiring postings in the window of [`period`]: per month 12 backend postings in
/// Singapore and 4 in Berlin. Python is in 8 of the 12 local ones in the first three months and in
/// 2 later, Rust the other way round; Kafka is in half throughout, Gleam in 3, Kotlin in 1, and
/// fintech in 2. Plus one news item of this week that the model cites.
fn store_market(store: &Store, dir: &std::path::Path) {
    let months = count::month_window(&period(), 6);
    for (m, month) in months.iter().enumerate() {
        let published = chrono::DateTime::parse_from_rfc3339(&format!("{month}-01T00:00:00Z")).unwrap().with_timezone(&Utc);
        let late = m >= 3;
        for n in 0..16 {
            let mut skills = Vec::new();
            let place = if n < 12 { "Singapore" } else { "Berlin" };
            if n >= 12 {
                skills.push("Go, Kubernetes");
            } else {
                if (!late && n < 8) || (late && n < 2) {
                    skills.push("Python");
                }
                if (late && n < 8) || (!late && n < 2) {
                    skills.push("Rust");
                }
                if n % 2 == 0 {
                    skills.push("Kafka");
                }
                if n < 3 {
                    skills.push("Gleam");
                }
                if n == 5 {
                    skills.push("Kotlin");
                }
                if n < 2 {
                    skills.push("fintech payments");
                }
            }
            let title = format!("Acme{n} | Backend Engineer | {place}");
            let text = format!("# Hacker News, Ask HN: Who is hiring? ({month})\n\n{title} | {}", skills.join(", "));
            stored_item(store, dir, &format!("https://news.ycombinator.com/item?id={}", 1000 + m * 100 + n), &title, &text, Some(published));
        }
    }
    stored_item(store, dir, "https://news.test/market", "Market news", "# Market news\n\nCompanies in Singapore keep hiring backend engineers this quarter.", Some(Utc::now()));
}

/// A digest that gets some things right and some against the app's numbers.
fn lens_points(user: &str) -> Value {
    let news = id_of(user, "Market news");
    let point = |section: &str, topic: &str, text: &str| json!({"section": section, "topic": topic, "text": text, "quote": "", "citations": [news]});
    json!({"points": [
        point("rising_skills", "Rust", "Rust is asked for more."),
        point("rising_skills", "Python", "Python is asked for more (the app counted it falling)."),
        point("falling_skills", "python", "Python is asked for less."),
        point("for_you", "Rust", "Build on Rust."),
        point("for_you", "COBOL", "Learn COBOL (not in the gap)."),
        point("wishes_check", "Kotlin", "Kotlin is steady."),
        point("wishes_check", "Haskell", "Haskell (not a wish)."),
    ]})
}

#[tokio::test]
async fn wishes_never_change_the_market_numbers_or_the_gap() {
    let (dir, core) = core();
    store_market(&core.store, dir.path());
    let profile = crate::pipeline::cv::Cv::parse(PROFILE).unwrap();
    let place = Preferences { locations: vec!["Singapore".into()], remote: crate::prefs::Remote::Onsite, ..Preferences::default() };
    let first = Preferences {
        roles: vec!["Product Manager".into()],
        must_skills: vec!["Kotlin".into(), "Gleam".into()],
        avoid_skills: vec!["PHP".into()],
        domains: vec!["fintech".into()],
        avoid_domains: vec!["gambling".into()],
        ..place.clone()
    };
    let second = Preferences {
        roles: vec!["Data Scientist".into()],
        must_skills: vec!["Elixir".into()],
        nice_skills: vec!["Rust".into()],
        domains: vec!["robotics".into()],
        ..place
    };
    let run = |prefs: Preferences| {
        let core = &core;
        let profile = profile.clone();
        async move {
            core.store.save_preferences(&prefs).unwrap();
            let llm = Scripted::new(lens_points);
            let config = DigestConfig { profile: Some(profile), ..digest_config(&llm) };
            let state = done(core.build_trends_digest_with(period(), config).await.unwrap());
            let prompt = llm.calls().into_iter().map(|(_, user)| user).find(|u| u.contains("<evidence>")).unwrap();
            (digest_of(core, &state).data.unwrap(), prompt)
        }
    };
    let (a, prompt_a) = run(first).await;
    let (b, _) = run(second).await;

    // The market numbers and the gap are the same; only the wishes (and their marks) differ.
    let unmark = |skills: &[SkillCount]| skills.iter().map(|s| SkillCount { wished: false, ..s.clone() }).collect::<Vec<_>>();
    assert_eq!(a.counts.documents, b.counts.documents);
    assert_eq!(unmark(&a.counts.skills), unmark(&b.counts.skills));
    assert_eq!(a.counts.local.as_ref().map(|l| (l.documents.clone(), unmark(&l.skills))), b.counts.local.as_ref().map(|l| (l.documents.clone(), unmark(&l.skills))));
    assert_eq!((a.counts.gap_scope, &a.counts.gap), (b.counts.gap_scope, &b.counts.gap));
    assert_ne!(a.counts.wishes, b.counts.wishes);
    assert_ne!(a.counts.skills, b.counts.skills, "the wished marks differ");

    // ... and they are not trivial: a local gap from the profile, directions from shares.
    assert_eq!(a.counts.documents, [16; 6]);
    assert_eq!(a.counts.gap_scope, GapScope::Local);
    let gap: Vec<(&str, GapBucket)> = a.counts.gap.iter().map(|g| (g.skill.as_str(), g.bucket)).collect();
    assert_eq!(gap, [("Rust", GapBucket::BuildOn), ("Kafka", GapBucket::LearnNext), ("Kotlin", GapBucket::LearnNext), ("Python", GapBucket::AtRisk)]);
    assert!(a.counts.skills.iter().all(|s| s.skill != "Gleam"), "a wish is not part of the market vocabulary");
    let gleam = a.counts.wishes.iter().find(|w| w.wish == "Gleam").unwrap();
    assert_eq!((gleam.mentions, gleam.direction), (18, Direction::Steady), "but it is measured");
    assert_eq!(a.counts.field, ["backend", "engineer"]);

    // The prompt: the background from the profile, wishes only in their block, no target roles.
    let (head, _) = prompt_a.split_once("\nEvidence:").unwrap();
    assert!(head.contains("- Headline: Backend Engineer") && head.contains("- place: Singapore"), "{head}");
    assert!(!prompt_a.contains("test@example.com") && !prompt_a.contains("Product Manager"));
    let wishes = &head[head.find("Wishes to check").unwrap()..];
    for wish in ["Gleam", "fintech", "gambling", "PHP"] {
        assert!(wishes.contains(wish), "{wish}");
        assert_eq!(head.matches(wish).count(), wishes.matches(wish).count(), "{wish} outside the wishes block: {head}");
    }

    // The checks of the points against the counts.
    let kept = |data: &DigestData| data.points.iter().map(|p| (p.section, p.topic.clone())).collect::<Vec<_>>();
    assert_eq!(
        kept(&a),
        [
            (Section::RisingSkills, "Rust".to_string()),
            (Section::FallingSkills, "python".to_string()),
            (Section::ForYou, "Rust".to_string()),
            (Section::WishesCheck, "Kotlin".to_string()),
        ],
        "a rise the app counted as a fall, a skill outside the gap and a topic that is not a wish are dropped"
    );
    assert_eq!(a.dropped, 3);
    assert!(!kept(&b).contains(&(Section::WishesCheck, "Kotlin".to_string())), "Kotlin is not a wish of the second run");
    assert_eq!(b.dropped, 4);
}

#[tokio::test]
async fn enforce_citations_drops_points_against_the_counts() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let a = stored_item(&store, dir.path(), "https://a.test/1", "News", "# News\n\nSomething happened in the market.", None);
    let skill = |name: &str, direction: Direction| SkillCount { skill: name.into(), by_month: vec![9], total: 9, direction, ..SkillCount::default() };
    let gap = |name: &str, bucket: GapBucket| GapSkill { skill: name.into(), bucket, direction: Direction::Steady, share_permille: 100 };
    let wish = |name: &str, kind: WishKind| WishSignal { wish: name.into(), kind, avoided: false, direction: Direction::Unclear, mentions: 0 };
    let mut state = TrendRun::new(period());
    state.digest.items = vec![a];
    state.digest.counts = Some(SkillCounts {
        // The two scopes disagree on Rust and Python: a point that one table supports is kept.
        skills: vec![skill("Python", Direction::Rising), skill("Rust", Direction::Falling)],
        local: Some(ScopeCounts {
            documents: vec![80],
            skills: vec![
                skill("Rust", Direction::Rising),
                skill("Python", Direction::Falling),
                skill("Kafka", Direction::Steady),
                skill("Zig", Direction::Unclear),
            ],
        }),
        gap_scope: GapScope::Local,
        gap: vec![gap("Rust", GapBucket::BuildOn), gap("Go", GapBucket::LearnNext)],
        wishes: vec![wish("Kotlin", WishKind::Skill), wish("fintech", WishKind::Domain)],
        ..SkillCounts::default()
    });
    let point = |section: Section, topic: &str| Point { section, topic: topic.into(), text: format!("{} {topic}", section.as_str()), quote: String::new(), citations: vec![a] };
    use Section::*;
    state.digest.points = vec![
        point(RisingSkills, "Rust"),
        point(RisingSkills, "python"),
        point(RisingSkills, "Kafka"),
        point(RisingSkills, "Zig"),
        point(RisingSkills, "Mojo"),
        point(FallingSkills, "Python"),
        point(FallingSkills, "Rust"),
        point(FallingSkills, "Kafka"),
        point(ForYou, ""),
        point(ForYou, "golang"),
        point(ForYou, "Kafka"),
        point(WishesCheck, "KOTLIN"),
        point(WishesCheck, ""),
        point(WishesCheck, "Rust"),
        point(Domains, "fintech"),
    ];

    let state = cite::enforce_citations(env_with(store), state).await.unwrap();

    let kept: Vec<(Section, &str)> = state.digest.points.iter().map(|p| (p.section, p.topic.as_str())).collect();
    assert_eq!(
        kept,
        [
            (RisingSkills, "Rust"),
            (RisingSkills, "python"),
            (RisingSkills, "Zig"),
            (RisingSkills, "Mojo"),
            (FallingSkills, "Python"),
            (FallingSkills, "Rust"),
            (Domains, "fintech"),
            (ForYou, ""),
            (ForYou, "golang"),
            (WishesCheck, "KOTLIN"),
        ],
        "either table supports Rust and Python; unclear and uncounted skills rest on their evidence; golang is Go, a gap skill; wishes ignore case"
    );
    assert_eq!(state.digest.dropped, 5, "Kafka rising and falling (steady in the only table that counts it), Kafka for you, two wishes");
}

#[test]
fn a_report_saved_before_the_market_scan_still_loads() {
    let old = r#"{"items":[{"id":3,"title":"T","url":"https://t.test/"}],
        "points":[{"section":"for_you","topic":"","text":"Matches your Rust preference.","quote":"","citations":[3]}],
        "counts":{"months":["2026-10","2026-11"],"documents":[3,4],"skills":[{"skill":"Rust","by_month":[1,2],"total":3}]},
        "items_read":2,"dropped":1}"#;
    let data: DigestData = serde_json::from_str(old).unwrap();
    assert_eq!(data.counts.skills[0], SkillCount { skill: "Rust".into(), by_month: vec![1, 2], total: 3, ..SkillCount::default() });
    assert_eq!(data.counts.skills[0].direction, Direction::Unclear);
    assert!(data.counts.local.is_none() && data.counts.pipeline.is_none());
    assert!(data.counts.gap.is_empty() && data.counts.wishes.is_empty() && data.counts.uncatalogued.is_empty());
    assert_eq!(data.counts.gap_scope, GapScope::Global);

    let (_dir, core) = core();
    let row = core
        .store
        .add_report(&crate::store::NewReport { kind: "weekly".into(), period: "2026-W46".into(), title: "Old".into(), body_md: "old".into(), citations_json: old.into() })
        .unwrap();
    assert_eq!(core.trend_digest(row.id).unwrap().unwrap().data, Some(data.clone()));
    // Rendered again, it has no part it lacks the data for.
    let md = cite::render_markdown(&Period::parse("2026-W46").unwrap(), &data);
    assert!(!md.contains("## Gap") && !md.contains("## Wishes") && !md.contains("Terms not in the list"), "{md}");
}

#[test]
fn the_markdown_shows_the_gap_the_wishes_and_the_new_terms_from_the_counts() {
    let data = DigestData {
        counts: SkillCounts {
            months: vec!["2026-11".into()],
            documents: vec![40],
            gap_scope: GapScope::Local,
            gap: vec![
                GapSkill { skill: "Rust".into(), bucket: GapBucket::BuildOn, direction: Direction::Rising, share_permille: 125 },
                GapSkill { skill: "Kafka".into(), bucket: GapBucket::LearnNext, direction: Direction::Steady, share_permille: 500 },
                GapSkill { skill: "Py|thon".into(), bucket: GapBucket::AtRisk, direction: Direction::Falling, share_permille: 7 },
            ],
            wishes: vec![
                WishSignal { wish: "Kotlin".into(), kind: WishKind::Skill, avoided: false, direction: Direction::Steady, mentions: 6 },
                WishSignal { wish: "[gambling](http://evil.test)".into(), kind: WishKind::Domain, avoided: true, direction: Direction::Falling, mentions: 0 },
            ],
            uncatalogued: vec![TermCount { term: "mojo *now*".into(), items: 4 }],
            ..SkillCounts::default()
        },
        ..DigestData::default()
    };
    let md = cite::render_markdown(&Period::parse("2026-W46").unwrap(), &data);
    for expected in [
        "## Gap, sorted by the app\n\nSkills of the local postings of your field",
        "- Build on: Rust (rising, 12.5%)\n- Learn next: Kafka (steady, 50.0%)\n- At risk: Py\\|thon (falling, 0.7%)\n",
        "| Wish | Kind | Avoid? | Direction | Mentions |",
        "| Kotlin | skill | no | steady | 6 |",
        "| \\[gambling\\](http\\://evil.test) | domain | yes | no data | 0 |",
        "Terms not in the list (items naming them): mojo \\*now\\* (4)",
    ] {
        assert!(md.contains(expected), "{expected:?} missing in {md}");
    }
    assert!(!md.contains("](http://"), "{md}");
}

#[tokio::test]
async fn hiring_postings_only_fill_the_room_the_other_items_leave() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in_memory().unwrap();
    let now = Utc::now();
    // Five postings of one backfilled thread, the newest items of the period.
    let postings: Vec<i64> = (0..5)
        .map(|n| {
            let text = format!("# Hacker News, Ask HN: Who is hiring? (this month)\n\nAcme{n} | Rust");
            stored_item(&store, dir.path(), &format!("https://news.ycombinator.com/item?id={}", 100 + n), &format!("Acme{n} | Rust"), &text, Some(now))
        })
        .collect();
    // Three other items, older: an HN text post (an item URL too), a feed post, a page.
    let story = stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=7", "Ask HN: Is anyone hiring SREs?", "# Ask HN: Is anyone hiring SREs?\n\n12 points", Some(now - Span::hours(1)));
    let feed = stored_item(&store, dir.path(), "https://blog.test/1", "A post", "# A post\n\nWho is hiring? We are.", Some(now - Span::hours(2)));
    let page = stored_item(&store, dir.path(), "https://page.test/", "A page", "# A page", Some(now - Span::hours(3)));
    // A posting of an older thread, backfilled (fetched) now: it belongs to its own month.
    let old_text = "# Hacker News, Ask HN: Who is hiring? (two months ago)\n\nOldCo | Go";
    let old = stored_item(&store, dir.path(), "https://news.ycombinator.com/item?id=50", "OldCo | Go", old_text, Some(now - Span::days(60)));

    let ids = |items: Vec<TrendItem>| items.into_iter().map(|i| i.id).collect::<Vec<_>>();
    let picked = ids(period_items(&store, &period(), 4).await.unwrap());
    assert_eq!(picked.len(), 4);
    assert_eq!(picked[..3], [story, feed, page], "every other item first, newest first");
    assert!(postings.contains(&picked[3]), "one posting fills the room left");

    let all = ids(period_items(&store, &period(), 100).await.unwrap());
    assert_eq!(all.len(), 8, "with room, the postings are read too");
    assert!(!all.contains(&old), "a backfilled posting is not evidence for this period");
    assert_eq!(all[..3], [story, feed, page]);
    assert_eq!(ids(period_items(&store, &period(), 2).await.unwrap()), [story, feed], "no room: no posting");
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
