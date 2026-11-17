//! "Suggest sources": search the web through the user's Chrome for job boards and careers pages that
//! fit the wanted roles, check each one, and keep the ones with jobs as suggestions the user can add
//! or dismiss (Settings > Job sources).
//!
//! ```text
//! match core.suggest_sources().await? {                  // "Suggest sources"
//!     SuggestOutcome::Done { saved } => { .. }            // saved new suggestions
//!     SuggestOutcome::Waiting { .. } => { .. }            // the search engine asks the user to confirm
//! }
//! core.suggest_waiting()?;                                // the paused search, for "Open browser"
//! core.suggest_open_browser().await?;                     // a focused Private tab on the page
//! core.suggest_resume().await?;                           // "Continue" (or Suggest sources again)
//! core.source_suggestions()?;                             // the new suggestions, best first
//! core.dismiss_source_suggestion(id)?;                    // never suggested again
//! core.mark_source_suggestion_added(id)?;                 // after the source was added
//! ```
//!
//! No model is involved: the queries ([`queries`]), the reading of results pages ([`results`]) and
//! the check ([`verify`]) are code, and a results page never chooses what is visited: only a
//! candidate that the check accepts becomes a suggestion, and only the user adds it.
//!
//! Results pages are rendered in a **Private** tab of the user's Chrome (the JobPilot Incognito
//! window: no cookies of theirs), guarded like every discovery tab and not disguised
//! (`navigator.webdriver` stays true). A consent page, captcha or login the search engine shows is
//! never solved: the search pauses (`graph::nodes::challenge`) and the user deals with it in a
//! focused tab, then continues.
//!
//! - [`queries`]: up to eight searches from the preferences (or the profile's roles).
//! - [`results`]: a results page -> job board / careers page candidates, minus what is known.
//! - [`verify`]: the check seam ([`VerifySource`]) and its live wiring point.
//! - `graph`: the `source_suggest` graph. `api`: the `Core` methods.

mod api;
mod graph;
pub mod queries;
mod refused;
pub mod results;
#[cfg(test)]
mod tests;
pub mod verify;

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use url::Url;

pub use api::SuggestOutcome;
pub use graph::{Checked, SuggestState, MAX_VERIFY, SUGGEST_GRAPH, SUGGEST_THREAD};
pub use queries::{build_queries, MAX_QUERIES};
pub use results::Found;
pub use verify::{matching_titles, Verified, VerifySource};

use super::deps::Notifier;
use super::fetch::MAX_POSTING_BYTES;
use super::source::{NetPages, Pages};
use crate::browser::render::Renderer;
use crate::net::http::{Http, HttpLimits};
use crate::Core;

/// The search engine's address, with `{q}` where the URL-encoded query goes.
pub const DEFAULT_SEARCH_URL: &str = "https://www.google.com/search?q={q}&num=20";

/// Where the searches go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestConfig {
    /// A results page address with `{q}` for the query ([`DEFAULT_SEARCH_URL`]). Tests point it at a
    /// fixture.
    pub search_url: String,
}

impl Default for SuggestConfig {
    fn default() -> Self {
        Self { search_url: DEFAULT_SEARCH_URL.into() }
    }
}

impl SuggestConfig {
    /// The results page of `query`: the template with the query URL-encoded into `{q}`. `Err` for a
    /// template without `{q}` or one that is not an http(s) address.
    pub fn url_for(&self, query: &str) -> Result<Url> {
        if !self.search_url.contains("{q}") {
            bail!("the search address has no {{q}} for the query");
        }
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        let url = Url::parse(&self.search_url.replace("{q}", &encoded)).context("the search address is not valid")?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            bail!("the search address must be http or https");
        }
        Ok(url)
    }
}

/// What a suggestion search needs from the outside, replaceable in tests.
#[derive(Clone)]
pub struct SuggestDeps {
    /// Reads results pages (only [`Pages::rendered`] is used: a Private tab, as the search has no
    /// session).
    pub pages: Arc<dyn Pages>,
    /// Checks a candidate (see [`verify`]).
    pub verify: Arc<dyn VerifySource>,
    /// Tells the user the search waits for them (labels only).
    pub notify: Notifier,
    pub config: SuggestConfig,
}

impl SuggestDeps {
    /// `pages` and `verify` with the default search engine and desktop notifications.
    pub fn new(pages: Arc<dyn Pages>, verify: Arc<dyn VerifySource>) -> Self {
        Self { pages, verify, notify: Arc::new(crate::notify::desktop_notify), config: SuggestConfig::default() }
    }

    /// The running app's: results pages rendered in the user's Chrome (public hosts only) and the
    /// live verifier ([`verify::live_verifier`]).
    pub fn live(core: &Core) -> Result<Self> {
        let pages: Arc<dyn Pages> = Arc::new(NetPages::new(search_client()?, Some(Renderer::new(core, false))));
        Ok(Self::new(pages, verify::live_verifier(core)))
    }
}

/// The one plain-read client of the search, for the life of the process, so its per-host delay
/// holds across searches (Suggest, Continue, Suggest again).
fn search_client() -> Result<Http> {
    static CLIENT: std::sync::OnceLock<Http> = std::sync::OnceLock::new();
    if let Some(client) = CLIENT.get() {
        return Ok(client.clone());
    }
    let client = Http::new(&HttpLimits { max_bytes: MAX_POSTING_BYTES, ..HttpLimits::default() })?;
    Ok(CLIENT.get_or_init(|| client).clone())
}
