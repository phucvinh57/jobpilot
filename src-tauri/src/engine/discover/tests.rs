//! Graph tests of the runner: fake sources, `llm::fake`, a real SQLite file; no network.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{bail, Result};
use async_trait::async_trait;
use futures::FutureExt;
use serde_json::json;

use super::*;
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::llm::fake::FakeLlm;
use crate::engine::llm::Llm;
use crate::engine::prefs::Preferences;
use crate::engine::store::{LeadState, NewApplication, NewLead, NewSource, SourcePurpose};
use crate::engine::{Paths, Secrets};

const POSTING: &str = "We are hiring. You will build and run backend services in Rust, own their quality, and work with a small team.";

fn cand(host: &str, n: u32, company: &str, title: &str) -> Candidate {
    Candidate {
        url: format!("https://{host}/jobs/{n}"),
        title: title.into(),
        company: company.into(),
        location: "Remote".into(),
        snippet: "snippet".into(),
    }
}

/// A scripted source: pages of candidates, an optional failure, and its own posting texts.
struct FakeSource {
    pages: Vec<Vec<Candidate>>,
    /// `search` fails with this message while it is set.
    failure: Mutex<Option<String>>,
    /// `posting` returns `None` (so the runner's fetch hook is used) instead of text.
    no_text: bool,
    searched: AtomicUsize,
}

impl FakeSource {
    fn new(pages: Vec<Vec<Candidate>>) -> Arc<Self> {
        Arc::new(Self { pages, failure: Mutex::new(None), no_text: false, searched: AtomicUsize::new(0) })
    }

    fn failing(message: &str) -> Arc<Self> {
        let s = Self::new(vec![vec![]]);
        *s.failure.lock().unwrap() = Some(message.into());
        s
    }
}

#[async_trait]
impl JobSource for FakeSource {
    async fn search(&self, _prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        self.searched.fetch_add(1, Ordering::SeqCst);
        if let Some(m) = self.failure.lock().unwrap().clone() {
            bail!("{m}");
        }
        let at: usize = ctx.cursor.as_deref().map_or(0, |c| c.parse().unwrap());
        Ok(SearchPage {
            candidates: self.pages.get(at).cloned().unwrap_or_default(),
            next: (at + 1 < self.pages.len()).then(|| (at + 1).to_string()),
        })
    }

    async fn posting(&self, c: &Candidate) -> Result<Option<String>> {
        Ok((!self.no_text).then(|| format!("{}\n\n{POSTING}", c.title)))
    }
}

/// A data directory with a profile and a set of fakes; `deps` builds what a run needs.
struct Harness {
    dir: tempfile::TempDir,
    llm: Arc<FakeLlm>,
    /// source name -> adapter
    sources: Arc<Mutex<HashMap<String, Arc<FakeSource>>>>,
    /// While false, source "b" meets a login wall.
    solved: Arc<AtomicBool>,
    /// Sources named here meet a blocked page at every search.
    always_blocked: Arc<Mutex<Vec<String>>>,
    notices: Arc<Mutex<Vec<String>>>,
    insight_calls: Arc<AtomicUsize>,
    /// What the insight hook answers: `Ok(Some(json))`, `Ok(None)` or an error.
    insight: Arc<Mutex<Result<Option<String>, String>>>,
    fetched: Arc<Mutex<Vec<String>>>,
    fetch_answer: Arc<Mutex<Result<String, String>>>,
}

fn scores() -> serde_json::Value {
    json!({
        "skills": {"score": 80, "known": true, "reason": "matches"},
        "domain": {"score": 60, "known": true, "reason": "ok"},
        "location": {"score": 100, "known": true, "reason": "remote"},
        "salary": {"score": 0, "known": false, "reason": "not stated"},
        "seniority": {"score": 70, "known": true, "reason": "fits"}
    })
}

impl Harness {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self {
            dir,
            llm: Arc::new(FakeLlm::with_replies(std::iter::repeat_n(scores(), 200))),
            sources: Arc::default(),
            solved: Arc::new(AtomicBool::new(false)),
            always_blocked: Arc::default(),
            notices: Arc::default(),
            insight_calls: Arc::default(),
            insight: Arc::new(Mutex::new(Ok(None))),
            fetched: Arc::default(),
            fetch_answer: Arc::new(Mutex::new(Ok(POSTING.to_string()))),
        }
    }

    fn open(&self) -> Core {
        let paths = Paths::new(self.dir.path().join("jp"));
        let core = Core::open_at(paths, Secrets::memory()).unwrap();
        let profile = self.dir.path().join("profile.yaml");
        std::fs::write(&profile, "basics: {name: Ada, headline: Rust engineer}\nskills:\n  - {category: Languages, items: [Rust, Go]}\n").unwrap();
        core.set_profile_path(&profile).unwrap();
        core
    }

    fn add_source(&self, core: &Core, name: &str, adapter: Arc<FakeSource>) -> i64 {
        self.sources.lock().unwrap().insert(name.into(), adapter);
        core.store
            .add_source(&NewSource { kind: "fake".into(), name: name.into(), url: format!("https://{name}.test"), purpose: SourcePurpose::Jobs, session_id: None })
            .unwrap()
            .id
    }

    fn deps(&self) -> DiscoverDeps {
        let llm: Arc<dyn Llm> = self.llm.clone();
        let mut deps = DiscoverDeps::new(llm, DiscoverLimits { host_delay: std::time::Duration::ZERO, ..DiscoverLimits::default() }).expect("deps");
        let sources = self.sources.clone();
        deps.sources = Arc::new(move |source| {
            let found = sources.lock().unwrap().get(&source.name).cloned();
            found.map(|s| s as Arc<dyn JobSource>).ok_or_else(|| anyhow::anyhow!("no fake for {}", source.name))
        });
        let (solved, blocked) = (self.solved.clone(), self.always_blocked.clone());
        deps.challenge = Arc::new(move |check| {
            let (solved, blocked) = (solved.clone(), blocked.clone());
            async move {
                let name = check.source.name.clone();
                if check.stage != CheckStage::Search {
                    return Ok(None);
                }
                if blocked.lock().unwrap().contains(&name) {
                    return Ok(Some(ChallengeKind::Blocked));
                }
                Ok((name == "b" && !solved.load(Ordering::SeqCst)).then_some(ChallengeKind::Login))
            }
            .boxed()
        });
        let notices = self.notices.clone();
        deps.notify = Arc::new(move |title, body| notices.lock().unwrap().push(format!("{title}: {body}")));
        let (calls, answer) = (self.insight_calls.clone(), self.insight.clone());
        deps.insight = Arc::new(move |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            let answer = answer.lock().unwrap().clone();
            async move { answer.map_err(|e| anyhow::anyhow!(e)) }.boxed()
        });
        let (fetched, answer) = (self.fetched.clone(), self.fetch_answer.clone());
        deps.fetch = Arc::new(move |req| {
            fetched.lock().unwrap().push(req.candidate.url.clone());
            let answer = answer.lock().unwrap().clone();
            async move { answer.map_err(|e| anyhow::anyhow!(e)) }.boxed()
        });
        deps
    }
}

fn leads(core: &Core, state: LeadState) -> Vec<crate::engine::store::Lead> {
    core.store.list_leads(state).unwrap()
}

fn result_of(summary: &RunSummary, source_id: i64) -> &SourceResult {
    summary.sources.iter().find(|r| r.source_id == source_id).expect("the source is in the summary")
}

