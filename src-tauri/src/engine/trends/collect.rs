//! The `trends` graph: collection and, for a run started with the digest on, the digest
//! ([`super::digest`]).
//!
//! ```text
//! START -> plan -> { hackernews, yc, feeds, report_search } -> store_items -> wait_human
//!       -> END                                                        (collection only)
//!       -> extract -> count -> synthesize -> enforce_citations -> save_report -> END   (with the digest)
//! ```
//!
//! - `plan` reads the enabled trend sources, whether a search API key is saved, and builds the
//!   report queries from the market frame ([`MarketFrame`]: the profile's roles and the preferred
//!   locations, never the wished skills, domains or roles). Only ids, names and queries go into the
//!   state, never a key.
//! - The collectors run in parallel and each writes only its own part of the state: the lists
//!   `items`, `outcomes` and `hn_settled` (append fields) and, for `feeds`, `challenge`. Item text
//!   goes to `<data_dir>/trends/<hash of the URL>.txt` right away, so a checkpoint carries
//!   addresses, not pages.
//! - A slow or failing source never fails the run. Every source gets its own timeout and every
//!   collector a total budget (`CollectLimits`); what runs out is left out of this run and shows up
//!   as [`SourceStatus::TimedOut`] in `outcomes` (and in the source's `last_status`), so
//!   `store_items`, which the library runs once all four collectors are done, waits at most one
//!   budget.
//! - `report_search` is skipped, without a request, when no search API key is saved. The check is
//!   the node's first step, not a route in front of it: the join waits for every incoming edge, so
//!   a skipped branch would hold `store_items` back forever.
//! - `store_items` deduplicates by URL into `trend_items`, then adds the Hacker News hiring
//!   threads read in full (`hn_settled`) to kv [`HN_SETTLED_KEY`], which the next run's
//!   `hackernews` node reads: those threads are not downloaded again. A run that fails or is
//!   cancelled before `store_items` records none; a retry re-runs the failed step.
//! - `wait_human` is the shared challenge interrupt (`graph::nodes::challenge`): a collector that
//!   meets a login or bot wall sets `challenge`; the items of the other sources are stored first,
//!   then the thread pauses until the user continues
//!   ([`Core::resume_trends_collection`](crate::engine::Core)). Only the `page` adapter can detect one,
//!   by status code ([`detect_challenge`]); trends never opens a browser.
//!
//! # The digest
//!
//! After `wait_human` a route (`TrendRun::digest.enabled`) leads to `extract` or to END. The
//! digest nodes read their items from the store, so what the other collectors stored is enough
//! when one source waits at a challenge (see [`super::digest`]).

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::time::Instant;

use super::config::TrendConfig;
use super::digest::{self, DigestConfig, DigestEnv, DigestState};
use super::frame::MarketFrame;
use super::http::{FetchError, Http};
use super::period::Period;
use super::source::report_search::{self, build_queries};
use super::source::{hackernews, page, rss, yc, Ctx, Item, SearchProvider, SourceKind};
use super::text::truncate;
use crate::engine::graph::nodes::{self, Challenge, ChallengeKind, Challenged};
use crate::engine::graph::{GraphDef, END, START};
use crate::engine::pipeline::cv::Cv;
use crate::engine::redact::redact;
use crate::engine::store::{NewTrendItem, Source, SourcePurpose};
use crate::engine::{Paths, Secrets, Store};

/// Name of the graph in the checkpoint store.
pub const GRAPH: &str = "trends";
/// Slack the library's per-node timeout gets on top of a collector's own budget.
const NODE_MARGIN: Duration = Duration::from_secs(30);
/// kv key of the Hacker News "Who is hiring" threads read in full, their postings stored: a JSON
/// array of `hackernews::settled_key`s (`"903@412"`: id and comment count), newest thread first,
/// one per thread. `hackernews::collect` does not download them again while the count is the same
/// (and always reads the newest thread); only `store_items` writes it.
pub const HN_SETTLED_KEY: &str = "trends.hn_settled_threads";
/// How many settled threads kv keeps (the newest): two years of monthly threads, more than any
/// `hiring_threads` window.
const HN_SETTLED_KEEP: usize = 24;

