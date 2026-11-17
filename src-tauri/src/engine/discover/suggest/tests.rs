//! Graph tests of "Suggest sources": fake results pages, fake checks, a real SQLite file; no
//! network, no browser.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;

use super::verify::fake::FakeVerify;
use super::*;
use crate::engine::browser::challenge::ChallengeFound;
use crate::engine::discover::source::{Page, Pages};
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::graph::{ProgressEvent, ThreadStatus, WaitReason};
use crate::engine::health::{classify, Problem};
use crate::engine::prefs::{Preferences, Remote};
use crate::engine::store::{NewSource, NewSuggestion, SourcePurpose, SuggestionState};
use crate::engine::{Paths, Secrets};

const SEARCH: &str = "https://search.example/search?q={q}";

/// Results pages by URL, rendered "in the browser"; a blocked URL shows a captcha until unblocked,
/// and a failing one answers an error.
#[derive(Default)]
struct SearchPages {
    pages: Mutex<HashMap<String, String>>,
    blocked: Mutex<HashSet<String>>,
    failing: Mutex<HashMap<String, String>>,
    asked: Mutex<Vec<String>>,
}

impl SearchPages {
    fn page(&self, url: &str, html: &str) {
        self.pages.lock().unwrap().insert(url.into(), html.into());
    }
    fn block(&self, url: &str, on: bool) {
        let mut blocked = self.blocked.lock().unwrap();
        if on {
            blocked.insert(url.into());
        } else {
            blocked.remove(url);
        }
    }
    fn fail(&self, url: &str, error: Option<&str>) {
        let mut failing = self.failing.lock().unwrap();
        match error {
            Some(e) => failing.insert(url.into(), e.into()),
            None => failing.remove(url),
        };
    }
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl Pages for SearchPages {
    async fn plain(&self, _url: &str) -> Result<Page> {
        bail!("a results page is never read without the browser")
    }

    async fn rendered(&self, source: &crate::engine::store::Source, url: &str) -> Result<Page> {
        assert_eq!(source.session_id, None, "results pages are read in a Private tab");
        self.asked.lock().unwrap().push(url.to_string());
        if self.blocked.lock().unwrap().contains(url) {
            return Err(anyhow::Error::new(ChallengeFound { kind: ChallengeKind::Captcha, url: url.to_string() }));
        }
        if let Some(e) = self.failing.lock().unwrap().get(url) {
            return Err(anyhow!("{e}"));
        }
        let html = self.pages.lock().unwrap().get(url).cloned().unwrap_or_default();
        Ok(Page { url: url.to_string(), html })
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    core: Core,
    pages: Arc<SearchPages>,
    notices: Arc<Mutex<Vec<String>>>,
}

impl Harness {
    fn new(prefs: Preferences) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        core.store.save_preferences(&prefs).unwrap();
        Self { _dir: dir, core, pages: Arc::default(), notices: Arc::default() }
    }

    fn rust_in_berlin() -> Self {
        Self::new(Preferences { roles: vec!["Rust Engineer".into()], locations: vec!["Berlin".into()], remote: Remote::Any, ..Preferences::default() })
    }

    fn deps(&self, verify: Arc<FakeVerify>) -> SuggestDeps {
        let notices = self.notices.clone();
        SuggestDeps {
            pages: self.pages.clone(),
            verify,
            notify: Arc::new(move |title, body| notices.lock().unwrap().push(format!("{title}: {body}"))),
            config: SuggestConfig { search_url: SEARCH.into() },
        }
    }

    /// The results page address of query number `i` (from 0).
    fn url(&self, i: usize) -> String {
        let prefs = self.core.store.preferences().unwrap();
        let queries = build_queries(&prefs, None).unwrap();
        SuggestConfig { search_url: SEARCH.into() }.url_for(&queries[i]).unwrap().to_string()
    }
}

fn inputs(core: &Core) -> Vec<String> {
    core.source_suggestions().unwrap().into_iter().map(|s| s.input).collect()
}

#[tokio::test]
async fn a_captcha_pauses_the_search_and_continue_searches_the_same_query_then_saves_checked_sources() {
    let h = Harness::rust_in_berlin();
    let core = &h.core;
    // The user has a Lever source, and dismissed a Greenhouse suggestion earlier.
    core.store.add_source(&NewSource { kind: "lever".into(), url: "jobs.lever.co/known".into(), purpose: SourcePurpose::Jobs, ..Default::default() }).unwrap();
    let dismissed = NewSuggestion { kind: "greenhouse".into(), input: "boards.greenhouse.io/dismissedco".into(), ..Default::default() };
    let dismissed = core.store.upsert_suggestion(&dismissed).unwrap().unwrap();
    core.dismiss_source_suggestion(dismissed.id).unwrap();

    h.pages.page(
        &h.url(0),
        r#"<a href="/url?q=https://boards.greenhouse.io/acme/jobs/1&amp;sa=U">Acme</a>
           <a href="https://jobs.lever.co/known/1">Known</a>
           <a href="https://boards.greenhouse.io/dismissedco/jobs/3">Dismissed</a>"#,
    );
    h.pages.page(
        &h.url(1),
        r#"<a href="https://jobs.ashbyhq.com/beta/1">Beta</a>
           <a href="https://careers.gamma.example/jobs/42">Gamma</a>
           <a href="https://boards.greenhouse.io/acme/jobs/2">Acme again</a>"#,
    );
    h.pages.page(&h.url(2), r#"<a href="https://careers.zero.example/">Zero</a> <a href="https://jobs.lever.co/broken">Broken</a>"#);
    h.pages.block(&h.url(1), true);
    let verify = Arc::new(
        FakeVerify::new()
            .ok("greenhouse", "boards.greenhouse.io/acme", "Acme", 10, 4)
            .ok("ashby", "jobs.ashbyhq.com/beta", "Beta", 5, 5)
            .ok("career_page", "https://careers.gamma.example/jobs", "Gamma", 3, 1)
            .ok("career_page", "https://careers.zero.example/", "Zero", 0, 0)
            .fails("jobs.lever.co/broken", "no such board on Lever"),
    );

    let outcome = core.suggest_sources_with(h.deps(verify.clone())).await.unwrap();
    let SuggestOutcome::Waiting { thread, reason } = outcome else { panic!("expected a pause, got {outcome:?}") };
    assert_eq!(thread, SUGGEST_THREAD);
    assert_eq!(reason, WaitReason::Captcha { source_id: 0, url: h.url(1) });
    let waiting = core.suggest_waiting().unwrap().expect("listed for the Open browser banner");
    assert_eq!((waiting.thread.as_str(), waiting.graph.as_str()), (SUGGEST_THREAD, SUGGEST_GRAPH));
    assert!(core.source_suggestions().unwrap().is_empty(), "nothing is saved before the checks");
    assert!(verify.asked().is_empty());
    assert_eq!(h.notices.lock().unwrap().len(), 1);
    // Resuming a search that is not waiting, or opening a tab for it, is refused cleanly elsewhere;
    // here the user solved the captcha in Chrome.
    h.pages.block(&h.url(1), false);

    let outcome = core.suggest_resume_with(h.deps(verify.clone())).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: 3 });
    assert!(core.suggest_waiting().unwrap().is_none());
    assert_eq!(h.pages.asked()[..3], [h.url(0), h.url(1), h.url(1)], "the same query again after Continue");
    assert_eq!(h.pages.asked().len(), 5, "four queries, one of them twice");
    assert_eq!(
        verify.asked(),
        ["boards.greenhouse.io/acme", "jobs.ashbyhq.com/beta", "https://careers.gamma.example/jobs", "https://careers.zero.example/", "jobs.lever.co/broken"],
        "known sources and dismissed suggestions are never checked; each board once"
    );
    let saved = core.source_suggestions().unwrap();
    assert_eq!(inputs(core), ["jobs.ashbyhq.com/beta", "boards.greenhouse.io/acme", "https://careers.gamma.example/jobs"], "best first");
    assert_eq!((saved[1].name.as_str(), saved[1].jobs, saved[1].matching), ("Acme", 10, 4));
    assert_eq!(saved[1].query, "\"Rust Engineer\" Berlin site:boards.greenhouse.io");
    assert_eq!(core.store.get_suggestion(dismissed.id).unwrap().unwrap().state, SuggestionState::Dismissed);