#[tokio::test]
async fn leads_are_saved_while_a_blocked_source_waits_and_resuming_it_finishes_the_run() {
    let h = Harness::new();
    let core = h.open();
    core.save_preferences(Preferences { exclude_companies: vec!["Evil".into()], ..Default::default() }).unwrap();
    // Already known: a lead (by URL) and an application (by company and role).
    core.store.add_lead(&NewLead { url: "https://c.test/jobs/1".into(), dedup_key: "x-y".into(), ..Default::default() }).unwrap();
    core.store
        .create_application(&NewApplication {
            slug: "globex-sre".into(),
            dir: "/tmp/globex-sre".into(),
            company: "Globex".into(),
            role: "Site Reliability Engineer".into(),
            ..Default::default()
        })
        .unwrap();

    let a = h.add_source(
        &core,
        "a",
        FakeSource::new(vec![vec![
            cand("a.test", 1, "Acme", "Backend Engineer"),
            cand("a.test", 2, "Evil Corp", "Backend Engineer"),
            cand("a.test", 3, "Initech", "Platform Engineer"),
            cand("a.test", 3, "Initech", "Platform Engineer (dup on the same page)"),
            Candidate { url: "javascript:alert(1)".into(), ..cand("a.test", 4, "Hax", "Hacker") },
        ]]),
    );
    let b = h.add_source(&core, "b", FakeSource::new(vec![vec![cand("b.test", 1, "Umbrella", "Data Engineer")]]));
    let c = h.add_source(
        &core,
        "c",
        FakeSource::new(vec![vec![
            cand("c.test", 1, "Known Inc", "Anything"),
            cand("c.test", 2, "Globex Corp", "Site Reliability Engineer"),
            cand("c.test", 3, "Hooli", "Rust Developer"),
        ]]),
    );

    let summary = core.discover_now_with(h.deps()).await.unwrap();

    // a: Acme + Initech saved, Evil dismissed. c: Hooli saved; the lead's URL and the application's job are known.
    assert_eq!(result_of(&summary, a).status, SourceStatus::Done);
    assert_eq!((result_of(&summary, a).saved, result_of(&summary, a).dismissed), (2, 1));
    assert_eq!(result_of(&summary, c).status, SourceStatus::Done);
    assert_eq!(result_of(&summary, c).saved, 1);
    assert_eq!(result_of(&summary, b).status, SourceStatus::Waiting);
    assert_eq!(result_of(&summary, b).note, "login");
    assert_eq!((summary.saved, summary.dismissed, summary.waiting, summary.failed), (3, 1, 1, 0));

    let new: Vec<_> = leads(&core, LeadState::New).into_iter().filter(|l| l.source_id.is_some()).collect();
    let mut names: Vec<_> = new.iter().map(|l| l.company.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, ["Acme", "Hooli", "Initech"]);
    let acme = new.iter().find(|l| l.company == "Acme").unwrap();
    assert_eq!(acme.score, Some(77.1), "6550 / 85: the salary was not judged, so its weight does not count");
    assert!(acme.score_json.as_deref().unwrap().contains("\"matches\""));
    assert_eq!(acme.source_id, Some(a));
    assert_eq!(acme.url, "https://a.test/jobs/1");
    let jd = std::fs::read_to_string(acme.jd_path.as_ref().unwrap()).unwrap();
    assert!(jd.starts_with("# Acme \u{2014} Backend Engineer\nSource: https://a.test/jobs/1\n\n"), "{jd}");
    assert!(jd.contains("Rust") && acme.jd_path.as_ref().unwrap().starts_with(lead_dir(&core.paths, acme.id)));
    let dismissed = leads(&core, LeadState::Dismissed);
    let evil = dismissed.iter().find(|l| l.company == "Evil Corp").unwrap();
    assert_eq!(evil.dismiss_reason.as_deref(), Some("excluded company: Evil"));
    assert_eq!(evil.score, None);
    assert_eq!(h.llm.calls().len(), 3, "one model call per scored lead, none for the dismissed or known ones");
    assert!(h.llm.calls()[0].user.contains("Headline: Rust engineer") && !h.llm.calls()[0].user.contains("Ada"));

    // The source's status and the summary are kept.
    assert_eq!(core.store.get_source(a).unwrap().unwrap().last_status.as_deref(), Some("ok"));
    assert_eq!(core.store.get_source(b).unwrap().unwrap().last_status.as_deref(), Some("needs_human"));
    assert!(core.store.get_source(a).unwrap().unwrap().last_run_at.is_some());
    assert_eq!(core.discover_last_run().unwrap().unwrap(), summary);
    assert_eq!(h.notices.lock().unwrap().len(), 1);
    assert!(h.notices.lock().unwrap()[0].contains("b: sign in"), "{:?}", h.notices);

    // One source waits at wait_human; the others are done.
    let waiting = core.discover_waiting().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!((waiting[0].source_id, waiting[0].source_name.as_str()), (b, "b"));
    assert_eq!(waiting[0].reason, WaitReason::Login { source_id: b, url: "https://b.test".into() });
    let thread = waiting[0].thread.clone();
    let mut before: Vec<_> = leads(&core, LeadState::New).iter().chain(&dismissed).map(|l| l.id).collect();
    before.sort_unstable();

    // "Restart": a new Core over the same files. Nothing is lost.
    drop(core);
    let core = h.open();
    let waiting = core.discover_waiting().unwrap();
    assert_eq!(waiting.len(), 1, "the paused source survives a restart");
    assert_eq!(waiting[0].thread, thread);
    let mut after: Vec<_> = leads(&core, LeadState::New).iter().chain(&leads(&core, LeadState::Dismissed)).map(|l| l.id).collect();
    after.sort_unstable();
    assert_eq!(after, before);

    // The user solved the login: Continue.
    h.solved.store(true, Ordering::SeqCst);
    let done = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!((done.status, done.saved), (SourceStatus::Done, 1));
    assert!(core.discover_resume_with(h.deps(), &thread).await.unwrap_err().to_string().contains("finished"));
    assert!(core.discover_waiting().unwrap().is_empty());
    assert!(leads(&core, LeadState::New).iter().any(|l| l.company == "Umbrella"));
    assert_eq!(core.store.get_source(b).unwrap().unwrap().last_status.as_deref(), Some("ok"));
    assert_eq!(h.llm.calls().len(), 4);
}

#[tokio::test]
async fn a_failing_source_ends_only_its_own_thread_and_retry_resumes_it() {
    let h = Harness::new();
    let core = h.open();
    let good = h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Backend Engineer")]]));
    let bad_adapter = FakeSource::failing("fetching boards.test: HTTP 503 Service Unavailable");
    let bad = h.add_source(&core, "d", bad_adapter.clone());

    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&summary, good).status, SourceStatus::Done);
    assert_eq!(result_of(&summary, bad).status, SourceStatus::Failed);
    assert!(result_of(&summary, bad).note.contains("503"), "{:?}", result_of(&summary, bad));
    assert_eq!((summary.saved, summary.failed), (1, 1));
    let status = core.store.get_source(bad).unwrap().unwrap().last_status.unwrap();
    assert!(status.starts_with("failed: ") && status.contains("503"), "{status}");

    // The failed thread is kept; once the site is back, Retry re-runs the failed node.
    let thread = source_thread(&summary.run, bad);
    assert!(matches!(core.graphs.status(&thread).unwrap().unwrap().status, crate::engine::graph::ThreadStatus::Failed(_)));
    *bad_adapter.failure.lock().unwrap() = None;
    let retried = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!(retried.status, SourceStatus::Done);
    assert_eq!(core.store.get_source(bad).unwrap().unwrap().last_status.as_deref(), Some("ok"));
}

#[tokio::test]
async fn a_source_without_an_adapter_fails_alone_with_a_clear_message() {
    let h = Harness::new();
    let core = h.open();
    let ok = h.add_source(&core, "a", FakeSource::new(vec![vec![]]));
    let yc = core
        .store
        .add_source(&NewSource { kind: "yc".into(), name: "yc".into(), url: String::new(), purpose: SourcePurpose::Jobs, session_id: None })
        .unwrap()
        .id;
    // The real factory, so an unknown kind is reported by the adapter table.
    let mut deps = h.deps();
    let fake = deps.sources.clone();
    let http = source::FakeHttp::with([]);
    deps.sources = Arc::new(move |s| if s.kind == "fake" { fake(s) } else { source::adapter_for(s, http.clone()) });
    let summary = core.discover_now_with(deps).await.unwrap();
    assert_eq!(result_of(&summary, ok).status, SourceStatus::Done);
    assert_eq!(result_of(&summary, yc).status, SourceStatus::Failed);
    assert!(result_of(&summary, yc).note.contains("no adapter for source kind `yc`"), "{:?}", result_of(&summary, yc));
}