/// The graph thread of a period.
pub fn thread_id(period: &Period) -> String {
    format!("trends:{}", period.label())
}

/// What `plan` decided to collect.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Plan {
    /// `sources` ids of kind `hackernews`, `yc`, and `rss` or `page`.
    pub hackernews: Vec<i64>,
    pub yc: Vec<i64>,
    pub feeds: Vec<i64>,
    /// Set when a search API key is saved.
    pub search: Option<SearchPlan>,
    /// Why there is no [`search`](Self::search).
    pub search_skip: Option<String>,
}

/// Report search: which API, and the queries built from the market frame. Never the key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPlan {
    pub provider: SearchProvider,
    pub queries: Vec<String>,
}

/// An item a collector read: stored by `store_items`, its text already on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collected {
    /// `None` for report search, which has no source row.
    pub source_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub text_path: PathBuf,
}

/// How one source (or the report search) went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SourceStatus {
    Ok,
    /// Not run, with the reason.
    Skipped { reason: String },
    /// Left out of this run: its timeout or its collector's budget ran out.
    TimedOut,
    /// An error (secrets masked, no URL).
    Failed { error: String },
    /// A login, captcha or block page: see [`TrendRun::challenge`].
    NeedsHuman { kind: ChallengeKind },
}

/// The result of one source in one run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceOutcome {
    pub source_id: Option<i64>,
    /// `hackernews`, `yc`, `rss`, `page` or `report_search`.
    pub kind: String,
    pub status: SourceStatus,
    /// New items (not known before this run) the source gave.
    pub items: usize,
}

/// What `store_items` did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stored {
    pub new: usize,
    pub duplicates: usize,
}

/// The state of a `trends` thread.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrendRun {
    pub period: Period,
    #[serde(default)]
    pub plan: Plan,
    /// Append list: what the collectors read.
    #[serde(default)]
    pub items: Vec<Collected>,
    /// Append list: one entry per source run (and per skipped collector).
    #[serde(default)]
    pub outcomes: Vec<SourceOutcome>,
    #[serde(default)]
    pub stored: Stored,
    /// Set by `feeds` when a source needs the user; cleared by "Continue".
    #[serde(default)]
    pub challenge: Option<Challenge>,
    /// The digest: whether to build it, and what its nodes made.
    #[serde(default)]
    pub digest: DigestState,
    /// Append list: Hacker News hiring threads (ids) read in full this run (`hackernews::Hn::settled`);
    /// `store_items` records them in kv [`HN_SETTLED_KEY`] once their postings are stored.
    #[serde(default)]
    pub hn_settled: Vec<String>,
}

impl TrendRun {
    pub fn new(period: Period) -> Self {
        Self {
            period,
            plan: Plan::default(),
            items: Vec::new(),
            outcomes: Vec::new(),
            stored: Stored::default(),
            challenge: None,
            digest: DigestState::default(),
            hn_settled: Vec::new(),
        }
    }

    /// Outcomes of one adapter kind.
    pub fn outcomes_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a SourceOutcome> {
        self.outcomes.iter().filter(move |o| o.kind == kind)
    }
}

impl Challenged for TrendRun {
    fn challenge(&self) -> Option<&Challenge> {
        self.challenge.as_ref()
    }
}

/// What the nodes share. Cheap to clone; holds no secret (keys are read when needed).
struct Env {
    store: Store,
    secrets: Secrets,
    paths: Paths,
    config: Arc<TrendConfig>,
    http: Http,
    /// The profile the market frame reads the background from (`DigestConfig::profile`);
    /// `None`: no background, the queries come from the place alone.
    profile: Option<Cv>,
}

impl Env {
    fn ctx(&self, known: &Arc<HashSet<String>>, period: Period) -> Ctx {
        Ctx { http: self.http.clone(), config: self.config.clone(), known: known.clone(), period }
    }
}

