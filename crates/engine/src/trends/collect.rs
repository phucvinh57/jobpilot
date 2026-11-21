//! The `trends` graph: collection and, for a run started with the digest on, the digest
//! ([`super::digest`]).
//!
//! ```text
//! START -> plan -> { hackernews, yc, feeds, people } -> store_items -> wait_human
//!       -> END                                                        (collection only)
//!       -> count -> extract -> synthesize -> enforce_citations -> save_report -> END   (with the digest)
//! ```
//!
//! - `plan` reads the enabled trend sources. Only their ids go into the state.
//! - The collectors run in parallel and each writes only its own part of the state: the lists
//!   `items`, `outcomes` and `hn_settled` (append fields) and, for `feeds`, `challenge`. Item text
//!   goes to `<data_dir>/trends/<hash of the URL>.txt` right away, so a checkpoint carries
//!   addresses, not pages.
//! - A slow or failing source never fails the run. Every source gets its own timeout and every
//!   collector a total budget (`CollectLimits`); what runs out is left out of this run and shows up
//!   as [`SourceStatus::TimedOut`] in `outcomes` (and in the source's `last_status`), so
//!   `store_items`, which the library runs once all the collectors are done, waits at most one
//!   budget.
//! - `people` reads the sources of followed people (`source::person`). A story that names a
//!   person is often stored already, by the Hacker News source, with its title alone: for this
//!   collector only the items of people's sources count as known, so the linked interview is read
//!   once, and `store_items` hands the stored item to the person's source (with the fuller text).
//! - `store_items` deduplicates by URL into `trend_items`, then adds the Hacker News hiring
//!   threads read in full (`hn_settled`) to kv [`HN_SETTLED_KEY`], which the next run's
//!   `hackernews` node reads: those threads are not downloaded again. A run that fails or is
//!   cancelled before `store_items` records none; a retry re-runs the failed step.
//! - `wait_human` is the shared challenge interrupt (`graph::nodes::challenge`): a collector that
//!   meets a login or bot wall sets `challenge`; the items of the other sources are stored first,
//!   then the thread pauses until the user continues
//!   ([`Core::resume_trends_collection`](crate::Core)). Only the `page` adapter can detect one,
//!   by status code ([`detect_challenge`]); trends never opens a browser.
//!
//! # The digest
//!
//! After `wait_human` a route (`TrendRun::digest.enabled`) leads to `count` or to END. The
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
use super::http::{FetchError, Http};
use super::period::Period;
use super::source::{hackernews, page, person, rss, yc, Ctx, Item, SourceKind};
use super::text::truncate;
use crate::graph::nodes::{self, Challenge, ChallengeKind, Challenged};
use crate::graph::{GraphDef, END, START};
use crate::redact::redact;
use crate::store::{NewTrendItem, Source, SourcePurpose};
use crate::{Paths, Secrets, Store};

/// Name of the graph in the checkpoint store.
pub const GRAPH: &str = "trends";
/// Slack the library's per-node timeout gets on top of a collector's own budget.
const NODE_MARGIN: Duration = Duration::from_secs(30);
/// kv key of the Hacker News "Who is hiring" threads read in full, their postings stored: a JSON
/// array of `hackernews::settled_key`s (`"903@412"`: id and comment count), newest thread first,
/// one per thread. `hackernews::collect` does not download them again while the count is the same
/// (and always reads the newest thread); only `store_items` writes it.
pub const HN_SETTLED_KEY: &str = "trends.hn_settled_threads";
/// How many settled threads kv keeps (the newest): two years of monthly threads, as many as the
/// widest window (`CollectLimits::hiring_threads_for` a year).
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
    /// `sources` ids of kind `person`.
    pub people: Vec<i64>,
}

/// An item a collector read: stored by `store_items`, its text already on disk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Collected {
    /// Always set by the collectors (an `Option` so the saved runs keep loading).
    pub source_id: Option<i64>,
    pub url: String,
    pub title: String,
    pub published_at: Option<DateTime<Utc>>,
    pub text_path: PathBuf,
}

/// How one source went.
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
    /// `hackernews`, `yc`, `rss`, `page` or `person`.
    pub kind: String,
    pub status: SourceStatus,
    /// New items (not known before this run) the source gave.
    pub items: usize,
    /// New items the source exposed but this run left unread: past a cap (`max_items`, the Hacker
    /// News page cap) or its time budget. Counted, never silent; what a source cannot count (a
    /// hiring thread left unread) counts as 1. 0 when nothing was cut, and for a source that did
    /// not finish. A checkpoint saved before this field existed loads with 0.
    #[serde(default)]
    pub cut: usize,
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