#[tokio::test]
async fn three_challenges_in_a_row_disable_the_source_for_the_day() {
    let h = Harness::new();
    let core = h.open();
    let id = h.add_source(&core, "e", FakeSource::new(vec![vec![cand("e.test", 1, "Acme", "Engineer")]]));
    h.always_blocked.lock().unwrap().push("e".into());

    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);
    assert_eq!(result_of(&summary, id).note, "blocked");
    let thread = source_thread(&summary.run, id);

    // The user continues, and it is blocked again, twice more.
    let second = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!(second.status, SourceStatus::Waiting, "the second strike waits again");
    let third = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!(third.status, SourceStatus::Done, "the third strike ends the thread");
    assert_eq!(third.note, "paused for today after repeated challenges");
    assert!(core.discover_waiting().unwrap().is_empty());
    assert_eq!(h.notices.lock().unwrap().len(), 2, "no notification for the strike that ends it");
    let status = core.store.get_source(id).unwrap().unwrap().last_status.unwrap();
    assert!(status.starts_with("paused for today"), "{status}");
    assert!(leads(&core, LeadState::New).is_empty());

    // The next run leaves it alone today.
    let next = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&next, id).status, SourceStatus::NotRun);
    assert!(result_of(&next, id).note.contains("paused for today"));

    // Tomorrow it is tried again.
    core.store.kv_set(&format!("discover.paused.{id}"), "2000-01-01").unwrap();
    h.always_blocked.lock().unwrap().clear();
    let tomorrow = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!((result_of(&tomorrow, id).status, result_of(&tomorrow, id).saved), (SourceStatus::Done, 1));
}

#[tokio::test]
async fn a_challenge_resets_when_a_page_goes_through() {
    // "In a row": a page that went through in between starts the count again.
    let h = Harness::new();
    let core = h.open();
    let id = h.add_source(&core, "b", FakeSource::new(vec![vec![cand("b.test", 1, "Acme", "Engineer")], vec![cand("b.test", 2, "Initech", "Engineer")]]));
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);
    h.solved.store(true, Ordering::SeqCst);
    let thread = source_thread(&summary.run, id);
    let done = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!((done.status, done.saved), (SourceStatus::Done, 2));
    assert_eq!(leads(&core, LeadState::New).len(), 2);
}

#[tokio::test]
async fn cancelling_a_waiting_source_skips_it_and_keeps_the_others() {
    let h = Harness::new();
    let core = h.open();
    h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    let b = h.add_source(&core, "b", FakeSource::new(vec![vec![cand("b.test", 1, "Umbrella", "Engineer")]]));
    core.discover_now_with(h.deps()).await.unwrap();
    let waiting = core.discover_waiting().unwrap();
    assert_eq!(waiting.len(), 1);

    core.discover_cancel(&waiting[0].thread).unwrap();
    assert!(core.discover_waiting().unwrap().is_empty());
    assert_eq!(core.store.get_source(b).unwrap().unwrap().last_status.as_deref(), Some("cancelled"));
    assert_eq!(leads(&core, LeadState::New).len(), 1, "the finished source keeps its lead");
    assert!(core.discover_cancel("prefs").is_err(), "only discovery threads");
    assert!(core.discover_resume_with(h.deps(), "discover:1").await.is_err());
    let gone = core.discover_resume_with(h.deps(), &waiting[0].thread).await.unwrap_err();
    assert!(gone.to_string().contains("cancelled"), "{gone:#}");
    assert_eq!(
        core.store.get_source(b).unwrap().unwrap().last_status.as_deref(),
        Some("cancelled"),
        "a refused resume does not touch the source's status"
    );
}

#[tokio::test]
async fn pages_per_run_count_new_pages_and_the_next_run_goes_deeper() {
    let h = Harness::new();
    let core = h.open();
    let pages: Vec<Vec<Candidate>> =
        (0..5).map(|p| (0..2).map(|i| cand("p.test", p * 2 + i, &format!("Company{p}{i}"), "Engineer")).collect()).collect();
    let adapter = FakeSource::new(pages);
    let id = h.add_source(&core, "p", adapter.clone());
    let mut deps = h.deps();
    deps.limits.pages_per_run = 2;
    deps.limits.page_size = 2;

    let first = core.discover_now_with(deps.clone()).await.unwrap();
    assert_eq!(result_of(&first, id).saved, 4, "two pages of two");
    assert_eq!(adapter.searched.load(Ordering::SeqCst), 2);

    // The next run finds the first two pages known (free) and takes the next two new ones.
    let second = core.discover_now_with(deps.clone()).await.unwrap();
    assert_eq!(result_of(&second, id).saved, 4);
    assert_eq!(adapter.searched.load(Ordering::SeqCst), 2 + 4, "the known pages were looked at, not counted");
    let third = core.discover_now_with(deps).await.unwrap();
    assert_eq!(result_of(&third, id).saved, 2, "the last page");
    assert_eq!(leads(&core, LeadState::New).len(), 10);
}

#[tokio::test]
async fn a_page_with_a_cursor_that_never_ends_is_bounded() {
    struct Endless;
    #[async_trait]
    impl JobSource for Endless {
        async fn search(&self, _: &Preferences, _: &SearchCtx) -> Result<SearchPage> {
            Ok(SearchPage { candidates: vec![], next: Some("again".into()) })
        }
    }
    let h = Harness::new();
    let core = h.open();
    let id = core
        .store
        .add_source(&NewSource { kind: "x".into(), name: "x".into(), url: String::new(), purpose: SourcePurpose::Jobs, session_id: None })
        .unwrap()
        .id;
    let mut deps = h.deps();
    deps.sources = Arc::new(|_| Ok(Arc::new(Endless) as Arc<dyn JobSource>));
    deps.limits.pages_per_run = 2;
    let summary = core.discover_now_with(deps).await.unwrap();
    assert_eq!(result_of(&summary, id).status, SourceStatus::Done, "ends after pages_per_run * 10 empty pages");
}

#[tokio::test]
async fn postings_without_text_use_the_fetch_hook_and_unreadable_ones_are_skipped() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("f.test", 1, "Acme", "Engineer"), cand("f.test", 2, "Initech", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "f", adapter);

    // The page cannot be read (the default fetch says so for a JS app): skipped, and unknown, so a later run retries.
    *h.fetch_answer.lock().unwrap() = Err("https://f.test/jobs/1 needs a browser to render: only 12 characters of text".into());
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!((result_of(&summary, id).saved, result_of(&summary, id).skipped), (0, 2));
    assert_eq!(result_of(&summary, id).skip_kinds, BTreeMap::from([("unreadable".to_string(), 2)]), "kinds, not the error text");
    assert_eq!(h.fetched.lock().unwrap().len(), 2);
    assert!(leads(&core, LeadState::New).is_empty());
    assert!(!core.store.lead_exists("https://f.test/jobs/1", "").unwrap());

    // Now the hook returns text.
    *h.fetch_answer.lock().unwrap() = Ok(POSTING.to_string());
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&summary, id).saved, 2);
    let lead = &leads(&core, LeadState::New)[0];
    assert!(std::fs::read_to_string(lead.jd_path.as_ref().unwrap()).unwrap().contains("backend services"));

    // A text that is too short is a bot page, not a posting.
    *h.fetch_answer.lock().unwrap() = Ok("Just a moment...".to_string());
    h.sources.lock().unwrap().insert(
        "f".into(),
        Arc::new(FakeSource { pages: vec![vec![cand("f.test", 3, "Hooli", "Engineer")]], failure: Mutex::new(None), no_text: true, searched: AtomicUsize::new(0) }),
    );
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!((result_of(&summary, id).saved, result_of(&summary, id).skipped), (0, 1));
    assert_eq!(result_of(&summary, id).skip_kinds, BTreeMap::from([("too_short".to_string(), 1)]));
}