/// The graph (see the module docs). Rebuild it with the same `config` to resume a thread.
pub fn graph(
    store: Store,
    secrets: Secrets,
    paths: Paths,
    config: TrendConfig,
    digest: DigestConfig,
) -> Result<GraphDef<TrendRun>> {
    let http = Http::new(&config.limits.http())?;
    let collector_timeout = config.limits.collector_budget + NODE_MARGIN;
    let profile = digest.profile.clone();
    let digest_env = Arc::new(DigestEnv { store: store.clone(), secrets: secrets.clone(), config: digest });
    let env = Arc::new(Env { store, secrets, paths, config: Arc::new(config), http, profile });
    let mut graph = GraphDef::new(GRAPH).append("items").append("outcomes").append("hn_settled").node("plan", {
        let env = env.clone();
        move |s| plan(env.clone(), s)
    });
    for (name, kind) in [("hackernews", SourceKind::HackerNews), ("yc", SourceKind::Yc)] {
        let env = env.clone();
        graph = graph.node(name, move |s| group(env.clone(), s, kind));
    }
    let graph = graph
        .node("feeds", {
            let env = env.clone();
            move |s| group(env.clone(), s, SourceKind::Rss)
        })
        .node("report_search", {
            let env = env.clone();
            move |s| report_search_node(env.clone(), s)
        })
        .node("store_items", {
            let env = env.clone();
            move |s| store_items(env.clone(), s)
        })
        .wait("wait_human", nodes::challenge::<TrendRun>)
        .edge(START, "plan")
        .parallel("plan", &["hackernews", "yc", "feeds", "report_search"], "store_items")
        .edge("store_items", "wait_human")
        .route("wait_human", |s| if s.digest.enabled { "extract" } else { END }, &["extract", END])
        .node_timeout("hackernews", collector_timeout)
        .node_timeout("yc", collector_timeout)
        .node_timeout("feeds", collector_timeout)
        .node_timeout("report_search", collector_timeout);
    Ok(digest::add_nodes(graph, digest_env))
}

// ---------------------------------------------------------------- plan

async fn plan(env: Arc<Env>, mut s: TrendRun) -> Result<TrendRun> {
    let mut plan = Plan::default();
    for source in env.store.list_enabled_sources(SourcePurpose::Trends)? {
        match SourceKind::parse(&source.kind) {
            Some(SourceKind::HackerNews) => plan.hackernews.push(source.id),
            Some(SourceKind::Yc) => plan.yc.push(source.id),
            Some(SourceKind::Rss | SourceKind::Page) => plan.feeds.push(source.id),
            None => tracing::warn!(source_id = source.id, "a trend source of an unknown kind is ignored"),
        }
    }
    match super::search_provider(&env.secrets) {
        Ok(Some(provider)) => {
            let prefs = env.store.preferences().unwrap_or_else(|_| {
                tracing::warn!("the saved preferences could not be read; the queries use no place");
                Default::default()
            });
            // Background and place only: the frame reads no wish from the preferences.
            let frame = MarketFrame::new(env.profile.as_ref(), &prefs);
            let queries = build_queries(&frame, &s.period, env.config.limits.max_queries);
            plan.search = Some(SearchPlan { provider, queries });
        }
        Ok(None) => plan.search_skip = Some("no search API key saved".into()),
        Err(_) => {
            tracing::warn!("the keyring could not be read; report search is skipped");
            plan.search_skip = Some("the keyring could not be read".into());
        }
    }
    tracing::info!(
        period = %s.period.label(),
        hackernews = plan.hackernews.len(),
        yc = plan.yc.len(),
        feeds = plan.feeds.len(),
        queries = plan.search.as_ref().map_or(0, |p| p.queries.len()),
        "trend collection planned"
    );
    s.plan = plan;
    Ok(s)
}

// ---------------------------------------------------------------- collectors

