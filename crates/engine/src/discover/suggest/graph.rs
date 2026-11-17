//! The `source_suggest` graph: one thread ([`SUGGEST_THREAD`]) per search.
//!
//! ```text
//! START -> plan -> search -+-> search                     (the next query)
//!                          +-> wait_human -> search       (a consent page, captcha or login: the user deals with it)
//!                          `-> verify -> save -> END      (every query searched)
//! ```
//!
//! - `plan`: the queries ([`build_queries`]); fails with the "no profile" problem when there is
//!   nothing to search for.
//! - `search`: one query per visit, the results page rendered in a Private tab. Its new candidates
//!   join `found` and the query index moves on; a [`ChallengeFound`] leaves the index where it is
//!   and raises `challenge`, so `wait_human` pauses the thread and "Continue" (which clears it)
//!   searches the same query again. A page that cannot be read for another reason is skipped,
//!   unless the browser cannot be used or the network is down (the thread fails: fix it, Retry).
//! - `verify`: up to [`MAX_VERIFY`] candidates that are still unknown, through the
//!   [`VerifySource`](super::VerifySource) seam; a refused one is dropped and only counted.
//! - `save`: upsert into `source_suggestions` (a dismissed or added one is never revived).
//!
//! Every node is idempotent: a resume re-runs the node from the state it started with. Logs carry
//! counts and query numbers only (never a query, a page or an address).

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::queries::build_queries;
use super::results::{new_sources, Found, Known};
use super::verify::Verified;
use super::{refused, SuggestDeps};
use crate::browser::challenge::ChallengeFound;
use crate::browser::BridgeError;
use crate::discover::score::excluded_company;
use crate::graph::nodes::{self, Challenge, ChallengeKind, Challenged};
use crate::graph::{GraphDef, END, START};
use crate::health::{classify, Problem};
use crate::store::{NewSuggestion, Source, SourcePurpose};
use crate::Core;

/// The graph's name in the checkpoint store.
pub const SUGGEST_GRAPH: &str = "source_suggest";
/// The one thread of the graph: a new search replaces a finished one, a paused one is continued.
pub const SUGGEST_THREAD: &str = "source_suggest";
/// Most candidates checked by one search.
pub const MAX_VERIFY: usize = 12;
/// Checks running at once.
const VERIFY_CONCURRENCY: usize = 3;
/// The checks read pages in the browser, a few at a time: give them more than a node's default.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(30 * 60);
/// What [`ChallengeFound`] is recorded against: the search has no `sources` row.
const NO_SOURCE: i64 = 0;

/// The state of a search.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SuggestState {
    /// The searches to run, in order.
    pub queries: Vec<String>,
    /// The index of the next query to search.
    pub next: usize,
    /// Candidates the searches found, new to the user, each once (with its query).
    pub found: Vec<Found>,
    /// Candidates the check accepted.
    pub verified: Vec<Checked>,
    /// Candidates the check refused, or that came back known, excluded or without jobs.
    pub dropped: usize,
    /// Suggestions stored as `new` by `save`.
    pub saved: usize,
    /// A consent page, captcha or login the search engine showed: `wait_human` pauses on it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub challenge: Option<Challenge>,
}

impl Challenged for SuggestState {
    fn challenge(&self) -> Option<&Challenge> {
        self.challenge.as_ref()
    }
}

/// A checked candidate and the search that found it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Checked {
    #[serde(flatten)]
    pub source: Verified,
    pub query: String,
}

/// What the nodes share.
struct Shared {
    core: Core,
    deps: SuggestDeps,
}

fn node<F, Fut>(sh: &Arc<Shared>, f: F) -> impl Fn(SuggestState) -> Fut + Send + Sync + 'static
where
    F: Fn(Arc<Shared>, SuggestState) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<SuggestState>> + Send + 'static,
{
    let sh = sh.clone();
    move |s| f(sh.clone(), s)
}