#[tokio::test]
async fn skipped_postings_are_counted_by_kind() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![
            cand("k.test", 1, "Acme", "Engineer"),
            cand("k.test", 2, "Initech", "Engineer"),
            cand("k.test", 3, "Hooli", "Engineer"),
            cand("k.test", 4, "Umbrella", "Engineer"),
        ]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "k", adapter);
    let mut deps = h.deps();
    deps.fetch = Arc::new(|req| {
        let error = match req.candidate.url.rsplit('/').next() {
            Some("1") => anyhow::Error::new(crate::engine::pipeline::ingest::NeedsBrowser { url: req.candidate.url.clone(), reason: "js".into() }),
            Some("2") => anyhow::anyhow!("fetching k.test: HTTP 404 Not Found"),
            Some("3") => anyhow::anyhow!("fetching k.test: HTTP 403 Forbidden"),
            _ => anyhow::anyhow!("the page has no text"),
        };
        async move { Err(error) }.boxed()
    });
    let summary = core.discover_now_with(deps).await.unwrap();
    let kinds = &result_of(&summary, id).skip_kinds;
    assert_eq!(
        *kinds,
        BTreeMap::from([
            ("needs_browser".to_string(), 1),
            ("gone".to_string(), 1),
            ("forbidden".to_string(), 1),
            ("unreadable".to_string(), 1)
        ])
    );
    assert_eq!(result_of(&summary, id).skipped, 4);
    let kept = core.discover_last_run().unwrap().unwrap();
    assert_eq!(kept.sources[0].skip_kinds, *kinds, "the kinds are part of the kept summary");
}

#[tokio::test]
async fn a_bot_wall_skips_the_posting_and_does_not_fail_the_thread() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("w.test", 1, "Acme", "Engineer"), cand("w.test", 2, "Initech", "Engineer"), cand("w.test", 3, "Hooli", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "w", adapter);
    let mut deps = h.deps();
    // 429 and 503 read as network trouble to `health::classify`, but here they are a wall.
    deps.fetch = Arc::new(|req| {
        let code = if req.candidate.url.ends_with("/1") { 429 } else if req.candidate.url.ends_with("/2") { 503 } else { 403 };
        let wall = crate::engine::pipeline::ingest::NeedsBrowser {
            url: req.candidate.url.clone(),
            reason: format!("the server answered HTTP {code} (bot check or login wall)"),
        };
        async move { Err(anyhow::Error::new(wall)) }.boxed()
    });
    let summary = core.discover_now_with(deps).await.unwrap();
    let result = result_of(&summary, id);
    assert_eq!(result.status, SourceStatus::Done, "{}", result.note);
    assert_eq!((result.saved, result.skipped), (0, 3));
    assert_eq!(result.skip_kinds, BTreeMap::from([("needs_browser".to_string(), 3)]));
    assert!(!result.note.contains("w.test"), "{}", result.note);
}

#[tokio::test]
async fn a_failed_fetch_never_carries_the_posting_address_into_the_failure() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("f.test", 1, "Acme", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "f", adapter);
    let mut deps = h.deps();
    // A fetch hook whose network error repeats the address (with a token in the query).
    deps.fetch = Arc::new(|req| {
        let error = anyhow::anyhow!("error sending request for url ({}?token=tok-0123456789): connection refused", req.candidate.url);
        async move { Err(error) }.boxed()
    });
    let summary = core.discover_now_with(deps).await.unwrap();
    let result = result_of(&summary, id);
    assert_eq!(result.status, SourceStatus::Failed);
    assert!(result.note.contains("connection refused"), "the cause is kept: {}", result.note);
    assert!(!result.note.contains("f.test/jobs/1") && !result.note.contains("tok-0123"), "{}", result.note);
    let stored = core.graphs.status(&source_thread(&summary.run, id)).unwrap().expect("the thread exists");
    let stored = format!("{:?}", stored.status);
    assert!(!stored.contains("f.test/jobs/1") && !stored.contains("tok-0123"), "{stored}");
}

#[tokio::test]
async fn a_posting_whose_session_is_in_use_is_skipped_by_kind_and_read_again_next_run() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("f.test", 1, "Acme", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "f", adapter);
    // What the browser read says while the user's window is open on the session.
    *h.fetch_answer.lock().unwrap() = Err(crate::engine::browser::gate::SessionInUse::Window.to_string());
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    let result = result_of(&summary, id);
    assert_eq!((result.status, result.saved, result.skipped), (SourceStatus::Done, 0, 1), "the source did not fail");
    assert_eq!(result.skip_kinds, BTreeMap::from([("session_in_use".to_string(), 1)]));
    assert!(!core.store.lead_exists("https://f.test/jobs/1", "").unwrap(), "unknown, so the next run reads it");
    assert_eq!(super::run::skip_kind(&anyhow::Error::new(crate::engine::browser::gate::SessionInUse::Busy)), "session_in_use");
}

#[test]
fn needs_browser_loses_its_address_before_it_travels() {
    use crate::engine::pipeline::ingest::NeedsBrowser;
    let url = "https://acme.test/jobs/7?utm=abc";
    let wall = anyhow::Error::new(NeedsBrowser { url: format!("{url}#x"), reason: format!("{url} answered HTTP 403 (bot check or login wall)") });
    let cleaned = run::without_url(wall.context("reading the page"), url);
    let text = format!("{cleaned:#}");
    assert!(text.contains("HTTP 403") && !text.contains("acme.test"), "{text}");
    assert!(cleaned.downcast_ref::<run::PageNeedsBrowser>().is_some(), "still recognisable as a wall");
    // Any other error loses the address too; one without it is returned as it was.
    let plain = run::without_url(anyhow::anyhow!("GET {url} failed: reset"), url);
    assert_eq!(plain.to_string(), "GET <posting> failed: reset");
    let other = run::without_url(anyhow::anyhow!("HTTP 404"), url);
    assert_eq!(other.to_string(), "HTTP 404");
}

#[tokio::test]
async fn a_posting_found_by_two_sources_is_counted_as_a_duplicate() {
    /// Holds each source's posting fetch until both have got there, so both pages went through
    /// dedup before either lead is saved.
    struct Meet(Arc<FakeSource>, Arc<tokio::sync::Barrier>);
    #[async_trait]
    impl JobSource for Meet {
        async fn search(&self, prefs: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
            self.0.search(prefs, ctx).await
        }
        async fn posting(&self, c: &Candidate) -> Result<Option<String>> {
            self.1.wait().await;
            self.0.posting(c).await
        }
    }
    let h = Harness::new();
    let core = h.open();
    let barrier = Arc::new(tokio::sync::Barrier::new(2));
    let mut meeting: HashMap<String, Arc<Meet>> = HashMap::new();
    let mut ids = Vec::new();
    for (name, host) in [("a", "a.test"), ("c", "c.test")] {
        let src = FakeSource::new(vec![vec![cand(host, 1, "Acme Inc", "Backend Engineer")]]);
        meeting.insert(name.into(), Arc::new(Meet(src.clone(), barrier.clone())));
        ids.push(h.add_source(&core, name, src));
    }
    let mut deps = h.deps();
    deps.sources = Arc::new(move |source| {
        meeting.get(&source.name).map(|m| m.clone() as Arc<dyn JobSource>).ok_or_else(|| anyhow::anyhow!("no fake"))
    });
    let summary = core.discover_now_with(deps).await.unwrap();
    assert_eq!(summary.saved, 1, "one lead for the job");
    assert_eq!(summary.skipped, 1);
    let skipped_by: Vec<_> = ids.iter().filter_map(|id| result_of(&summary, *id).skip_kinds.get("duplicate").copied()).collect();
    assert_eq!(skipped_by, [1], "the later save found its twin");
}