/// The `hackernews`, `yc` and `feeds` nodes: run every planned source of the group, one after the
/// other (they share the per-host delay), each with its own timeout.
async fn group(env: Arc<Env>, mut s: TrendRun, group: SourceKind) -> Result<TrendRun> {
    let ids = match group {
        SourceKind::HackerNews => s.plan.hackernews.clone(),
        SourceKind::Yc => s.plan.yc.clone(),
        SourceKind::Rss | SourceKind::Page => s.plan.feeds.clone(),
    };
    if ids.is_empty() {
        return Ok(s);
    }
    let deadline = Instant::now() + env.config.limits.collector_budget;
    let known = Arc::new(known_urls(&env.store)?);
    let ctx = env.ctx(&known, s.period);
    let saved_settled = if group == SourceKind::HackerNews { settled_threads(&env.store) } else { HashSet::new() };
    for id in ids {
        // Deleted since the plan, or not of this group any more: nothing to do.
        let Some(source) = env.store.get_source(id)?.filter(|src| src.enabled) else { continue };
        let Some(kind) = SourceKind::parse(&source.kind) else { continue };
        // Hacker News also names the hiring threads it read in full (set inside `work`).
        let mut settled = Vec::new();
        let work = async {
            match kind {
                SourceKind::HackerNews => {
                    hackernews::collect(&ctx, &saved_settled).await.map(|hn| {
                        settled = hn.settled;
                        hn.items
                    })
                }
                SourceKind::Yc => yc::collect(&ctx).await,
                SourceKind::Rss => rss::collect(&ctx, &source.url).await,
                SourceKind::Page => page::collect(&ctx, &source.url).await,
            }
        };
        let done = attempt(&env, deadline, Some(&source), kind.as_str(), work).await;
        // Only when the postings are in `items`: not after a timeout or a failed write.
        if done.outcome.status == SourceStatus::Ok {
            s.hn_settled.extend(settled);
        }
        done.apply(&mut s);
    }
    Ok(s)
}

/// The settled hiring threads saved in kv [`HN_SETTLED_KEY`]. Unreadable or invalid: a warning,
/// and none (every thread in the window is read).
fn settled_threads(store: &Store) -> HashSet<String> {
    match load_settled(store) {
        Ok(keys) => keys.into_iter().collect(),
        Err(_) => {
            tracing::warn!("the settled hiring threads could not be read; every hiring thread is read");
            HashSet::new()
        }
    }
}

/// The saved keys in their saved order (newest thread first), with a store error as an error; a
/// value that is not a list of strings is a warning and none.
fn load_settled(store: &Store) -> Result<Vec<String>> {
    let Some(json) = store.kv_get(HN_SETTLED_KEY)? else { return Ok(Vec::new()) };
    match serde_json::from_str::<Vec<String>>(&json) {
        Ok(keys) => Ok(keys),
        Err(_) => {
            tracing::warn!("the saved settled hiring threads are not a list of keys and are ignored");
            Ok(Vec::new())
        }
    }
}

async fn report_search_node(env: Arc<Env>, mut s: TrendRun) -> Result<TrendRun> {
    let skip = |s: &mut TrendRun, reason: &str| {
        tracing::info!("report search skipped: {reason}");
        s.outcomes.push(SourceOutcome {
            source_id: None,
            kind: "report_search".into(),
            status: SourceStatus::Skipped { reason: reason.into() },
            items: 0,
        });
    };
    let Some(search) = s.plan.search.clone() else {
        let reason = s.plan.search_skip.clone().unwrap_or_else(|| "no search API key saved".into());
        skip(&mut s, &reason);
        return Ok(s);
    };
    // The key is read now and lives only in this function: not in the state, not in a log.
    let key = match env.secrets.get(search.provider.secret_name()) {
        Ok(Some(key)) if !key.trim().is_empty() => key.trim().to_string(),
        _ => {
            skip(&mut s, "no search API key saved");
            return Ok(s);
        }
    };
    let deadline = Instant::now() + env.config.limits.collector_budget;
    let known = Arc::new(known_urls(&env.store)?);
    let ctx = env.ctx(&known, s.period);
    let work = report_search::collect(&ctx, search.provider, &key, &search.queries);
    attempt(&env, deadline, None, "report_search", work).await.apply(&mut s);
    Ok(s)
}

