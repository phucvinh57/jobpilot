//! The states of the two graphs and what a run reports.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::score::Scored;
use super::source::Candidate;
use crate::graph::nodes::{Challenge, Challenged};
use crate::graph::WaitReason;

/// Graph name of one source's search (thread `discover:<run>:<source id>`).
pub const SOURCE_GRAPH: &str = "source_run";
/// Graph name of a whole run (thread `discover:<run>`).
pub const DISCOVERY_GRAPH: &str = "discovery";
/// kv key holding the last run's [`RunSummary`] as JSON.
pub const LAST_RUN_KEY: &str = "discover.last_run";

/// The thread of a whole run.
pub fn discovery_thread(run: &str) -> String {
    format!("discover:{run}")
}

/// The thread of one source within a run.
pub fn source_thread(run: &str, source_id: i64) -> String {
    format!("discover:{run}:{source_id}")
}

/// `(run, source id)` of a source thread.
pub fn parse_source_thread(thread: &str) -> Option<(String, i64)> {
    let mut parts = thread.strip_prefix("discover:")?.split(':');
    let (run, id) = (parts.next()?, parts.next()?);
    parts.next().is_none().then_some(())?;
    Some((run.to_string(), id.parse().ok()?))
}

/// One source's run: search a page, drop what is known, fetch, filter, score, save; then the next
/// page. Everything a restart needs is here, so a resumed thread goes on where it stopped.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SourceState {
    pub run: String,
    pub source_id: i64,
    /// Where the next search starts (`None` at the first page).
    pub cursor: Option<String>,
    /// No page is left to search: the adapter has no more, or the page limit was reached.
    pub paged_out: bool,
    /// Pages that brought something new (the limit counts these).
    pub pages: u32,
    /// Pages searched, new or not.
    pub scanned: u32,
    /// The last search result, before it is deduplicated.
    pub page: Vec<Candidate>,
    /// Where the page after `page` starts; taken over when `page` is.
    pub page_next: Option<String>,
    /// New postings of this run, in order (append list); `taken` of them were started.
    pub candidates: Vec<Candidate>,
    pub taken: usize,
    /// The posting being worked on.
    pub current: Option<Candidate>,
    /// Its text.
    pub jd: String,
    /// Why the hard filters dropped it.
    pub verdict: Option<String>,
    pub scored: Option<Scored>,
    pub insight: Option<String>,
    pub saved: u32,
    pub dismissed: u32,
    /// Postings left alone: no readable text, or saved meanwhile by someone else.
    pub skipped: u32,
    /// `skipped` by kind (`needs_browser`, `session_in_use`, `gone`, `forbidden`, `unreadable`,
    /// `too_short`, `duplicate`): labels only, never page text.
    pub skip_kinds: BTreeMap<String, u32>,
    /// What the user has to deal with before the source goes on.
    pub challenge: Option<Challenge>,
    /// Challenges in a row; a page that goes through resets it.
    pub strikes: u32,
    /// Why the source ended early.
    pub stop: Option<String>,
}

impl SourceState {
    pub fn new(run: &str, source_id: i64) -> Self {
        Self { run: run.to_string(), source_id, ..Self::default() }
    }
}

impl Challenged for SourceState {
    fn challenge(&self) -> Option<&Challenge> {
        self.challenge.as_ref()
    }
}

/// How a source ended in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    Done,
    /// Paused at a login, captcha or block page; see `Core::discover_waiting`.
    Waiting,
    Failed,
    Cancelled,
    /// Not started: paused for the day, or still waiting from an earlier run.
    NotRun,
}

/// One source in a [`RunSummary`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceResult {
    pub source_id: i64,
    pub thread: String,
    pub status: SourceStatus,
    pub saved: u32,
    pub dismissed: u32,
    pub skipped: u32,
    /// `skipped` by kind, see [`SourceState::skip_kinds`].
    #[serde(default)]
    pub skip_kinds: BTreeMap<String, u32>,
    /// What the user should know: `login` / `captcha` / `blocked` for a waiting source, the error
    /// for a failed one (secrets masked), why it was not run, or why it ended early. Empty when
    /// all is well.
    pub note: String,
}