#[tokio::test]
async fn challenges_with_a_clean_check_in_between_are_not_in_a_row() {
    // Postings 1, 3, 5 meet a login wall (and pass when fetched again after Continue): three
    // challenges, but never three in a row, so the source is not given up.
    let h = Harness::new();
    let core = h.open();
    let page: Vec<_> = (1..=5).map(|n| cand("r.test", n, &format!("Company{n}"), "Engineer")).collect();
    let id = h.add_source(&core, "r", FakeSource::new(vec![page]));
    let checks = Arc::new(AtomicUsize::new(0));
    let mut deps = h.deps();
    deps.challenge = {
        let checks = checks.clone();
        Arc::new(move |check| {
            let checks = checks.clone();
            async move {
                Ok(match check.stage {
                    CheckStage::Search => None,
                    // Posting checks 0, 2, 4 find a wall; 1, 3, 5, 6 are clean.
                    CheckStage::Posting => {
                        let n = checks.fetch_add(1, Ordering::SeqCst);
                        (n.is_multiple_of(2) && n < 5).then_some(ChallengeKind::Login)
                    }
                })
            }
            .boxed()
        })
    };
    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    let thread = source_thread(&summary.run, id);
    let mut last = result_of(&summary, id).clone();
    let mut continues = 0;
    while last.status == SourceStatus::Waiting {
        continues += 1;
        assert!(continues <= 5, "it should not keep waiting");
        last = core.discover_resume_with(deps.clone(), &thread).await.unwrap();
    }
    assert_eq!(continues, 3, "three walls, each continued");
    assert_eq!((last.status, last.saved), (SourceStatus::Done, 5), "{last:?}");
    assert_eq!(last.note, "", "not paused for the day");
    assert!(core.store.kv_get(&format!("discover.paused.{id}")).unwrap().is_none());
}

#[test]
fn a_failed_source_note_never_carries_a_secret() {
    let h = Harness::new();
    let core = h.open();
    let id = h.add_source(&core, "a", FakeSource::new(vec![vec![]]));
    let message = "boom: Authorization: Bearer abcdefghijklmnop1234 and key sk-abcdefghijklmnopqrstuvwxyz0123";
    let failure = crate::engine::graph::Failure { problem: crate::engine::health::Problem::Other, node: "search".into(), message: message.into() };
    let result = run::source_result(&core.store, "discover:1:1", id, Ok(Outcome::Failed(failure)));
    let status = core.store.get_source(id).unwrap().unwrap().last_status.unwrap();
    for text in [&result.note, &status] {
        assert!(text.contains("boom"), "{text}");
        assert!(!text.contains("abcdefghijklmnop1234") && !text.contains("sk-abcdefghijklmnop"), "{text}");
    }
}

#[tokio::test]
async fn a_network_failure_while_fetching_fails_the_thread_and_retry_goes_on() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("f.test", 1, "Acme", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "f", adapter);
    *h.fetch_answer.lock().unwrap() = Err("fetching f.test: error sending request for url".into());
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&summary, id).status, SourceStatus::Failed);

    *h.fetch_answer.lock().unwrap() = Ok(POSTING.to_string());
    let thread = source_thread(&summary.run, id);
    let retried = core.discover_resume_with(h.deps(), &thread).await.unwrap();
    assert_eq!((retried.status, retried.saved), (SourceStatus::Done, 1), "the failed posting is fetched again");
}

