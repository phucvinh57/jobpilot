//! Market trends. Collection: sources the user adds (Hacker News, the YC directory, feeds, pages,
//! people to follow),
//! collected into `trend_items` by the `trends` graph ([`collect`]). The digest ([`digest`]): what the items say, with skill counts
//! and checked citations, saved as a report.
//!
//! ```text
//! core.add_trend_source(SourceKind::HackerNews, "", "")?;
//! core.add_trend_source(SourceKind::Rss, "Eng blog", "https://blog.example.com/feed.xml")?;
//! core.add_trend_source(SourceKind::Person, "Jensen Huang", "")?;      // interviews and talks
//! let outcome = core.collect_trends(Period::current_week()).await?; // Outcome<TrendRun>
//! let outcome = core.collect_trends_and_digest(Period::current_week()).await?; // and the digest
//! let latest = core.latest_trend_digest()?;                         // Option<Digest>
//! ```
//!
//! Layout: [`source`] has one adapter per kind (no store, no disk, only [`http::Http`]), [`collect`]
//! the graph and its state, [`digest`] the nodes after the collection and the report API,
//! [`config`] the injectable endpoints and limits, [`period`] the week, month, quarter or year a run
//! covers (one report per period), [`pdf`] a saved report as a PDF (`assets/trends.typ`).
//!
//! Rules that hold for every adapter: read-only GET,
//! http(s) only, a minimum gap per host and caps on items and bytes ([`CollectLimits`]), text from a
//! page is data and never decides what is fetched, addresses taken from third-party data (feed
//! entries) must be on public hosts, and URLs of user feeds never appear in errors or logs (they
//! may carry a token).

pub mod collect;
pub mod config;
pub mod digest;
pub mod frame;
pub use crate::net::http;
pub mod pdf;
pub mod period;
pub mod source;
mod text;
#[cfg(test)]
mod testutil;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use anyhow::{bail, ensure, Context, Result};

pub use collect::{
    detect_challenge, graph, thread_id, Collected, Plan, SourceOutcome, SourceStatus, Stored, TrendRun, GRAPH,
};
pub use config::{CollectLimits, Endpoints, TrendConfig};
pub use digest::{
    digest_graph, digest_thread_id, Claim, CitedItem, Digest, DigestConfig, DigestData, DigestLimits, DigestState, Extraction,
    Point, Section, SkillCount, SkillCounts, TrendChallenge, DIGEST_GRAPH,
};
pub use period::Period;
pub use source::{Item, SourceKind};

use crate::graph::Outcome;
use crate::store::{NewSource, Source, SourcePurpose};
use crate::{Core, Paths};

/// Longest name of a person to follow: what a point of the digest can carry as its topic, so an
/// `outlook` point can always name them in full.
pub const MAX_PERSON_NAME_CHARS: usize = digest::MAX_TOPIC_CHARS;

impl Paths {
    /// `trends/`: the text of collected items, one `<hash of the URL>.txt` each.
    pub fn trends_dir(&self) -> PathBuf {
        self.data_dir.join("trends")
    }
}

impl Core {
    // ------------------------------------------------------------ sources

    /// The user's trend sources (enabled or not), oldest first.
    pub fn trend_sources(&self) -> Result<Vec<Source>> {
        self.store.list_sources(SourcePurpose::Trends)
    }