impl SourceResult {
    pub(crate) fn of(thread: &str, source_id: i64, status: SourceStatus, state: Option<&SourceState>, note: String) -> Self {
        Self {
            source_id,
            thread: thread.to_string(),
            status,
            saved: state.map_or(0, |s| s.saved),
            dismissed: state.map_or(0, |s| s.dismissed),
            skipped: state.map_or(0, |s| s.skipped),
            skip_kinds: state.map(|s| s.skip_kinds.clone()).unwrap_or_default(),
            note,
        }
    }
}

/// What a run found; also kept as kv `discover.last_run`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunSummary {
    pub run: String,
    pub started_at: String,
    pub finished_at: String,
    /// New leads.
    pub saved: u32,
    /// Postings dropped by the hard filters.
    pub dismissed: u32,
    pub skipped: u32,
    /// Sources waiting for the user.
    pub waiting: u32,
    pub failed: u32,
    pub sources: Vec<SourceResult>,
}

impl RunSummary {
    /// Add up the sources.
    pub(crate) fn of(run: &str, started_at: &str, finished_at: &str, sources: Vec<SourceResult>) -> Self {
        let count = |status| sources.iter().filter(|r| r.status == status).count() as u32;
        Self {
            run: run.to_string(),
            started_at: started_at.to_string(),
            finished_at: finished_at.to_string(),
            saved: sources.iter().map(|r| r.saved).sum(),
            dismissed: sources.iter().map(|r| r.dismissed).sum(),
            skipped: sources.iter().map(|r| r.skipped).sum(),
            waiting: count(SourceStatus::Waiting),
            failed: count(SourceStatus::Failed),
            sources,
        }
    }
}

/// The parent graph's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryState {
    pub run: String,
    pub started_at: String,
    /// The sources that get a thread.
    pub sources: Vec<i64>,
    /// Sources not run, then every source's result.
    pub results: Vec<SourceResult>,
    pub summary: Option<RunSummary>,
}

/// `login` / `captcha` / `blocked`, for notes and notifications.
pub fn wait_label(reason: &WaitReason) -> &'static str {
    match reason {
        WaitReason::Login { .. } => "login",
        WaitReason::Captcha { .. } => "captcha",
        WaitReason::Blocked { .. } => "blocked",
        _ => "waiting",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_ids_round_trip() {
        let t = source_thread("1731500000000", 7);
        assert_eq!(t, "discover:1731500000000:7");
        assert_eq!(parse_source_thread(&t), Some(("1731500000000".to_string(), 7)));
        assert_eq!(parse_source_thread(&discovery_thread("1731500000000")), None, "the run's own thread has no source");
        assert_eq!(parse_source_thread("discover:1:x"), None);
        assert_eq!(parse_source_thread("discover:1:2:3"), None);
        assert_eq!(parse_source_thread("prefs"), None);
    }

    #[test]
    fn a_state_saved_by_an_older_version_still_loads() {
        let old: SourceState = serde_json::from_str(r#"{"run": "1", "source_id": 3}"#).unwrap();
        assert_eq!(old, SourceState::new("1", 3));
    }

    #[test]
    fn the_summary_adds_up_the_sources() {
        let done = SourceState { saved: 2, dismissed: 1, skipped: 1, ..SourceState::new("1", 1) };
        let summary = RunSummary::of(
            "1",
            "a",
            "b",
            vec![
                SourceResult::of("t1", 1, SourceStatus::Done, Some(&done), String::new()),
                SourceResult::of("t2", 2, SourceStatus::Waiting, None, "login".into()),
                SourceResult::of("t3", 3, SourceStatus::Failed, None, "boom".into()),
            ],
        );
        assert_eq!((summary.saved, summary.dismissed, summary.skipped, summary.waiting, summary.failed), (2, 1, 1, 1, 1));
        let round: RunSummary = serde_json::from_str(&serde_json::to_string(&summary).unwrap()).unwrap();
        assert_eq!(round, summary);
    }
}
