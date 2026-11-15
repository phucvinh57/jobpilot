//! What the view calls: run the digest, read the reports, see which source waits for the user.
//!
//! ```ignore
//! // "Collect & build digest": collect, then the digest (a source at a login or captcha pauses it).
//! let outcome = core.collect_trends_and_digest(Period::current_week()).await?;
//! // After the user dealt with the page: carry on (collection, then the digest).
//! core.resume_trends_collection(period, TrendConfig::default()).await?;
//! // "Regenerate": the digest alone, from the items already stored.
//! core.build_trends_digest(period).await?;
//! let latest = core.latest_trend_digest()?;           // Digest { report, data }
//! ```

use anyhow::Result;

use super::{digest_graph, digest_thread_id, DigestConfig, DigestData};
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::graph::{GraphDef, Outcome, ThreadStatus, WaitReason};
use crate::engine::store::Report;
use crate::engine::trends::collect::{self, thread_id, TrendRun};
use crate::engine::trends::config::TrendConfig;
use crate::engine::trends::period::Period;
use crate::engine::trends::source::SearchProvider;
use crate::engine::Core;

/// A saved report with its data read back. `data` is `None` for a row this version cannot read
/// (the view then shows `report.body_md` as it is).
#[derive(Clone, Debug)]
pub struct Digest {
    pub report: Report,
    pub data: Option<DigestData>,
}

impl Digest {
    pub fn from_report(report: Report) -> Self {
        let data = serde_json::from_str(&report.citations_json).ok();
        Self { report, data }
    }

    /// The period the report covers.
    pub fn period(&self) -> Option<Period> {
        Period::parse(&self.report.period)
    }
}

/// A source that stopped the collection of `period` and waits for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrendChallenge {
    pub period: Period,
    pub source_id: i64,
    /// The source's name (`"?"` if it was deleted since).
    pub source_name: String,
    /// The page to open.
    pub url: String,
    pub kind: ChallengeKind,
}

impl Core {
    /// Collect the trend items of `period` and then build its digest, with the default endpoints,
    /// limits and the default AI provider. A source that needs the user pauses the run
    /// ([`Outcome::Waiting`]): the other sources' items are stored by then.
    pub async fn collect_trends_and_digest(&self, period: Period) -> Result<Outcome<TrendRun>> {
        self.collect_trends_and_digest_with(period, TrendConfig::default(), DigestConfig::default()).await
    }

    /// [`collect_trends_and_digest`](Self::collect_trends_and_digest) with explicit endpoints,
    /// limits and model (tests). Starts a fresh run of thread `trends:<period>` like
    /// [`collect_trends_with`](Self::collect_trends_with).
    pub async fn collect_trends_and_digest_with(
        &self,
        period: Period,
        config: TrendConfig,
        digest: DigestConfig,
    ) -> Result<Outcome<TrendRun>> {
        self.run_trends(period, config, digest, true).await
    }

    /// Build the digest of `period` from the items already stored, without collecting: also the way
    /// to "regenerate", and to get a digest while a collector waits at a challenge. Thread
    /// `trends-digest:<period>`, independent of the collection's.
    pub async fn build_trends_digest(&self, period: Period) -> Result<Outcome<TrendRun>> {
        self.build_trends_digest_with(period, DigestConfig::default()).await
    }

    /// [`build_trends_digest`](Self::build_trends_digest) with an explicit model and limits.
    pub async fn build_trends_digest_with(&self, period: Period, digest: DigestConfig) -> Result<Outcome<TrendRun>> {
        let graph = digest_graph(self.store.clone(), self.secrets.clone(), self.with_profile(digest));
        let mut input = TrendRun::new(period);
        input.digest.enabled = true;
        self.start_trends_thread(&graph, &digest_thread_id(&period), input).await
    }

    /// Carry on a failed digest built by [`build_trends_digest`](Self::build_trends_digest)
    /// ("Retry").
    pub async fn resume_trends_digest(&self, period: Period) -> Result<Outcome<TrendRun>> {
        self.resume_trends_digest_with(period, DigestConfig::default()).await
    }