#[tokio::test]
async fn insight_comes_from_the_hook_and_never_blocks_a_lead() {
    let h = Harness::new();
    let core = h.open();
    h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    *h.insight.lock().unwrap() = Ok(Some(r#"{"domain": "robotics"}"#.into()));
    core.discover_now_with(h.deps()).await.unwrap();
    let lead = &leads(&core, LeadState::New)[0];
    assert_eq!(lead.insight_json.as_deref(), Some(r#"{"domain": "robotics"}"#));
    assert_eq!(h.insight_calls.load(Ordering::SeqCst), 1);

    // An insight that is not JSON is dropped; one that fails leaves the lead without it.
    for (n, answer) in [Ok(Some("not json".to_string())), Err("pages unreachable".to_string()), Ok(None)].into_iter().enumerate() {
        *h.insight.lock().unwrap() = answer;
        h.sources.lock().unwrap().insert("a".into(), FakeSource::new(vec![vec![cand("a.test", 10 + n as u32, &format!("Co{n}"), "Engineer")]]));
        let summary = core.discover_now_with(h.deps()).await.unwrap();
        assert_eq!(summary.saved, 1, "the lead is saved anyway");
    }
    let without: Vec<_> = leads(&core, LeadState::New).into_iter().filter(|l| l.insight_json.is_none()).collect();
    assert_eq!(without.len(), 3);
    assert!(without.iter().all(|l| l.score.is_some()), "and scored");
}

#[tokio::test]
async fn insight_is_not_asked_for_a_posting_the_filters_dropped() {
    let h = Harness::new();
    let core = h.open();
    core.save_preferences(Preferences { exclude_companies: vec!["Acme".into()], ..Default::default() }).unwrap();
    h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!((summary.saved, summary.dismissed), (0, 1));
    assert_eq!(h.insight_calls.load(Ordering::SeqCst), 0);
    assert!(h.llm.calls().is_empty());
}

#[tokio::test]
async fn no_provider_and_no_profile_are_setup_problems_not_crashes() {
    let h = Harness::new();
    let core = h.open();
    let err = core.discover_now().await.unwrap_err();
    assert_eq!(crate::engine::health::classify(&format!("{err:#}")), crate::engine::health::Problem::NoProvider, "{err:#}");

    let dir = tempfile::tempdir().unwrap();
    let bare = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
    bare.set_profile_path(&dir.path().join("missing.yaml")).unwrap();
    let err = bare.discover_now_with(h.deps()).await.unwrap_err();
    assert!(format!("{err:#}").contains("cannot read the profile"), "{err:#}");
}

#[tokio::test]
async fn disabled_sources_and_trend_sources_are_not_searched() {
    let h = Harness::new();
    let core = h.open();
    let off = h.add_source(&core, "off", FakeSource::new(vec![vec![cand("o.test", 1, "Acme", "Engineer")]]));
    core.store.set_source_enabled(off, false).unwrap();
    core.store
        .add_source(&NewSource { kind: "rss".into(), name: "trends".into(), url: String::new(), purpose: SourcePurpose::Trends, session_id: None })
        .unwrap();
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    assert!(summary.sources.is_empty());
    assert_eq!(core.discover_last_run().unwrap().unwrap().saved, 0);
}

#[tokio::test]
async fn the_run_reports_progress_and_runs_on_the_global_runtime() {
    let h = Harness::new();
    let core = Arc::new(h.open());
    h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    let mut progress = core.graphs.subscribe();
    let deps = h.deps();
    let rx = {
        let core = core.clone();
        crate::engine::runtime::spawn(async move { core.discover_now_with(deps).await })
    };
    let summary = rx.await.unwrap().unwrap();
    assert_eq!(summary.saved, 1);
    let mut graphs_done = Vec::new();
    while let Ok(p) = progress.try_recv() {
        if p.event == crate::engine::graph::ProgressEvent::Done {
            graphs_done.push(p.graph);
        }
    }
    graphs_done.sort();
    assert_eq!(graphs_done, [DISCOVERY_GRAPH, SOURCE_GRAPH]);
}

#[tokio::test]
async fn a_challenge_after_a_fetch_continues_with_the_same_posting_and_a_waiting_source_reports_what_it_saved() {
    let h = Harness::new();
    let core = h.open();
    let pages = vec![vec![cand("m.test", 1, "Acme", "Engineer")], vec![cand("m.test", 2, "Initech", "Engineer")]];
    let id = h.add_source(&core, "m", FakeSource::new(pages));
    let (searches, postings) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let hook = |searches: Arc<AtomicUsize>, postings: Arc<AtomicUsize>| -> ChallengeProbe {
        Arc::new(move |check| {
            let (searches, postings) = (searches.clone(), postings.clone());
            async move {
                Ok(match check.stage {
                    // The second search page meets a captcha, once.
                    CheckStage::Search => (searches.fetch_add(1, Ordering::SeqCst) == 1).then_some(ChallengeKind::Captcha),
                    // The first posting meets a login wall, once.
                    CheckStage::Posting => {
                        assert!(check.text.contains("backend services"), "the check sees the fetched text");
                        (postings.fetch_add(1, Ordering::SeqCst) == 0).then_some(ChallengeKind::Login)
                    }
                })
            }
            .boxed()
        })
    };
    let mut deps = h.deps();
    deps.challenge = hook(searches.clone(), postings.clone());

    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    let thread = source_thread(&summary.run, id);
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);
    assert_eq!(result_of(&summary, id).note, "login", "the first posting hit the login wall");
    assert_eq!(core.discover_waiting().unwrap()[0].reason, WaitReason::Login { source_id: id, url: "https://m.test/jobs/1".into() });
    assert!(leads(&core, LeadState::New).is_empty());

    // Continue: the same posting is fetched again; then the second page meets the captcha.
    let second = core.discover_resume_with(deps.clone(), &thread).await.unwrap();
    assert_eq!(second.status, SourceStatus::Waiting);
    assert_eq!(second.note, "captcha");
    assert_eq!(second.saved, 1, "what the source stored before it paused is reported");
    assert_eq!(leads(&core, LeadState::New).len(), 1);

    let done = core.discover_resume_with(deps, &thread).await.unwrap();
    assert_eq!((done.status, done.saved), (SourceStatus::Done, 2));
    assert_eq!(h.llm.calls().len(), 2, "each posting is scored once, though the first was fetched twice");
}

#[tokio::test]
async fn a_source_still_waiting_from_an_earlier_run_is_not_started_again() {
    let h = Harness::new();
    let core = h.open();
    let b = h.add_source(&core, "b", FakeSource::new(vec![vec![cand("b.test", 1, "Umbrella", "Engineer")]]));
    let first = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&first, b).status, SourceStatus::Waiting);
    let second = core.discover_now_with(h.deps()).await.unwrap();
    assert_eq!(result_of(&second, b).status, SourceStatus::NotRun);
    assert!(result_of(&second, b).note.contains("still waiting"), "{:?}", result_of(&second, b));
    assert_eq!(core.discover_waiting().unwrap().len(), 1, "one paused thread, not two");
}

#[tokio::test]
async fn postings_of_one_host_are_fetched_one_delay_apart_by_the_default_fetch() {
    use crate::engine::net::fixtures::Fixtures;
    let fx = Fixtures::start().await;
    let page = format!(
        "<!doctype html><html><head><title>Engineer</title></head><body><main><h1>Engineer</h1><p>{}</p></main></body></html>",
        "You will build and run backend services in Rust and own their quality. ".repeat(10)
    );
    let candidates: Vec<Candidate> = (1..=3)
        .map(|n| {
            fx.route(&format!("/jobs/{n}"), 200, "text/html", page.clone());
            Candidate { url: format!("{}/jobs/{n}", fx.base()), title: "Engineer".into(), company: format!("Co{n}"), location: "Remote".into(), snippet: String::new() }
        })
        .collect();
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource { pages: vec![candidates], failure: Mutex::new(None), no_text: true, searched: AtomicUsize::new(0) });
    h.add_source(&core, "f", adapter);
    let mut deps = h.deps();
    // The real fetch hook, with a delay between requests to one host and the local fixture allowed.
    let limits = DiscoverLimits { host_delay: std::time::Duration::from_millis(60), allow_private_hosts: true, ..deps.limits };
    deps.fetch = DiscoverDeps::new(deps.llm.clone(), limits).unwrap().fetch;
    let started = std::time::Instant::now();
    core.discover_now_with(deps).await.unwrap();
    assert_eq!(fx.hits().len(), 3, "each posting was fetched through the shared client");
    assert!(started.elapsed() >= std::time::Duration::from_millis(110), "two delays for three fetches: {:?}", started.elapsed());
}

#[tokio::test]
async fn the_default_fetch_never_reads_a_private_address() {
    use crate::engine::net::fixtures::Fixtures;
    let fx = Fixtures::start().await;
    fx.route("/jobs/1", 200, "text/html", "<html><body>secret</body></html>");
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![Candidate { url: format!("{}/jobs/1", fx.base()), title: "Engineer".into(), company: "Acme".into(), ..Default::default() }]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    h.add_source(&core, "f", adapter);
    let mut deps = h.deps();
    // The default fetch with the default (strict) limits: the fixture is on 127.0.0.1.
    deps.fetch = DiscoverDeps::new(deps.llm.clone(), DiscoverLimits { host_delay: std::time::Duration::ZERO, ..DiscoverLimits::default() }).unwrap().fetch;
    let summary = core.discover_now_with(deps).await.unwrap();
    assert!(fx.hits().is_empty(), "a posting link to a private address was never requested");
    assert_eq!((summary.saved, summary.skipped), (0, 1));
    assert_eq!(summary.sources[0].skip_kinds.get("unreadable"), Some(&1));
    assert!(!core.store.lead_exists(&format!("{}/jobs/1", fx.base()), "").unwrap());
}

