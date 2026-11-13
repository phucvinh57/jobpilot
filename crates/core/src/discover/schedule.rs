//! The optional daily run (decision 5): while the app is open, a timer asks [`Core::discover_due`]
//! and, when it says yes, the app runs the same "Search now" path. Off by default (kv `discover.daily`).

use anyhow::Result;
use chrono::{DateTime, Duration, Utc};

use super::state::{DISCOVERY_GRAPH, SOURCE_GRAPH};
use crate::graph::ThreadStatus;
use crate::store::{SourcePurpose, GRAPH_RUNNING};
use crate::Core;

/// kv key of the setting ("1" = on); a missing key means off.
pub const DAILY_KEY: &str = "discover.daily";
/// kv key of the time the last daily run was started (RFC 3339), so that a run that failed before
/// it saved a summary (no provider, no network) is not started again every few minutes.
pub const DAILY_ATTEMPT_KEY: &str = "discover.daily_attempt";
/// How long after a run the next daily one is due.
pub const DAILY_EVERY_HOURS: i64 = 24;

/// Whether a daily run is due: the setting is on, there is a source to search, nothing is running,
/// and the last run (or attempt) is more than a day old or there was none. A time in the future (the
/// clock was set back) counts as due, so a bad clock never blocks the run for long.
pub fn is_due(enabled: bool, enabled_sources: usize, running: bool, last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    enabled && enabled_sources > 0 && !running && last.is_none_or(|t| t > now || now - t >= Duration::hours(DAILY_EVERY_HOURS))
}

impl Core {
    /// Whether "Search daily while JobPilot is open" is on. Off by default.
    pub fn discover_daily_enabled(&self) -> Result<bool> {
        Ok(self.store.kv_get(DAILY_KEY)?.as_deref() == Some("1"))
    }

    pub fn set_discover_daily(&self, on: bool) -> Result<()> {
        self.store.kv_set(DAILY_KEY, if on { "1" } else { "0" })
    }

    /// Whether a discovery run, or a source being continued, is running in this session.
    pub fn discover_running(&self) -> Result<bool> {
        for t in self.store.graph_threads_with_status(GRAPH_RUNNING)? {
            if (t.graph == DISCOVERY_GRAPH || t.graph == SOURCE_GRAPH)
                && matches!(self.graphs.status(&t.thread)?.map(|i| i.status), Some(ThreadStatus::Running))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether the daily run is due at `now` (see [`is_due`]). A database read: call it off the UI thread.
    pub fn discover_due(&self, now: DateTime<Utc>) -> Result<bool> {
        if !self.discover_daily_enabled()? {
            return Ok(false);
        }
        let sources = self.store.list_enabled_sources(SourcePurpose::Jobs)?.len();
        let last = self.discover_last_activity()?;
        Ok(is_due(true, sources, self.discover_running()?, last, now))
    }

    /// Record that a daily run starts at `now` (see [`DAILY_ATTEMPT_KEY`]).
    pub fn discover_daily_started(&self, now: DateTime<Utc>) -> Result<()> {
        self.store.kv_set(DAILY_ATTEMPT_KEY, &now.to_rfc3339())?;
        tracing::info!("daily discovery run started");
        Ok(())
    }

    /// The newest of the last run's end and the last daily attempt. An unreadable last-run summary
    /// counts as none, so it never blocks the daily run.
    fn discover_last_activity(&self) -> Result<Option<DateTime<Utc>>> {
        let parse = |s: &str| DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc));
        let last_run = self.discover_last_run().unwrap_or_else(|e| {
            tracing::warn!("the last discovery summary is unreadable, ignoring it: {e:#}");
            None
        });
        let run = last_run.and_then(|r| parse(&r.finished_at).or_else(|| parse(&r.started_at)));
        let attempt = self.store.kv_get(DAILY_ATTEMPT_KEY)?.and_then(|s| parse(&s));
        Ok(run.max(attempt))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::discover::{RunSummary, LAST_RUN_KEY};
    use crate::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn ran_at(core: &Core, finished: DateTime<Utc>) {
        let summary = RunSummary { run: "r".into(), finished_at: finished.to_rfc3339(), ..Default::default() };
        core.store.kv_set(LAST_RUN_KEY, &serde_json::to_string(&summary).unwrap()).unwrap();
    }

    #[test]
    fn the_rule_needs_the_setting_a_source_no_run_and_a_day_since_the_last() {
        let now = Utc::now();
        let ago = |h| Some(now - Duration::hours(h));
        assert!(is_due(true, 1, false, None, now), "never ran");
        assert!(is_due(true, 2, false, ago(25), now), "ran 25h ago");
        assert!(is_due(true, 1, false, ago(24), now), "exactly a day");
        assert!(!is_due(true, 1, false, ago(2), now), "ran 2h ago");
        assert!(!is_due(false, 1, false, None, now), "setting off");
        assert!(!is_due(true, 0, false, None, now), "no source");
        assert!(!is_due(true, 1, true, ago(30), now), "a run in progress");
        assert!(is_due(true, 1, false, Some(now + Duration::hours(1)), now), "a time in the future does not block");
    }

    #[test]
    fn the_setting_is_off_until_it_is_turned_on() {
        let (_dir, core) = core();
        assert!(!core.discover_daily_enabled().unwrap());
        core.set_discover_daily(true).unwrap();
        assert!(core.discover_daily_enabled().unwrap());
        core.set_discover_daily(false).unwrap();
        assert!(!core.discover_daily_enabled().unwrap());
    }

    #[test]
    fn due_reads_the_setting_the_sources_and_the_last_run() {
        let (_dir, core) = core();
        let now = Utc::now();
        assert!(!core.discover_due(now).unwrap(), "off");
        core.set_discover_daily(true).unwrap();
        assert!(!core.discover_due(now).unwrap(), "no source");
        let src = core.add_job_source("hn_hiring", "", "", None).unwrap();
        assert!(core.discover_due(now).unwrap(), "never ran");
        ran_at(&core, now - Duration::hours(2));
        assert!(!core.discover_due(now).unwrap(), "ran 2h ago");
        ran_at(&core, now - Duration::hours(25));
        assert!(core.discover_due(now).unwrap(), "ran 25h ago");
        core.discover_daily_started(now - Duration::minutes(20)).unwrap();
        assert!(!core.discover_due(now).unwrap(), "a daily run was started 20 minutes ago and may have failed");
        core.discover_daily_started(now - Duration::hours(26)).unwrap();
        assert!(core.discover_due(now).unwrap());
        core.set_job_source_enabled(src.id, false).unwrap();
        assert!(!core.discover_due(now).unwrap(), "the only source is disabled");
    }

    #[test]
    fn an_unreadable_last_run_does_not_block_the_daily_run() {
        let (_dir, core) = core();
        core.set_discover_daily(true).unwrap();
        core.add_job_source("hn_hiring", "", "", None).unwrap();
        core.store.kv_set(LAST_RUN_KEY, "not json").unwrap();
        assert!(core.discover_due(Utc::now()).unwrap());
    }
}