/// What the nodes share. Cheap to clone; holds no secret.
struct Env {
    store: Store,
    paths: Paths,
    config: Arc<TrendConfig>,
    http: Http,
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
    let digest_env = Arc::new(DigestEnv { store: store.clone(), secrets, config: digest });
    let env = Arc::new(Env { store, paths, config: Arc::new(config), http });
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
        .node("people", {
            let env = env.clone();
            move |s| group(env.clone(), s, SourceKind::Person)
        })
        .node("store_items", {
            let env = env.clone();
            move |s| store_items(env.clone(), s)
        })
        .wait("wait_human", nodes::challenge::<TrendRun>)
        .edge(START, "plan")
        .parallel("plan", &["hackernews", "yc", "feeds", "people"], "store_items")
        .edge("store_items", "wait_human")
        .route("wait_human", |s| if s.digest.enabled { "count" } else { END }, &["count", END])
        .node_timeout("hackernews", collector_timeout)
        .node_timeout("yc", collector_timeout)
        .node_timeout("feeds", collector_timeout)
        .node_timeout("people", collector_timeout);
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
            Some(SourceKind::Person) => plan.people.push(source.id),
            None => tracing::warn!(source_id = source.id, "a trend source of an unknown kind is ignored"),
        }
    }
    tracing::info!(
        period = %s.period.label(),
        hackernews = plan.hackernews.len(),
        yc = plan.yc.len(),
        feeds = plan.feeds.len(),
        people = plan.people.len(),
        "trend collection planned"
    );
    s.plan = plan;
    Ok(s)
}

// ---------------------------------------------------------------- collectors

