//! Market trends. Collection: sources the user adds (Hacker News, the YC directory, feeds, pages)
//! and, with a search API key, annual and quarterly reports, all collected into `trend_items` by
//! the `trends` graph ([`collect`]). The digest ([`digest`]): what the items say, with skill counts
//! and checked citations, saved as a report.
//!
//! ```text
//! core.add_trend_source(SourceKind::HackerNews, "", "")?;
//! core.add_trend_source(SourceKind::Rss, "Eng blog", "https://blog.example.com/feed.xml")?;
//! core.set_search_key(SearchProvider::Brave, "BSA...")?;            // keyring only
//! let outcome = core.collect_trends(Period::current_week()).await?; // Outcome<TrendRun>
//! let outcome = core.collect_trends_and_digest(Period::current_week()).await?; // and the digest
//! let latest = core.latest_trend_digest()?;                         // Option<Digest>
//! ```
//!
//! Layout: [`source`] has one adapter per kind (no store, no disk, only [`http::Http`]), [`collect`]
//! the graph and its state, [`digest`] the nodes after the collection and the report API,
//! [`config`] the injectable endpoints and limits, [`period`] the week or quarter a run covers.
//!
//! Rules that hold for every adapter: read-only GET (the search APIs take a POST for the query),
//! http(s) only, a minimum gap per host and caps on items and bytes ([`CollectLimits`]), text from a
//! page is data and never decides what is fetched, addresses taken from third-party data (feed
//! entries, search results) must be on public hosts, URLs of user feeds never appear in errors or
//! logs (they may carry a token), and API keys live in the keyring and nowhere else.

pub mod collect;
pub mod config;
pub mod digest;
pub mod frame;
pub use crate::engine::net::http;
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
    detect_challenge, graph, thread_id, Collected, Plan, SearchPlan, SourceOutcome, SourceStatus, Stored, TrendRun, GRAPH,
};
pub use config::{CollectLimits, Endpoints, TrendConfig};
pub use digest::{
    digest_graph, digest_thread_id, Claim, CitedItem, Digest, DigestConfig, DigestData, DigestLimits, DigestState, Extraction,
    Point, Section, SkillCount, SkillCounts, TrendChallenge, DIGEST_GRAPH,
};
pub use period::Period;
pub use source::{Item, SearchProvider, SourceKind};

use crate::engine::graph::Outcome;
use crate::engine::store::{NewSource, Source, SourcePurpose};
use crate::engine::{Core, Paths, Secrets};

impl Paths {
    /// `trends/`: the text of collected items, one `<hash of the URL>.txt` each.
    pub fn trends_dir(&self) -> PathBuf {
        self.data_dir.join("trends")
    }
}

/// The search API whose key is saved (Brave first when both are).
pub(crate) fn search_provider(secrets: &Secrets) -> Result<Option<SearchProvider>> {
    for provider in SearchProvider::ALL {
        if secrets.get(provider.secret_name())?.is_some_and(|key| !key.trim().is_empty()) {
            return Ok(Some(provider));
        }
    }
    Ok(None)
}

impl Core {
    // ------------------------------------------------------------ sources

    /// The user's trend sources (enabled or not), oldest first.
    pub fn trend_sources(&self) -> Result<Vec<Source>> {
        self.store.list_sources(SourcePurpose::Trends)
    }

    /// Add a trend source. Hacker News and YC need no address (`url` is ignored); a feed or page
    /// needs an http(s) one. An empty `name` becomes the kind's label or the address's host. Adding
    /// what is already there is an error.
    pub fn add_trend_source(&self, kind: SourceKind, name: &str, url: &str) -> Result<Source> {
        let url = if kind.needs_url() {
            source::normalize_url(url).with_context(|| format!("{} needs an http(s) address", kind.label()))?
        } else {
            String::new()
        };
        let existing = self.trend_sources()?;
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

    // ------------------------------------------------------------ search key

    /// The search API a key is saved for, if any.
    pub fn search_provider(&self) -> Result<Option<SearchProvider>> {
        search_provider(&self.secrets)
    }

    /// Save the API key of `provider` in the keyring; report search is enabled from then on.
    pub fn set_search_key(&self, provider: SearchProvider, key: &str) -> Result<()> {
        let key = key.trim();
        ensure!(!key.is_empty(), "the {} API key is empty", provider.label());
        self.secrets.set(provider.secret_name(), key)
    }

    /// Delete the saved key of `provider`.
    pub fn clear_search_key(&self, provider: SearchProvider) -> Result<()> {
        self.secrets.delete(provider.secret_name())
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