#[tokio::test]
async fn at_most_sources_in_parallel_sources_run_at_once() {
    struct Slow {
        active: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
    }
    #[async_trait]
    impl JobSource for Slow {
        async fn search(&self, _: &Preferences, _: &SearchCtx) -> Result<SearchPage> {
            let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(SearchPage::default())
        }
    }
    let h = Harness::new();
    let core = h.open();
    for n in 0..5 {
        core.store
            .add_source(&NewSource { kind: "slow".into(), name: format!("s{n}"), url: String::new(), purpose: SourcePurpose::Jobs, session_id: None })
            .unwrap();
    }
    let (active, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    let mut deps = h.deps();
    deps.limits.sources_in_parallel = 2;
    let shared = Arc::new(Slow { active, peak: peak.clone() });
    deps.sources = Arc::new(move |_| Ok(shared.clone() as Arc<dyn JobSource>));
    let summary = core.discover_now_with(deps).await.unwrap();
    assert_eq!(summary.sources.len(), 5);
    assert_eq!(peak.load(Ordering::SeqCst), 2);
}

// ---------------------------------------------------------------- challenges met in the browser (part B)

use crate::engine::browser::challenge::ChallengeFound;
use crate::engine::health::{Challenge, Problem};

/// A source whose search meets a challenge in the browser until the user has dealt with it.
struct Walled {
    /// The page the challenge is at.
    url: String,
    candidates: Vec<Candidate>,
    kind: ChallengeKind,
    solved: Arc<AtomicBool>,
    cursors: Mutex<Vec<Option<String>>>,
}

impl Walled {
    fn new(kind: ChallengeKind, candidates: Vec<Candidate>) -> Arc<Self> {
        Self::at("https://w.test/careers", kind, candidates)
    }

    fn at(url: &str, kind: ChallengeKind, candidates: Vec<Candidate>) -> Arc<Self> {
        Arc::new(Self { url: url.into(), candidates, kind, solved: Arc::new(AtomicBool::new(false)), cursors: Mutex::default() })
    }
}

#[async_trait]
impl JobSource for Walled {
    async fn search(&self, _: &Preferences, ctx: &SearchCtx) -> Result<SearchPage> {
        self.cursors.lock().unwrap().push(ctx.cursor.clone());
        if !self.solved.load(Ordering::SeqCst) {
            // As the real adapters do: the error carries the challenge under some context.
            return Err(anyhow::Error::new(ChallengeFound { kind: self.kind, url: self.url.clone() }).context("reading the listing"));
        }
        Ok(SearchPage { candidates: self.candidates.clone(), next: None })
    }

    async fn posting(&self, c: &Candidate) -> Result<Option<String>> {
        Ok(Some(format!("{}\n\n{POSTING}", c.title)))
    }
}

fn add_walled(core: &Core, deps: &mut DiscoverDeps, source: Arc<Walled>) -> i64 {
    let id = core
        .store
        .add_source(&NewSource { kind: "walled".into(), name: "w".into(), url: source.url.clone(), purpose: SourcePurpose::Jobs, session_id: None })
        .unwrap()
        .id;
    deps.sources = Arc::new(move |_| Ok(source.clone() as Arc<dyn JobSource>));
    id
}

#[tokio::test]
async fn a_challenge_an_adapter_met_in_the_browser_pauses_the_search_and_continue_searches_the_same_page() {
    let h = Harness::new();
    let core = h.open();
    let source = Walled::new(ChallengeKind::Login, vec![cand("w.test", 1, "Acme", "Engineer")]);
    let mut deps = h.deps();
    let id = add_walled(&core, &mut deps, source.clone());

    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);
    assert_eq!(result_of(&summary, id).note, "login");
    let waiting = core.discover_waiting().unwrap();
    assert_eq!(waiting[0].reason, WaitReason::Login { source_id: id, url: "https://w.test/careers".into() });
    assert_eq!(crate::engine::discover::needs_human(&waiting[0].reason), Some(Problem::NeedsHuman(Challenge::Login)));
    assert_eq!(core.store.get_source(id).unwrap().unwrap().last_status.as_deref(), Some("needs_human"));
    assert_eq!(h.notices.lock().unwrap().len(), 1);
    assert!(h.notices.lock().unwrap()[0].contains("w: sign in"), "{:?}", h.notices);
    assert!(leads(&core, LeadState::New).is_empty());

    // "Continue" once the user has logged in: the same page is searched again.
    source.solved.store(true, Ordering::SeqCst);
    let done = core.discover_resume_with(deps, &waiting[0].thread).await.unwrap();
    assert_eq!((done.status, done.saved), (SourceStatus::Done, 1));
    assert_eq!(*source.cursors.lock().unwrap(), [None, None], "the first page twice, not the second");
    assert_eq!(core.store.get_source(id).unwrap().unwrap().last_status.as_deref(), Some("ok"));
}

#[tokio::test]
async fn three_challenges_met_in_the_browser_in_a_row_pause_the_source_for_the_day() {
    let h = Harness::new();
    let core = h.open();
    let source = Walled::new(ChallengeKind::Captcha, vec![cand("w.test", 1, "Acme", "Engineer")]);
    let mut deps = h.deps();
    let id = add_walled(&core, &mut deps, source.clone());

    let first = core.discover_now_with(deps.clone()).await.unwrap();
    let thread = source_thread(&first.run, id);
    assert_eq!(core.discover_resume_with(deps.clone(), &thread).await.unwrap().status, SourceStatus::Waiting);
    let third = core.discover_resume_with(deps.clone(), &thread).await.unwrap();
    assert_eq!((third.status, third.note.as_str()), (SourceStatus::Done, "paused for today after repeated challenges"));
    assert_eq!(h.notices.lock().unwrap().len(), 2);
    let next = core.discover_now_with(deps).await.unwrap();
    assert_eq!(result_of(&next, id).status, SourceStatus::NotRun);
    assert_eq!(source.cursors.lock().unwrap().len(), 3, "no fourth attempt today");
}

#[tokio::test]
async fn a_challenge_the_fetch_met_in_the_browser_pauses_at_that_posting_and_continue_fetches_it_again() {
    let h = Harness::new();
    let core = h.open();
    let adapter = Arc::new(FakeSource {
        pages: vec![vec![cand("f.test", 1, "Acme", "Engineer"), cand("f.test", 2, "Initech", "Engineer")]],
        failure: Mutex::new(None),
        no_text: true,
        searched: AtomicUsize::new(0),
    });
    let id = h.add_source(&core, "f", adapter);
    let solved = Arc::new(AtomicBool::new(false));
    let fetched = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut deps = h.deps();
    let (solved_in, fetched_in) = (solved.clone(), fetched.clone());
    deps.fetch = Arc::new(move |req| {
        let url = req.candidate.url.clone();
        fetched_in.lock().unwrap().push(url.clone());
        let blocked = !solved_in.load(Ordering::SeqCst) && url.ends_with("/jobs/1");
        async move {
            if blocked {
                // The error travels under context, like a real fetch's.
                return Err(anyhow::Error::new(ChallengeFound { kind: ChallengeKind::Captcha, url }).context("rendering the posting"));
            }
            Ok(POSTING.to_string())
        }
        .boxed()
    });

    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    assert_eq!((result_of(&summary, id).status, result_of(&summary, id).note.as_str()), (SourceStatus::Waiting, "captcha"));
    assert_eq!(result_of(&summary, id).skipped, 0, "the posting is not skipped");
    let waiting = core.discover_waiting().unwrap();
    assert_eq!(waiting[0].reason, WaitReason::Captcha { source_id: id, url: "https://f.test/jobs/1".into() });
    assert_eq!(crate::engine::discover::needs_human(&waiting[0].reason), Some(Problem::NeedsHuman(Challenge::Captcha)));
    assert!(h.notices.lock().unwrap()[0].contains("f: solve the captcha"), "{:?}", h.notices);
    assert_eq!(*fetched.lock().unwrap(), ["https://f.test/jobs/1"], "the second posting waits behind the first");
    assert!(leads(&core, LeadState::New).is_empty());

    solved.store(true, Ordering::SeqCst);
    let done = core.discover_resume_with(deps, &waiting[0].thread).await.unwrap();
    assert_eq!((done.status, done.saved, done.skipped), (SourceStatus::Done, 2, 0));
    assert_eq!(*fetched.lock().unwrap(), ["https://f.test/jobs/1", "https://f.test/jobs/1", "https://f.test/jobs/2"], "the same posting, then the next");
}

#[tokio::test]
async fn the_default_probe_catches_a_block_page_served_as_a_posting() {
    struct Interstitial {
        solved: Arc<AtomicBool>,
    }
    #[async_trait]
    impl JobSource for Interstitial {
        async fn search(&self, _: &Preferences, _: &SearchCtx) -> Result<SearchPage> {
            Ok(SearchPage { candidates: vec![cand("i.test", 1, "Acme", "Engineer")], next: None })
        }
        async fn posting(&self, _: &Candidate) -> Result<Option<String>> {
            Ok(Some(if self.solved.load(Ordering::SeqCst) {
                POSTING.to_string()
            } else {
                "Just a moment... Enable JavaScript and cookies to continue".to_string()
            }))
        }
    }
    let h = Harness::new();
    let core = h.open();
    let solved = Arc::new(AtomicBool::new(false));
    let shared = Arc::new(Interstitial { solved: solved.clone() });
    let id = core
        .store
        .add_source(&NewSource { kind: "i".into(), name: "i".into(), url: "https://i.test".into(), purpose: SourcePurpose::Jobs, session_id: None })
        .unwrap()
        .id;
    let mut deps = h.deps();
    deps.sources = Arc::new(move |_| Ok(shared.clone() as Arc<dyn JobSource>));
    deps.challenge = text_probe();

    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    assert_eq!((result_of(&summary, id).status, result_of(&summary, id).note.as_str()), (SourceStatus::Waiting, "captcha"));
    assert_eq!(core.discover_waiting().unwrap()[0].reason, WaitReason::Captcha { source_id: id, url: "https://i.test/jobs/1".into() });
    assert!(leads(&core, LeadState::New).is_empty(), "the interstitial was not scored as a posting");
    assert_eq!(h.llm.calls().len(), 0);

    solved.store(true, Ordering::SeqCst);
    let done = core.discover_resume_with(deps, &core.discover_waiting().unwrap()[0].thread).await.unwrap();
    assert_eq!((done.status, done.saved), (SourceStatus::Done, 1));
}

