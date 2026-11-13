//! P10 - job discovery: sources find candidate postings, the runner drops the known ones, fetches,
//! filters and scores the rest and stores them as leads (design: `docs/plans/P10-discovery.md`).
//!
//! ```text
//! let summary = core.discover_now().await?;            // "Search now": every enabled job source
//! for w in core.discover_waiting()? { .. }              // sources paused at a login, captcha or block page
//! core.discover_open_browser(&w.thread).await?;         // "Open browser": a visible window on the page to deal with
//! core.discover_resume(&w.thread).await?;               // "Continue" (or "Retry" for a failed source)
//! core.discover_cancel(&w.thread)?;                     // skip that source
//! core.add_job_source("career_page", "", url, None)?;   // Settings > Job sources: add, enable, bind a session, remove
//! ```
//!
//! - [`source`]: the adapters (`greenhouse`, `lever`, `ashby`, `hn_hiring`; `yc` and `career_page`, which
//!   read web pages and use the managed browser when a page needs it) behind [`JobSource`].
//! - [`run`]: the `discovery` graph (one thread per run) and the `source_run` graph (one thread per
//!   source, so a paused source never holds back the others).
//! - [`score`]: hard filters in code, then one model call per lead, then the weighted sum.
//! - [`dedup`]: canonical URL and `slug(company)-slug(role)` against leads and applications.
//! - [`deps`]: the replaceable pieces of a run. [`DiscoverDeps::live`] wires the text-based challenge
//!   probe, `pipeline::insight` and the posting fetch with the managed browser as its fallback.
//! - [`handoff`]: the login / captcha / block page hand-off: [`Core::discover_open_browser`] opens the
//!   page visibly for the user (the app never solves a challenge), the default [`text_probe`], and
//!   [`needs_human`] (a paused run's reason as the `Problem` to show).
//! - [`sources`]: add, enable, bind a session to and remove job sources (`Core::job_sources`, ...).
//! - [`limits`]: the per-host delay, pages per run, and the three-challenges rule. Every request
//!   (adapters and posting fetch) goes through the shared safe client, [`crate::net::http`].
//!
//! Posting texts are untrusted: they only ever reach the model between markers, as data. URLs
//! come from the adapters and are restricted to http(s).

pub mod dedup;
pub mod deps;
mod fetch;
pub mod handoff;
pub mod limits;
pub mod promote;
pub mod queue;
mod run;
pub mod score;
pub mod source;
pub mod sources;
pub mod state;
#[cfg(test)]
mod tests;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};

pub use deps::{
    ChallengeCheck, ChallengeProbe, CheckStage, DiscoverDeps, FetchRequest, InsightHook, InsightRequest, Notifier,
    PostingFetcher, SourceFactory,
};
pub use handoff::{needs_human, text_probe};
pub use limits::{DiscoverLimits, MAX_CHALLENGES_IN_A_ROW};
pub use run::lead_dir;
pub use source::{Candidate, JobSource, SearchCtx, SearchPage};
pub use sources::{source_url_hint, JOB_SOURCE_KINDS};
pub use state::{
    discovery_thread, parse_source_thread, source_thread, RunSummary, SourceResult, SourceState, SourceStatus,
    DISCOVERY_GRAPH, LAST_RUN_KEY, SOURCE_GRAPH,
};

use crate::graph::{Outcome, ThreadStatus, WaitReason};
use crate::Core;

/// A source paused for the user: one line of the "needs you" list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WaitingSource {
    /// The thread to pass to [`Core::discover_resume`] or [`Core::discover_cancel`].
    pub thread: String,
    pub run: String,
    pub source_id: i64,
    /// The source's name, or its kind when it has none.
    pub source_name: String,
    /// Login, captcha or blocked, with the page to open.
    pub reason: WaitReason,
    pub since: DateTime<Utc>,
}

impl Core {
    /// "Search now": run every enabled job source, with the default AI provider and limits. Returns
    /// when each source is done, waiting for the user or failed; the summary is also kept as kv
    /// `discover.last_run`. Run it on the global runtime (`runtime::spawn`) and show progress from
    /// `core.graphs.subscribe()`.
    pub async fn discover_now(&self) -> Result<RunSummary> {
        self.discover_now_with(DiscoverDeps::live(self)?).await
    }