    // The user dismisses Acme and adds Beta; a new search brings neither back, even when another
    // page turns out to be Acme's board.
    let (beta, acme) = (saved[0].id, saved[1].id);
    core.dismiss_source_suggestion(acme).unwrap();
    core.mark_source_suggestion_added(beta).unwrap();
    h.pages.page(&h.url(3), r#"<a href="https://careers.acme-co.example/">Acme careers</a>"#);
    let verify = Arc::new(FakeVerify::new().resolved(
        "https://careers.acme-co.example/",
        Verified { kind: "greenhouse".into(), url: "boards.greenhouse.io/acme".into(), name: "Acme".into(), jobs: 12, matching: 6 },
    ));
    let outcome = core.suggest_sources_with(h.deps(verify.clone())).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: 0 });
    assert_eq!(
        verify.asked(),
        ["https://careers.zero.example/", "jobs.lever.co/broken", "https://careers.acme-co.example/"],
        "the stored ones (any state) are known now; refused candidates are not remembered, so checked again"
    );
    assert_eq!(inputs(core), ["https://careers.gamma.example/jobs"]);
    assert_eq!(core.store.get_suggestion(acme).unwrap().unwrap().state, SuggestionState::Dismissed);
    assert_eq!(core.store.get_suggestion(beta).unwrap().unwrap().state, SuggestionState::Added);
    core.dismiss_source_suggestion(beta).unwrap();
    assert_eq!(core.store.get_suggestion(beta).unwrap().unwrap().state, SuggestionState::Added, "an added one stays added");
    assert!(core.dismiss_source_suggestion(999).is_err());
}