    /// Add a trend source. Hacker News and YC need no address (`url` is ignored); a feed or page
    /// needs an http(s) one. An empty `name` becomes the kind's label or the address's host. A
    /// person to follow needs a name (one line, at most [`MAX_PERSON_NAME_CHARS`] characters); an
    /// address of their own (a feed or a page of their interviews) is optional. Adding what is
    /// already there is an error.
    pub fn add_trend_source(&self, kind: SourceKind, name: &str, url: &str) -> Result<Source> {
        let url = if kind.needs_url() || (kind.needs_name() && !url.trim().is_empty()) {
            source::normalize_url(url).with_context(|| {
                if kind.needs_name() {
                    "the address of their interviews must be an http(s) address (or leave it empty)".to_string()
                } else {
                    format!("{} needs an http(s) address", kind.label())
                }
            })?
        } else {
            String::new()
        };
        let existing = self.trend_sources()?;
        if kind.needs_name() {
            let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
            ensure!(!name.is_empty(), "type the name of the person to follow");
            ensure!(name.chars().count() <= MAX_PERSON_NAME_CHARS, "that name is too long (at most {MAX_PERSON_NAME_CHARS} characters)");
            ensure!(
                !existing.iter().any(|s| s.kind == kind.as_str() && s.name.to_lowercase() == name.to_lowercase()),
                "you already follow {name}"
            );
            return self.store.add_source(&NewSource { kind: kind.as_str().into(), name, url, purpose: SourcePurpose::Trends, session_id: None });
        }
        ensure!(
            !existing.iter().any(|s| s.kind == kind.as_str() && s.url == url),
            "{} is already a trend source",
            if url.is_empty() { kind.label() } else { "that address" }
        );
        let name = name.trim();
        let name = if !name.is_empty() {
            name.to_string()
        } else if let Some(host) = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_string)) {
            host
        } else {
            kind.label().to_string()
        };
        self.store.add_source(&NewSource { kind: kind.as_str().into(), name, url, purpose: SourcePurpose::Trends, session_id: None })
    }

    /// Remove a trend source. Its items stay (they lose the link to the source).
    pub fn remove_trend_source(&self, id: i64) -> Result<()> {
        self.trend_source(id)?;
        self.store.delete_source(id)
    }

    /// Turn a trend source on or off for the next run.
    pub fn set_trend_source_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.trend_source(id)?;
        self.store.set_source_enabled(id, enabled)
    }

    fn trend_source(&self, id: i64) -> Result<Source> {
        match self.store.get_source(id)? {
            Some(source) if source.purpose == SourcePurpose::Trends => Ok(source),
            Some(_) => bail!("source {id} is not a trend source"),
            None => bail!("trend source {id} not found"),
        }
    }

    // ------------------------------------------------------------ collection

    /// Collect the trend items of `period` with the default endpoints and limits.
    pub async fn collect_trends(&self, period: Period) -> Result<Outcome<TrendRun>> {
        self.collect_trends_with(period, TrendConfig::default()).await
    }

    /// [`collect_trends`](Self::collect_trends) with explicit endpoints and limits (tests).
    ///
    /// Starts a fresh run of thread `trends:<period>`: a finished run is replaced, and a paused or
    /// failed one is cancelled first (asking again means starting over; use
    /// [`resume_trends_collection`](Self::resume_trends_collection) to carry on). A run that is
    /// still going is an error. Per-source trouble never fails the run: see [`TrendRun::outcomes`].
    pub async fn collect_trends_with(&self, period: Period, config: TrendConfig) -> Result<Outcome<TrendRun>> {
        self.run_trends(period, config, DigestConfig::default(), false).await
    }

    /// Carry on a paused ("Continue" after a login or captcha) or failed ("Retry") collection of
    /// `period`. A run started with the digest goes on to the digest.
    pub async fn resume_trends_collection(&self, period: Period, config: TrendConfig) -> Result<Outcome<TrendRun>> {
        self.resume_trends_collection_with(period, config, DigestConfig::default()).await
    }

    /// [`resume_trends_collection`](Self::resume_trends_collection) with an explicit model and
    /// digest limits (tests).
    pub async fn resume_trends_collection_with(
        &self,
        period: Period,
        config: TrendConfig,
        digest: DigestConfig,
    ) -> Result<Outcome<TrendRun>> {
        let graph = graph(self.store.clone(), self.secrets.clone(), self.paths.clone(), config, self.with_profile(digest))?;
        self.graphs.resume(&graph, &thread_id(&period), |s| s.challenge = None).await
    }
}