/// The graph over `deps`.
pub(crate) fn graph(core: &Core, deps: SuggestDeps) -> GraphDef<SuggestState> {
    let sh = Arc::new(Shared { core: core.handles(), deps });
    GraphDef::new(SUGGEST_GRAPH)
        .node("plan", node(&sh, plan))
        .node("search", node(&sh, search))
        .wait("wait_human", nodes::challenge::<SuggestState>)
        .node("verify", node(&sh, verify))
        .node("save", node(&sh, save))
        .node_timeout("verify", VERIFY_TIMEOUT)
        .edge(START, "plan")
        .edge("plan", "search")
        // Routes, not edges, back into `search`: a node with several plain incoming edges waits
        // for all of them.
        .route("search", after_search, &["wait_human", "search", "verify"])
        .route("wait_human", |_| "search", &["search"])
        .edge("verify", "save")
        .edge("save", END)
}

fn after_search(s: &SuggestState) -> &'static str {
    if s.challenge.is_some() {
        "wait_human"
    } else if s.next < s.queries.len() {
        "search"
    } else {
        "verify"
    }
}

/// `plan`: the queries from the preferences (or the profile's roles).
async fn plan(sh: Arc<Shared>, _s: SuggestState) -> Result<SuggestState> {
    let prefs = sh.core.store.preferences()?;
    let cv = sh.core.profile()?.map(|p| p.cv);
    let queries = build_queries(&prefs, cv.as_ref())?;
    tracing::info!(queries = queries.len(), "source suggestion search planned");
    Ok(SuggestState { queries, ..SuggestState::default() })
}

/// The transient source a results page is rendered for: no session, so a Private tab.
fn search_source(url: &str) -> Source {
    Source {
        id: NO_SOURCE,
        kind: "web_search".into(),
        name: "Web search".into(),
        url: url.into(),
        purpose: SourcePurpose::Jobs,
        enabled: false,
        session_id: None,
        last_run_at: None,
        last_status: None,
    }
}

/// `search`: the results page of query `next`.
async fn search(sh: Arc<Shared>, mut s: SuggestState) -> Result<SuggestState> {
    let Some(query) = s.queries.get(s.next).cloned() else { return Ok(s) };
    let number = s.next + 1;
    let text = format!("Searching {number} of {}", s.queries.len());
    sh.core.graphs.note(SUGGEST_THREAD, SUGGEST_GRAPH, "search", &text);
    let url = sh.deps.config.url_for(&query)?;
    let page = match sh.deps.pages.rendered(&search_source(url.as_str()), url.as_str()).await {
        Ok(page) => page,
        Err(e) => {
            if let Some(found) = e.downcast_ref::<ChallengeFound>() {
                tracing::warn!(query = number, kind = ?found.kind, "the web search needs the user");
                (sh.deps.notify)("JobPilot needs you", ask_of(found.kind));
                s.challenge = Some(Challenge { kind: found.kind, source_id: NO_SOURCE, url: found.url.clone() });
                return Ok(s);
            }
            // The browser cannot be used, or the network is down: every other query would fail too.
            if is_fatal(&e) {
                return Err(e.context("searching the web"));
            }
            tracing::warn!(query = number, "a results page could not be read, skipped");
            s.next += 1;
            return Ok(s);
        }
    };
    let prefs = sh.core.store.preferences()?;
    let mut known = Known::load(&sh.core.store)?;
    for f in &s.found {
        known.insert(&f.kind, &f.input);
    }
    let fresh = new_sources(&page, &url, &query, &known, &prefs);
    tracing::info!(query = number, found = fresh.len(), "results page read");
    s.found.extend(fresh);
    s.next += 1;
    Ok(s)
}

/// A failure the user has to fix (the browser cannot be used, the network is down, ...): every
/// other read would fail the same way, so the search fails with it (and Retry continues).
fn is_fatal(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<BridgeError>()) || classify(&format!("{e:#}")) != Problem::Other
}