    pub async fn resume_trends_digest_with(&self, period: Period, digest: DigestConfig) -> Result<Outcome<TrendRun>> {
        let graph = digest_graph(self.store.clone(), self.secrets.clone(), self.with_profile(digest));
        self.graphs.resume(&graph, &digest_thread_id(&period), |_| {}).await
    }

    /// Every saved report, newest first.
    pub fn trend_reports(&self) -> Result<Vec<Report>> {
        let mut all = self.store.list_reports("weekly")?;
        all.extend(self.store.list_reports("quarterly")?);
        all.sort_by_key(|r| std::cmp::Reverse((r.created_at, r.id)));
        Ok(all)
    }

    /// One report, read back.
    pub fn trend_digest(&self, report_id: i64) -> Result<Option<Digest>> {
        Ok(self.store.get_report(report_id)?.map(Digest::from_report))
    }

    /// The newest report of any kind.
    pub fn latest_trend_digest(&self) -> Result<Option<Digest>> {
        Ok(self.trend_reports()?.into_iter().next().map(Digest::from_report))
    }

    /// Save `data` as the newest report of `period` (what `save_report` does; for sample data).
    pub fn save_trend_digest(&self, period: &Period, data: &DigestData) -> Result<Report> {
        super::cite::save(&self.store, period, data)
    }

    /// Collections paused at a login, captcha or block page, oldest first. Continue one with
    /// [`resume_trends_collection`](Self::resume_trends_collection).
    pub fn trend_challenges(&self) -> Result<Vec<TrendChallenge>> {
        let mut out = Vec::new();
        for waiting in self.graphs.waiting()? {
            if waiting.graph != collect::GRAPH {
                continue;
            }
            let Some(period) = waiting.thread.strip_prefix("trends:").and_then(Period::parse) else { continue };
            let (kind, source_id, url) = match waiting.reason {
                WaitReason::Login { source_id, url } => (ChallengeKind::Login, source_id, url),
                WaitReason::Captcha { source_id, url } => (ChallengeKind::Captcha, source_id, url),
                WaitReason::Blocked { source_id, url } => (ChallengeKind::Blocked, source_id, url),
                _ => continue,
            };
            let source_name = self.store.get_source(source_id)?.map_or_else(|| "?".to_string(), |s| s.name);
            out.push(TrendChallenge { period, source_id, source_name, url, kind });
        }
        Ok(out)
    }

    /// The search APIs a key is saved for (report search runs with any of them).
    pub fn saved_search_providers(&self) -> Result<Vec<SearchProvider>> {
        let mut saved = Vec::new();
        for provider in SearchProvider::ALL {
            if self.secrets.get(provider.secret_name())?.is_some_and(|key| !key.trim().is_empty()) {
                saved.push(provider);
            }
        }
        Ok(saved)
    }

    /// Start `graph` on `thread`: a finished run is replaced, a paused or failed one is cancelled
    /// first (asking again means starting over), a running one is an error.
    pub(crate) async fn start_trends_thread(
        &self,
        graph: &GraphDef<TrendRun>,
        thread: &str,
        input: TrendRun,
    ) -> Result<Outcome<TrendRun>> {
        if let Some(info) = self.graphs.status(thread)? {
            if !matches!(info.status, ThreadStatus::Running | ThreadStatus::Done) {
                self.graphs.cancel(thread)?;
            }
        }
        self.graphs.run(graph, thread, input).await
    }

    /// `digest` with the master profile, so the market frame knows the user's background.
    pub(crate) fn with_profile(&self, mut digest: DigestConfig) -> DigestConfig {
        if digest.profile.is_none() {
            digest.profile = Some(self.profile_path());
        }
        digest
    }

    /// Shared by [`collect_trends_with`](Self::collect_trends_with) and the digest variant.
    pub(crate) async fn run_trends(
        &self,
        period: Period,
        config: TrendConfig,
        digest: DigestConfig,
        with_digest: bool,
    ) -> Result<Outcome<TrendRun>> {
        let graph = collect::graph(self.store.clone(), self.secrets.clone(), self.paths.clone(), config, self.with_profile(digest))?;
        let mut input = TrendRun::new(period);
        input.digest.enabled = with_digest;
        self.start_trends_thread(&graph, &thread_id(&period), input).await
    }
}