#[tokio::test]
async fn open_browser_only_works_for_a_source_that_waits_for_the_user() {
    let h = Harness::new();
    let core = h.open();
    let a = h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    let summary = core.discover_now_with(h.deps()).await.unwrap();
    // A finished source, a thread that is not a source's, and one that does not exist.
    for thread in [source_thread(&summary.run, a), discovery_thread(&summary.run), "prefs".to_string(), "discover:1:9".to_string()] {
        assert!(core.discover_open_browser(&thread).await.is_err(), "{thread}");
    }
    let err = core.discover_open_browser(&source_thread(&summary.run, a)).await.unwrap_err();
    assert!(err.to_string().contains("not waiting"), "{err:#}");
}

#[tokio::test]
async fn open_browser_refuses_a_page_on_this_machine_or_a_private_network() {
    let h = Harness::new();
    let core = h.open();
    let source = Walled::at("http://127.0.0.1:9/wall", ChallengeKind::Login, vec![cand("w.test", 1, "Acme", "Engineer")]);
    let mut deps = h.deps();
    let id = add_walled(&core, &mut deps, source);
    let summary = core.discover_now_with(deps).await.unwrap();
    let thread = source_thread(&summary.run, id);
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);
    let err = format!("{:#}", core.discover_open_browser(&thread).await.unwrap_err());
    assert!(err.contains("private network"), "{err}");
    assert!(!super::handoff::has_window(&core.paths, &thread), "no browser was started");
    // Still waiting for the user: Skip works.
    core.discover_cancel(&thread).unwrap();
}

/// Wait (up to 20 s) until a window has asked the fixture server for `path`, then a moment longer, so
/// a second window would have asked too.
async fn wait_for_hits(fx: &crate::engine::net::fixtures::Fixtures, path: &str) {
    for _ in 0..100 {
        if !fx.hits_on(path).is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

/// The browser opens on the page the run paused at; "Continue" closes it. Needs a display.
#[tokio::test]
#[ignore = "opens a visible Chrome window"]
async fn open_browser_shows_the_page_and_continue_closes_the_window() {
    use crate::engine::net::fixtures::Fixtures;
    let fx = Fixtures::start().await;
    fx.route("/wall", 200, "text/html", "<html><head><title>Sign in</title></head><body><h1>Sign in to continue</h1></body></html>");
    let h = Harness::new();
    let core = h.open();
    let source = Walled::at(&format!("{}/wall", fx.base()), ChallengeKind::Login, vec![cand("w.test", 1, "Acme", "Engineer")]);
    let mut deps = h.deps();
    let id = add_walled(&core, &mut deps, source.clone());
    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    let thread = source_thread(&summary.run, id);
    assert_eq!(result_of(&summary, id).status, SourceStatus::Waiting);

    // No session is bound: a throwaway visible window. A double click (two calls at once) opens one
    // window, and opening again while it is open does nothing.
    let (first, second) = tokio::join!(super::handoff::open_browser(&core, &thread, true), super::handoff::open_browser(&core, &thread, true));
    first.unwrap();
    second.unwrap();
    super::handoff::open_browser(&core, &thread, true).await.unwrap();
    assert!(super::handoff::has_window(&core.paths, &thread));
    wait_for_hits(&fx, "/wall").await;
    assert_eq!(fx.hits_on("/wall").len(), 1, "one window loaded the page the run paused at");

    // Continue closes the window and goes on.
    source.solved.store(true, Ordering::SeqCst);
    let done = core.discover_resume_with(deps, &thread).await.unwrap();
    assert_eq!(done.status, SourceStatus::Done);
    assert!(!super::handoff::has_window(&core.paths, &thread));
}

/// With a bound session the window is that session's profile: it is locked while the window is open
/// and free again after "Continue". Needs a display.
#[tokio::test]
#[ignore = "opens a visible Chrome window"]
async fn open_browser_uses_the_bound_session_and_continue_frees_its_profile() {
    use crate::engine::net::fixtures::Fixtures;
    let fx = Fixtures::start().await;
    fx.route("/wall", 200, "text/html", "<html><head><title>Sign in</title></head><body><h1>Sign in to continue</h1></body></html>");
    let h = Harness::new();
    let core = h.open();
    let source = Walled::at(&format!("{}/wall", fx.base()), ChallengeKind::Login, vec![cand("w.test", 1, "Acme", "Engineer")]);
    let mut deps = h.deps();
    let id = add_walled(&core, &mut deps, source.clone());
    let session = core.sessions().create("Acme", "https://acme.test/login").await.unwrap();
    core.set_job_source_session(id, Some(session.id)).unwrap();
    let summary = core.discover_now_with(deps.clone()).await.unwrap();
    let thread = source_thread(&summary.run, id);

    let (first, second) = tokio::join!(super::handoff::open_browser(&core, &thread, true), super::handoff::open_browser(&core, &thread, true));
    first.unwrap();
    second.unwrap();
    assert!(core.sessions().is_open(session.id), "the window runs on the session's profile");
    assert!(core.sessions().launch(session.id, true).await.is_err(), "a profile has one live browser");
    wait_for_hits(&fx, "/wall").await;
    assert_eq!(fx.hits_on("/wall").len(), 1);

    // A read of the session while the window is open says so, instead of failing on the profile lock.
    let renderer = crate::engine::browser::render::Renderer::new(&core, true);
    let err = renderer.render(Some(session.id), &format!("{}/wall", fx.base())).await.unwrap_err();
    assert_eq!(err.downcast_ref::<crate::engine::browser::gate::SessionInUse>(), Some(&crate::engine::browser::gate::SessionInUse::Window), "{err:#}");

    source.solved.store(true, Ordering::SeqCst);
    assert_eq!(core.discover_resume_with(deps, &thread).await.unwrap().status, SourceStatus::Done);
    assert!(!core.sessions().is_open(session.id));
    // The window is closed and the profile is free for the next read.
    let page = renderer.render(Some(session.id), &format!("{}/wall", fx.base())).await.unwrap();
    assert!(page.html.contains("Sign in"));
    core.sessions().launch(session.id, true).await.unwrap().close().await.unwrap();
}

#[tokio::test]
async fn a_run_in_progress_is_running_and_not_due() {
    let h = Harness::new();
    let core = Arc::new(h.open());
    h.add_source(&core, "a", FakeSource::new(vec![vec![cand("a.test", 1, "Acme", "Engineer")]]));
    core.set_discover_daily(true).unwrap();
    assert!(core.discover_due(chrono::Utc::now()).unwrap());
    // The search stops at the challenge check until the test lets it go.
    let (reached, go) = (Arc::new(tokio::sync::Notify::new()), Arc::new(tokio::sync::Notify::new()));
    let mut deps = h.deps();
    let (r, g, first) = (reached.clone(), go.clone(), Arc::new(AtomicBool::new(true)));
    deps.challenge = Arc::new(move |_| {
        let (r, g, first) = (r.clone(), g.clone(), first.clone());
        async move {
            if first.swap(false, Ordering::SeqCst) {
                r.notify_one();
                g.notified().await;
            }
            Ok(None)
        }
        .boxed()
    });
    let run = {
        let core = core.clone();
        tokio::spawn(async move { core.discover_now_with(deps).await })
    };
    reached.notified().await;
    assert!(core.discover_running().unwrap());
    assert!(!core.discover_due(chrono::Utc::now()).unwrap(), "never two runs at once");
    go.notify_one();
    run.await.unwrap().unwrap();
    assert!(!core.discover_running().unwrap());
    assert!(!core.discover_due(chrono::Utc::now()).unwrap(), "it just ran");
}