/// What one source did, before it is applied to the state.
struct Attempt {
    outcome: SourceOutcome,
    items: Vec<Collected>,
    challenge: Option<Challenge>,
}

impl Attempt {
    fn apply(self, s: &mut TrendRun) {
        s.items.extend(self.items);
        s.outcomes.push(self.outcome);
        if s.challenge.is_none() {
            s.challenge = self.challenge;
        }
    }
}

/// Run `work` for a source under its timeout (and what is left of the collector's budget), save the
/// text of what it found, and record the result on the source row. Never fails: whatever goes
/// wrong is the source's outcome.
async fn attempt<Fut>(env: &Env, deadline: Instant, source: Option<&Source>, kind: &str, work: Fut) -> Attempt
where
    Fut: Future<Output = Result<Vec<Item>>>,
{
    let started = Instant::now();
    let budget = deadline.saturating_duration_since(started).min(env.config.limits.source_timeout);
    let result = if budget.is_zero() { None } else { tokio::time::timeout(budget, work).await.ok() };

    let mut items = Vec::new();
    let mut challenge = None;
    let status = match result {
        None => SourceStatus::TimedOut,
        Some(Err(e)) => match (detect_challenge(kind, &e), source) {
            (Some(found), Some(source)) => {
                challenge = Some(Challenge { kind: found, source_id: source.id, url: source.url.clone() });
                SourceStatus::NeedsHuman { kind: found }
            }
            _ => SourceStatus::Failed { error: short(&e) },
        },
        Some(Ok(found)) => match write_items(env, source, found).await {
            Ok(written) => {
                items = written;
                SourceStatus::Ok
            }
            Err(e) => SourceStatus::Failed { error: short(&e) },
        },
    };

    let source_id = source.map(|s| s.id);
    if let Some(id) = source_id {
        let recorded = match &status {
            SourceStatus::Ok => env.store.set_source_run(id, "ok"),
            SourceStatus::TimedOut => env.store.set_source_run(id, "timed out"),
            SourceStatus::Failed { error } => env.store.set_source_run(id, &format!("failed: {error}")),
            SourceStatus::NeedsHuman { .. } => env.store.set_source_status(id, "needs_human"),
            SourceStatus::Skipped { .. } => Ok(()),
        };
        if recorded.is_err() {
            tracing::warn!(source_id = id, "could not record the source's status");
        }
    }
    let label = match &status {
        SourceStatus::Ok => "ok",
        SourceStatus::Skipped { .. } => "skipped",
        SourceStatus::TimedOut => "timed_out",
        SourceStatus::Failed { .. } => "failed",
        SourceStatus::NeedsHuman { .. } => "needs_human",
    };
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if matches!(status, SourceStatus::Failed { .. } | SourceStatus::TimedOut) {
        tracing::warn!(source_id = source_id.unwrap_or(0), kind, status = label, ms, "trend source left out");
    } else {
        tracing::info!(source_id = source_id.unwrap_or(0), kind, status = label, items = items.len(), ms, "trend source collected");
    }
    let count = items.len();
    Attempt { outcome: SourceOutcome { source_id, kind: kind.into(), status, items: count }, items, challenge }
}

/// Tells a login or bot wall from an ordinary failure, by status code (no browser). API adapters
/// (Hacker News, YC, feeds, report search) never pause the run; a `page` source that answers 401
/// (login) or 403, 429, 503 (blocked) does.
pub fn detect_challenge(kind: &str, error: &anyhow::Error) -> Option<ChallengeKind> {
    if kind != SourceKind::Page.as_str() {
        return None;
    }
    match error.chain().find_map(|e| e.downcast_ref::<FetchError>())? {
        FetchError::Status(401) => Some(ChallengeKind::Login),
        FetchError::Status(403 | 429 | 503) => Some(ChallengeKind::Blocked),
        _ => None,
    }
}

fn short(error: &anyhow::Error) -> String {
    truncate(redact(&format!("{error:#}")).as_ref(), 200)
}

fn known_urls(store: &Store) -> Result<HashSet<String>> {
    Ok(store.list_trend_items(None)?.into_iter().map(|i| i.url).collect())
}