/// What the notification asks of the user.
fn ask_of(kind: ChallengeKind) -> &'static str {
    match kind {
        ChallengeKind::Login => "Suggest sources: the search engine asks you to sign in. Open the browser, then continue.",
        ChallengeKind::Captcha => "Suggest sources: the search engine asks you to confirm in Chrome. Open the browser, then continue.",
        ChallengeKind::Blocked => "Suggest sources: the search engine blocks the search. Look at the page, then continue or cancel.",
    }
}

/// `verify`: check up to [`MAX_VERIFY`] candidates that are still unknown.
async fn verify(sh: Arc<Shared>, mut s: SuggestState) -> Result<SuggestState> {
    let prefs = sh.core.store.preferences()?;
    let mut known = Known::load(&sh.core.store)?;
    let picked: Vec<Found> = s.found.iter().filter(|f| !known.contains(&f.kind, &f.input)).take(MAX_VERIFY).cloned().collect();
    let text = format!("Checking {} sources", picked.len());
    sh.core.graphs.note(SUGGEST_THREAD, SUGGEST_GRAPH, "verify", &text);
    let verifier = sh.deps.verify.clone();
    let checks = picked.into_iter().map(|found| {
        let verifier = verifier.clone();
        async move {
            let result = verifier.verify(&found.kind, &found.input).await;
            (found, result)
        }
    });
    let results: Vec<(Found, Result<Verified>)> = futures::stream::iter(checks).buffered(VERIFY_CONCURRENCY).collect().await;

    // The browser or the network failed: the search fails with that problem (Retry checks again),
    // instead of reading as "nothing found".
    if let Some(e) = results.iter().find_map(|(_, r)| r.as_ref().err().filter(|e| is_fatal(e))) {
        return Err(anyhow::anyhow!("{e:#}").context("checking the sources found"));
    }
    let checked = results.len();
    let (mut refused, mut empty) = (0usize, 0usize);
    let mut skip_later: Vec<(String, String)> = Vec::new();
    s.verified.clear();
    for (found, result) in results {
        let mut source = match result {
            Ok(v) => v,
            Err(_) => {
                refused += 1;
                skip_later.push((found.kind.clone(), found.input.clone()));
                continue;
            }
        };
        if source.jobs == 0 {
            skip_later.push((found.kind.clone(), found.input.clone()));
        }
        if source.name.trim().is_empty() {
            source.name = found.name.clone();
        }
        // No postings, a company the user excludes, or a page that is a source the user has (or
        // another candidate's): nothing to suggest.
        let keep = source.jobs > 0
            && !source.kind.trim().is_empty()
            && !source.url.trim().is_empty()
            && excluded_company(&prefs, &source.name).is_none()
            && known.insert(&source.kind, &source.url);
        if keep {
            s.verified.push(Checked { source, query: found.query });
        } else {
            empty += 1;
        }
    }
    s.dropped = refused + empty;
    // Remembered for a while, so the next searches check other candidates.
    refused::record(&sh.core.store, &skip_later, chrono::Utc::now())?;
    if refused > 0 {
        tracing::warn!(checked, refused, "suggested sources refused by the check");
    }
    tracing::info!(checked, accepted = s.verified.len(), dropped = s.dropped, "suggested sources checked");
    Ok(s)
}

/// `save`: store the checked sources as suggestions.
async fn save(sh: Arc<Shared>, mut s: SuggestState) -> Result<SuggestState> {
    let mut saved = 0usize;
    for c in &s.verified {
        let new = NewSuggestion {
            kind: c.source.kind.clone(),
            input: c.source.url.clone(),
            name: c.source.name.clone(),
            jobs: c.source.jobs,
            matching: c.source.matching,
            query: c.query.clone(),
        };
        if sh.core.store.upsert_suggestion(&new)?.is_some() {
            saved += 1;
        }
    }
    s.saved = saved;
    tracing::info!(saved, "source suggestions saved");
    Ok(s)
}