/// The `hackernews`, `yc`, `feeds` and `people` nodes: run every planned source of the group, one after the
/// other (they share the per-host delay), each with its own timeout.
async fn group(env: Arc<Env>, mut s: TrendRun, group: SourceKind) -> Result<TrendRun> {
    let ids = match group {
        SourceKind::HackerNews => s.plan.hackernews.clone(),
        SourceKind::Yc => s.plan.yc.clone(),
        SourceKind::Rss | SourceKind::Page => s.plan.feeds.clone(),
        SourceKind::Person => s.plan.people.clone(),
    };
    if ids.is_empty() {
        return Ok(s);
    }
    let deadline = Instant::now() + env.config.limits.collector_budget;
    // A person's source reads again what another source stored with a title alone (module docs).
    let known = if group == SourceKind::Person { env.store.person_item_urls()?.into_iter().collect() } else { known_urls(&env.store)? };
    let known = Arc::new(known);
    let ctx = env.ctx(&known, s.period);
    let saved_settled = if group == SourceKind::HackerNews { settled_threads(&env.store) } else { HashSet::new() };
    for id in ids {
        // Deleted since the plan, or not of this group any more: nothing to do.
        let Some(source) = env.store.get_source(id)?.filter(|src| src.enabled) else { continue };
        let Some(kind) = SourceKind::parse(&source.kind) else { continue };
        // Hacker News also names the hiring threads it read in full (set inside `work`). Every
        // adapter gives its new items and how many more a cap cut (a page is one item: none).
        let mut settled = Vec::new();
        let work = async {
            match kind {
                SourceKind::HackerNews => {
                    hackernews::collect(&ctx, &saved_settled).await.map(|hn| {
                        settled = hn.settled;
                        (hn.items, hn.cut)
                    })
                }
                SourceKind::Yc => yc::collect(&ctx).await,
                SourceKind::Rss => rss::collect(&ctx, &source.url).await,
                SourceKind::Page => page::collect(&ctx, &source.url).await.map(|items| (items, 0)),
                SourceKind::Person => person::collect(&ctx, &source.name, &source.url).await,
            }
        };
        let done = attempt(&env, deadline, &source, kind.as_str(), work).await;
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

/// Run `work` (the new items and how many more a cap cut) for a source under its timeout (and what
/// is left of the collector's budget), save the text of what it found, and record the result on the
/// source row. Never fails: whatever goes wrong is the source's outcome.
async fn attempt<Fut>(env: &Env, deadline: Instant, source: &Source, kind: &str, work: Fut) -> Attempt
where
    Fut: Future<Output = Result<(Vec<Item>, usize)>>,
{
    let started = Instant::now();
    let budget = deadline.saturating_duration_since(started).min(env.config.limits.source_timeout);
    let result = if budget.is_zero() { None } else { tokio::time::timeout(budget, work).await.ok() };

    let mut items = Vec::new();
    let mut cut = 0;
    let mut challenge = None;
    let status = match result {
        None => SourceStatus::TimedOut,
        Some(Err(e)) => match detect_challenge(kind, &e) {
            Some(found) => {
                challenge = Some(Challenge { kind: found, source_id: source.id, url: source.url.clone() });
                SourceStatus::NeedsHuman { kind: found }
            }
            None => SourceStatus::Failed { error: short(&e) },
        },
        Some(Ok((found, unread))) => match write_items(env, source, found).await {
            Ok(written) => {
                items = written;
                cut = unread;
                SourceStatus::Ok
            }
            Err(e) => SourceStatus::Failed { error: short(&e) },
        },
    };

    let source_id = source.id;
    let recorded = match &status {
        SourceStatus::Ok => env.store.set_source_run(source_id, "ok"),
        SourceStatus::TimedOut => env.store.set_source_run(source_id, "timed out"),
        SourceStatus::Failed { error } => env.store.set_source_run(source_id, &format!("failed: {error}")),
        SourceStatus::NeedsHuman { .. } => env.store.set_source_status(source_id, "needs_human"),
        SourceStatus::Skipped { .. } => Ok(()),
    };
    if recorded.is_err() {
        tracing::warn!(source_id, "could not record the source's status");
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
        tracing::warn!(source_id, kind, status = label, ms, "trend source left out");
    } else {
        tracing::info!(source_id, kind, status = label, items = items.len(), cut, ms, "trend source collected");
    }
    let count = items.len();
    Attempt { outcome: SourceOutcome { source_id: Some(source_id), kind: kind.into(), status, items: count, cut }, items, challenge }
}

/// Tells a login or bot wall from an ordinary failure, by status code (no browser). API adapters
/// (Hacker News, YC, feeds) never pause the run; a `page` source that answers 401
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

/// `<trends dir>/<32 hex of sha256(key)>.txt`: the same address always lands in the same file.
fn text_path(paths: &Paths, url: &str) -> PathBuf {
    let digest = Sha256::digest(url.as_bytes());
    let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
    paths.trends_dir().join(format!("{name}.txt"))
}

/// Save each item's text (cut at `max_text_chars`) and describe it for `store_items`.
async fn write_items(env: &Env, source: &Source, found: Vec<Item>) -> Result<Vec<Collected>> {
    let dir = env.paths.trends_dir();
    tokio::fs::create_dir_all(&dir).await.with_context(|| format!("creating {}", dir.display()))?;
    let max = env.config.limits.max_text_chars;
    let mut written = Vec::with_capacity(found.len());
    // A person's source keeps its own file per address: the same story may be written at the same
    // moment by the Hacker News collector, with its title alone.
    let person = source.kind == SourceKind::Person.as_str();
    for item in found {
        let path = if person { text_path(&env.paths, &format!("person:{}", item.url)) } else { text_path(&env.paths, &item.url) };
        let tmp = path.with_extension("txt.tmp");
        tokio::fs::write(&tmp, super::text::cap(&item.text, max)).await.with_context(|| format!("writing {}", tmp.display()))?;
        tokio::fs::rename(&tmp, &path).await.with_context(|| format!("saving {}", path.display()))?;
        written.push(Collected {
            source_id: Some(source.id),
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
/// without a source. An item a person's source read that is stored already goes to that source
/// (`Store::claim_trend_item_for_person`); it still counts as a duplicate.
async fn store_items(env: Arc<Env>, mut s: TrendRun) -> Result<TrendRun> {
    let mut stored = Stored::default();
    // Per source of the run: is it still there, and is it a person's.
    let mut live: std::collections::HashMap<i64, Option<bool>> = std::collections::HashMap::new();
    for item in &s.items {
        let (source_id, person) = match item.source_id {
            Some(id) => {
                let found = match live.get(&id) {
                    Some(known) => *known,
                    None => {
                        let found = env.store.get_source(id)?.map(|source| source.kind == SourceKind::Person.as_str());
                        live.insert(id, found);
                        found
                    }
                };
                (found.map(|_| id), found == Some(true))
            }
            None => (None, false),
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
            None => {
                stored.duplicates += 1;
                if person {
                    env.store.claim_trend_item_for_person(&new)?;
                }
            }
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