#[tokio::test]
async fn excluded_companies_are_not_suggested_even_when_the_check_names_them() {
    let h = Harness::new(Preferences { roles: vec!["Rust Engineer".into()], exclude_companies: vec!["MegaCorp".into(), "Omega".into()], ..Preferences::default() });
    h.pages.page(&h.url(0), r#"<a href="https://boards.greenhouse.io/megacorp">M</a> <a href="https://jobs.lever.co/o-labs">O</a>"#);
    let verify = Arc::new(FakeVerify::new().ok("lever", "jobs.lever.co/o-labs", "Omega", 4, 4));
    let outcome = h.core.suggest_sources_with(h.deps(verify.clone())).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: 0 });
    assert_eq!(verify.asked(), ["jobs.lever.co/o-labs"], "the board named after an excluded company is not even checked");
}

#[tokio::test]
async fn at_most_twelve_candidates_are_checked() {
    let h = Harness::rust_in_berlin();
    let html: String = (0..15).map(|i| format!(r#"<a href="https://jobs.lever.co/c{i}">C{i}</a>"#)).collect();
    h.pages.page(&h.url(0), &html);
    let verify = (0..15).fold(FakeVerify::new(), |v, i| v.ok("lever", &format!("jobs.lever.co/c{i}"), &format!("C{i}"), 2, 1));
    let verify = Arc::new(verify);
    let outcome = h.core.suggest_sources_with(h.deps(verify.clone())).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: MAX_VERIFY });
    assert_eq!(verify.asked().len(), MAX_VERIFY);
}

#[tokio::test]
async fn no_roles_and_no_profile_is_the_no_profile_problem_and_the_next_search_starts_over() {
    let h = Harness::new(Preferences::default());
    let err = h.core.suggest_sources_with(h.deps(Arc::new(FakeVerify::new()))).await.unwrap_err();
    assert_eq!(classify(&format!("{err:#}")), Problem::NoProfile, "{err:#}");
    assert!(h.core.suggest_waiting().unwrap().is_none());
    assert!(matches!(h.core.graphs.status(SUGGEST_THREAD).unwrap().map(|t| t.status), Some(ThreadStatus::Failed(_))));

    // The user sets a role: the failed search is replaced by a new one.
    h.core.store.save_preferences(&Preferences { roles: vec!["Rust Engineer".into()], ..Preferences::default() }).unwrap();
    let outcome = h.core.suggest_sources_with(h.deps(Arc::new(FakeVerify::new()))).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: 0 });
}