/// `<trends dir>/<32 hex of sha256(url)>.txt`: the same address always lands in the same file.
fn text_path(paths: &Paths, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    paths.trends_dir().join(format!("{name}.txt"))
}

/// Save each item's text (cut at `max_text_chars`) and describe it for `store_items`.
async fn write_items(env: &Env, source: Option<&Source>, found: Vec<Item>) -> Result<Vec<Collected>> {
    let dir = env.paths.trends_dir();
    tokio::fs::create_dir_all(&dir).await.with_context(|| format!("creating {}", dir.display()))?;
    let max = env.config.limits.max_text_chars;
    let mut written = Vec::with_capacity(found.len());
    for item in found {
        let path = text_path(&env.paths, &item.url);
        let tmp = path.with_extension("txt.tmp");
        tokio::fs::write(&tmp, super::text::cap(&item.text, max)).await.with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &path).await.with_context(|| format!("saving {}", path.display()))?;
        written.push(Collected {
            source_id: source.map(|s| s.id),
            url: item.url,
            title: item.title,
            published_at: item.published_at,
            text_path: path,
        });
    }
    Ok(written)
}

// ---------------------------------------------------------------- store_items

/// Deduplicate by URL into `trend_items`. Items of a source deleted during the run are stored
/// without a source.
async fn store_items(env: Arc<Env>, mut s: TrendRun) -> Result<TrendRun> {
    let mut stored = Stored::default();
    let mut live: std::collections::HashMap<i64, bool> = std::collections::HashMap::new();
    for item in &s.items {
        let source_id = match item.source_id {
            Some(id) => {
                let exists = match live.get(&id) {
                    Some(known) => *known,
                    None => {
                        let exists = env.store.get_source(id)?.is_some();
                        live.insert(id, exists);
                        exists
                    }
                };
                exists.then_some(id)
            }
            None => None,
        };
        let new = NewTrendItem {
            source_id,
            url: item.url.clone(),
            title: item.title.clone(),
            published_at: item.published_at,
            text_path: Some(item.text_path.clone()),
        };
        match env.store.add_trend_item(&new)? {
            Some(_) => stored.new += 1,
            None => stored.duplicates += 1,
        }
    }
    tracing::info!(period = %s.period.label(), new = stored.new, duplicates = stored.duplicates, "trend items stored");
    s.stored = stored;
    // After every item: a thread is recorded only once its postings are stored. A failure costs a
    // download next run, not the run.
    if record_settled(&env.store, &s.hn_settled).is_err() {
        tracing::warn!("could not record the hiring threads read in full; they are read again next run");
    }
    Ok(s)
}

/// Merge `new` into kv [`HN_SETTLED_KEY`]: one key per thread (a new key replaces the saved one of
/// its thread; among saved keys of one thread the first wins), the [`HN_SETTLED_KEEP`] newest
/// threads (numeric order of the ids). A key that is not a `hackernews::settled_key` is dropped, at
/// the next write. Writes only when the list changes.
pub(super) fn record_settled(store: &Store, new: &[String]) -> Result<()> {
    if new.is_empty() {
        return Ok(());
    }
    let saved = load_settled(store)?;
    let mut by_thread: std::collections::BTreeMap<u64, &String> = std::collections::BTreeMap::new();
    for key in &saved {
        if let Some(thread) = hackernews::settled_thread(key) {
            by_thread.entry(thread).or_insert(key);
        }
    }
    for key in new {
        if let Some(thread) = hackernews::settled_thread(key) {
            by_thread.insert(thread, key);
        }
    }
    let keys: Vec<String> = by_thread.into_values().rev().take(HN_SETTLED_KEEP).cloned().collect();
    if keys.len() == saved.len() && keys.iter().all(|k| saved.contains(k)) {
        return Ok(());
    }
    store.kv_set(HN_SETTLED_KEY, &serde_json::to_string(&keys)?)?;
    tracing::info!(kept = keys.len(), "hiring threads read in full recorded");
    Ok(())
}