    /// [`discover_now`](Self::discover_now) with explicit [`DiscoverDeps`].
    pub async fn discover_now_with(&self, deps: DiscoverDeps) -> Result<RunSummary> {
        let runner = run::Runner::new(self, deps)?;
        let run = run::new_run_id();
        let outcome = self.graphs.run(&runner.discovery, &discovery_thread(&run), run::new_discovery(&run)).await?;
        match outcome {
            Outcome::Done(state) => state.summary.context("the run ended without a summary"),
            Outcome::Failed(failure) => bail!("{}", failure.message),
            Outcome::Waiting(_) => bail!("the discovery run paused unexpectedly"),
            Outcome::Cancelled => bail!("the discovery run was cancelled"),
        }
    }

    /// Sources paused at a login, captcha or block page, oldest first. Survives a restart: on
    /// start, list them so the user can continue or skip each.
    pub fn discover_waiting(&self) -> Result<Vec<WaitingSource>> {
        let mut out = Vec::new();
        for w in self.graphs.waiting()?.into_iter().filter(|w| w.graph == SOURCE_GRAPH) {
            let Some((run, source_id)) = parse_source_thread(&w.thread) else { continue };
            let source_name = match self.store.get_source(source_id)? {
                Some(s) if !s.name.trim().is_empty() => s.name.trim().to_string(),
                Some(s) => s.kind,
                None => "(deleted source)".to_string(),
            };
            out.push(WaitingSource { thread: w.thread, run, source_id, source_name, reason: w.reason, since: w.since });
        }
        Ok(out)
    }

    /// "Continue" for a waiting source (after the user dealt with the page in the browser) and
    /// "Retry" for a failed one: the source goes on from its saved position. Returns how the
    /// thread ended this time (it may wait again).
    pub async fn discover_resume(&self, thread: &str) -> Result<SourceResult> {
        self.discover_resume_with(DiscoverDeps::live(self)?, thread).await
    }

    /// [`discover_resume`](Self::discover_resume) with explicit [`DiscoverDeps`].
    pub async fn discover_resume_with(&self, deps: DiscoverDeps, thread: &str) -> Result<SourceResult> {
        let (_, source_id) = parse_source_thread(thread).context("not a discovery source thread")?;
        match self.graphs.status(thread)?.map(|t| t.status) {
            Some(ThreadStatus::Waiting(_) | ThreadStatus::Failed(_)) => {}
            Some(ThreadStatus::Running) => bail!("that source is running right now"),
            Some(ThreadStatus::Done) => bail!("that source has finished"),
            None => bail!("no such source thread: it was cancelled or is too old"),
        }
        let runner = run::Runner::new(self, deps)?;
        // The user is done with the browser window: close it, so the next read can use its profile.
        handoff::close_window(&self.paths, thread).await;
        self.store.set_source_status(source_id, "running")?;
        let outcome = self.graphs.resume(&runner.source, thread, |s| s.challenge = None).await;
        Ok(run::source_result(&self.store, thread, source_id, outcome))
    }

    /// Skip a waiting (or failed) source for this run: its thread is deleted. The source stays
    /// enabled and is searched again by the next run.
    pub fn discover_cancel(&self, thread: &str) -> Result<()> {
        let (_, source_id) = parse_source_thread(thread).context("not a discovery source thread")?;
        handoff::discard_window(&self.paths, thread);
        self.graphs.cancel(thread)?;
        // The source may be gone; its status is only a label.
        let _ = self.store.set_source_status(source_id, "cancelled");
        Ok(())
    }

    /// The summary of the last run, if there was one.
    pub fn discover_last_run(&self) -> Result<Option<RunSummary>> {
        match self.store.kv_get(LAST_RUN_KEY)? {
            Some(json) => Ok(Some(serde_json::from_str(&json).context("the last run's summary is not valid")?)),
            None => Ok(None),
        }
    }
}