#[tokio::test]
async fn a_network_failure_fails_the_search_retry_goes_on_and_an_unreadable_page_is_skipped() {
    let h = Harness::rust_in_berlin();
    h.pages.fail(&h.url(0), Some("network error: net::ERR_CONNECTION_REFUSED at <page>"));
    h.pages.fail(&h.url(2), Some("the page ended on an address that is not a public web page"));
    h.pages.page(&h.url(1), r#"<a href="https://jobs.ashbyhq.com/beta">Beta</a>"#);
    let verify = Arc::new(FakeVerify::new().ok("ashby", "jobs.ashbyhq.com/beta", "Beta", 3, 2));
    let err = h.core.suggest_sources_with(h.deps(verify.clone())).await.unwrap_err();
    assert_eq!(classify(&format!("{err:#}")), Problem::Network, "{err:#}");
    assert!(h.core.suggest_resume_with(h.deps(verify.clone())).await.is_err(), "still down");

    h.pages.fail(&h.url(0), None);
    let outcome = h.core.suggest_resume_with(h.deps(verify.clone())).await.unwrap();
    assert_eq!(outcome, SuggestOutcome::Done { saved: 1 }, "query 3 was skipped, not fatal");
    assert!(h.core.suggest_resume_with(h.deps(verify)).await.is_err(), "nothing waits any more");
}

#[tokio::test]
async fn the_search_reports_its_progress_and_can_be_cancelled_while_it_waits() {
    let h = Harness::rust_in_berlin();
    let mut progress = h.core.graphs.subscribe();
    h.pages.block(&h.url(2), true);
    let outcome = h.core.suggest_sources_with(h.deps(Arc::new(FakeVerify::new()))).await.unwrap();
    assert!(matches!(outcome, SuggestOutcome::Waiting { .. }));
    let mut details = Vec::new();
    while let Ok(p) = progress.try_recv() {
        if let (true, ProgressEvent::Detail { text, .. }) = (p.graph == SUGGEST_GRAPH, p.event) {
            details.push(text);
        }
    }
    assert_eq!(details, ["Searching 1 of 4", "Searching 2 of 4", "Searching 3 of 4"]);

    h.core.suggest_cancel().unwrap();
    assert!(h.core.suggest_waiting().unwrap().is_none());
    assert!(h.core.graphs.status(SUGGEST_THREAD).unwrap().is_none());
    h.core.suggest_cancel().unwrap();
    assert!(h.core.suggest_open_browser().await.unwrap_err().to_string().contains("not waiting"));
}

#[test]
fn the_search_address_takes_the_query_encoded() {
    let config = SuggestConfig::default();
    let url = config.url_for("\"Rust Engineer\" Berlin site:jobs.lever.co").unwrap();
    assert_eq!(url.as_str(), "https://www.google.com/search?q=%22Rust+Engineer%22+Berlin+site%3Ajobs.lever.co&num=20");
    assert!(SuggestConfig { search_url: "https://x.example/search".into() }.url_for("q").is_err());
    assert!(SuggestConfig { search_url: "file:///tmp/{q}".into() }.url_for("q").is_err());
}

#[test]
fn adding_a_suggestion_stores_the_source_once_and_marks_it_added() {
    let h = Harness::rust_in_berlin();
    let new = NewSuggestion { kind: "greenhouse".into(), input: "boards.greenhouse.io/acme".into(), name: "acme".into(), jobs: 4, matching: 2, ..Default::default() };
    let id = h.core.store.upsert_suggestion(&new).unwrap().unwrap().id;
    let source = h.core.add_suggested_source(id).unwrap();
    assert_eq!((source.kind.as_str(), source.name.as_str()), ("greenhouse", "acme"));
    assert_eq!(h.core.store.get_suggestion(id).unwrap().unwrap().state, SuggestionState::Added);
    assert!(h.core.source_suggestions().unwrap().is_empty());
    assert!(h.core.add_suggested_source(id).unwrap_err().to_string().contains("already added"));
    assert_eq!(h.core.job_sources().unwrap().len(), 1, "added once");
    assert!(h.core.add_suggested_source(id + 99).unwrap_err().to_string().contains("no longer exists"));
}
