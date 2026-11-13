//! P13b - the digest: what the collected items say about where the market is going.
//!
//! ```text
//! (collection, P13a) ... store_items -> wait_human --digest?--> extract -> count -> synthesize
//!                                                                  -> enforce_citations -> save_report -> END
//! START -> extract -> ... -> save_report -> END                    (the digest alone, on stored items)
//! ```
//!
//! - `extract` ([`extract`]): per item of the period `{ topics[], claims[{text, quote}] }`, in
//!   batched model calls with bounded concurrency. Item text is wrapped as UNTRUSTED data; a claim
//!   whose quote is not in the item's text is dropped on the spot. Answers are cached next to the
//!   item's text, so regenerating a digest asks the model only about new items.
//! - `count` ([`count`]): skill frequency per month over hiring-thread postings and leads, counted
//!   in code against a fixed vocabulary (plus the user's own skills); the model never counts.
//! - `synthesize` ([`synthesize`]): one model call: rising and falling skills, active domains,
//!   funding and hiring signals, and what it means for the user's preferences. Each point carries
//!   `citations[]` of item ids and may carry one verbatim `quote`. The evidence in the prompt stays
//!   within [`DigestLimits::evidence_chars`] (fewer claims per item, then fewer items), and only the
//!   items shown can be cited.
//! - `enforce_citations` ([`cite`]): in code, drops every point without a valid citation (an id
//!   that is an item of this digest) and every point whose quote is not in a cited item (a quote
//!   that is a cited item's whole title always counts). It also enforces the prompt's limits: at
//!   most 5 citations per point, 4 points per section, 20 in all.
//! - `save_report` ([`cite`]): one row in `reports` (`weekly` / `quarterly`): `body_md` is the
//!   readable digest (model-written text is escaped, the only links are the cited items' stored
//!   http(s) addresses), `citations_json` is [`DigestData`] (cited items, points, skill counts) for
//!   the view. Regenerating adds a row; the newest wins.
//!
//! Two graphs share these nodes: `trends` (collection, then the digest when the run was started
//! with [`Core::collect_trends_and_digest`]) and `trends_digest` ([`Core::build_trends_digest`]),
//! which only reads what is already stored.
//!
//! # A collector at a challenge does not block the digest
//!
//! A source behind a login or captcha pauses the `trends` thread at `wait_human`, after
//! `store_items` has stored what the other collectors read. The digest reads items from the store,
//! not from the paused run, so the user can either continue (the paused thread then goes on to the
//! digest) or call [`Core::build_trends_digest`] to build it now from what is stored: the two
//! threads are independent (`trends:<period>` and `trends-digest:<period>`).
//!
//! # Rules
//!
//! Item titles, topics, claims and quotes all come from third parties: they reach the model only
//! between markers (with the markers neutralised), the system prompt says so
//! (`assets/prompts/trends.md`), nothing the model returns decides an action or an address (the
//! view opens only the URLs stored with the items), and every point is checked in code. Logs carry
//! ids and counts, never item text.

mod api;
mod cite;
mod count;
mod extract;
mod synthesize;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::collect::TrendRun;
use super::period::Period;
use crate::graph::{GraphDef, END, START};
use crate::llm::{self, Llm};
use crate::store::TrendItem;
use crate::{Secrets, Store};

pub use api::{Digest, TrendChallenge};
pub use count::{is_hiring_posting, month_window, Vocabulary};

/// Name of the digest-only graph in the checkpoint store.
pub const DIGEST_GRAPH: &str = "trends_digest";
/// Slack the library's per-node timeout gets on top of the extraction budget.
const NODE_MARGIN: Duration = Duration::from_secs(120);

/// The thread of a digest built from stored items (the collection has its own, `trends:<period>`).
pub fn digest_thread_id(period: &Period) -> String {
    format!("trends-digest:{}", period.label())
}

// ---------------------------------------------------------------- data

/// One thing an item says, with the words it rests on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claim {
    pub text: String,
    /// Copied from the item; checked in code ([`cite::quote_found`]).
    pub quote: String,
}

/// What the model read from one item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extraction {
    pub item_id: i64,
    #[serde(default)]
    pub topics: Vec<String>,
    #[serde(default)]
    pub claims: Vec<Claim>,
}

/// Where a point belongs in the digest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    RisingSkills,
    FallingSkills,
    Domains,
    Funding,
    Hiring,
    /// What it means for this user's preferences.
    ForYou,
}

impl Section {
    /// In display order.
    pub const ALL: [Section; 6] =
        [Section::RisingSkills, Section::FallingSkills, Section::Domains, Section::Funding, Section::Hiring, Section::ForYou];

    /// As the model and the stored JSON spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Section::RisingSkills => "rising_skills",
            Section::FallingSkills => "falling_skills",
            Section::Domains => "domains",
            Section::Funding => "funding",
            Section::Hiring => "hiring",
            Section::ForYou => "for_you",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|section| section.as_str() == s)
    }

    /// Heading in the view and in `body_md`.
    pub fn title(self) -> &'static str {
        match self {
            Section::RisingSkills => "Rising skills",
            Section::FallingSkills => "Falling skills",
            Section::Domains => "Active domains",
            Section::Funding => "Funding and launches",
            Section::Hiring => "Hiring signals",
            Section::ForYou => "What it means for you",
        }
    }

    /// A point of this section names a skill or a domain the user can add to their preferences.
    pub fn is_suggestible(self) -> bool {
        matches!(self, Section::RisingSkills | Section::Domains)
    }
}

/// One statement of the digest, tied to its sources.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub section: Section,
    /// The skill or domain the point is about; empty when it is not about one.
    #[serde(default)]
    pub topic: String,
    pub text: String,
    /// One verbatim quote from a cited item; empty when the point rests on topics only.
    #[serde(default)]
    pub quote: String,
    /// `trend_items` ids. After `enforce_citations`: at least one, each an item of this digest.
    pub citations: Vec<i64>,
}

/// One skill's frequency over the window of months.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCount {
    pub skill: String,
    /// Documents mentioning the skill, aligned with [`SkillCounts::months`].
    pub by_month: Vec<u32>,
    pub total: u32,
}

/// Skill frequency across leads and hiring-thread postings per month (counted in code).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillCounts {
    /// `2026-06` ... `2026-11`, oldest first; the last one is the period's month.
    pub months: Vec<String>,
    /// Documents (postings and leads) read per month, aligned with `months`.
    pub documents: Vec<u32>,
    /// Most mentioned first, ties by name; only skills that appear at all.
    pub skills: Vec<SkillCount>,
}

/// An item a point cites, kept with the digest so the citation opens even if the item is gone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitedItem {
    pub id: i64,
    pub title: String,
    pub url: String,
}

/// What `reports.citations_json` holds: everything the view needs besides `body_md`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DigestData {
    /// The items the points cite, by first use.
    pub items: Vec<CitedItem>,
    /// Points that passed `enforce_citations`, in section order.
    pub points: Vec<Point>,
    pub counts: SkillCounts,
    /// Items of the period the digest was built from.
    pub items_read: u32,
    /// Points the model wrote that were thrown away (no valid citation, or a quote not in the source).
    pub dropped: u32,
}

/// The digest part of a [`TrendRun`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DigestState {
    /// Build the digest after collecting (the run was started with `collect_trends_and_digest`).
    pub enabled: bool,
    /// Ids of the items the digest is about, newest first.
    pub items: Vec<i64>,
    pub extractions: Vec<Extraction>,
    /// Extraction batches that failed (the digest goes on with the rest).
    pub failed_batches: u32,
    pub counts: Option<SkillCounts>,
    /// The model's points, then (after `enforce_citations`) the ones that were kept.
    pub points: Vec<Point>,
    pub dropped: u32,
    /// The saved `reports` row.
    pub report_id: Option<i64>,
}

// ---------------------------------------------------------------- config

/// Size and time limits of a digest run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestLimits {
    /// Items of the period the digest reads (spread over the sources, newest first).
    pub max_items: usize,
    /// Items per extraction call.
    pub batch_items: usize,
    /// Extraction calls in flight at once.
    pub concurrency: usize,
    /// Characters of one item's text the model sees.
    pub item_chars: usize,
    /// Characters of evidence in the digest prompt, so it fits a small context: the claims per item
    /// shrink first, then the last items are left out of the prompt (and cannot be cited).
    pub evidence_chars: usize,
    /// All extraction calls together may take this long; the batches not done by then are left out.
    pub extract_budget: Duration,
}

impl Default for DigestLimits {
    fn default() -> Self {
        Self { max_items: 120, batch_items: 6, concurrency: 3, item_chars: 4_000, evidence_chars: 20_000, extract_budget: Duration::from_secs(600) }
    }
}

/// What the digest nodes need besides the store: the model and the limits.
#[derive(Clone, Default)]
pub struct DigestConfig {
    /// The model. `None`: the default AI provider, looked up when a node needs it (tests pass a fake).
    pub llm: Option<Arc<dyn Llm>>,
    pub limits: DigestLimits,
}

impl fmt::Debug for DigestConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestConfig").field("llm", &self.llm.as_ref().map(|_| "..")).field("limits", &self.limits).finish()
    }
}

/// What the digest nodes share. Cheap to clone; holds no secret (keys are read when needed).
pub(crate) struct DigestEnv {
    pub store: Store,
    pub secrets: Secrets,
    pub config: DigestConfig,
}

impl DigestEnv {
    /// The configured model, else the default provider with its key from the keyring.
    pub fn llm(&self) -> Result<Arc<dyn Llm>> {
        if let Some(llm) = &self.config.llm {
            return Ok(llm.clone());
        }
        let provider = self
            .store
            .default_provider()?
            .context("no AI provider is set up: add one in Settings > AI providers and make it the default")?;
        let key = self.secrets.get(&llm::api_key_secret(provider.id))?.filter(|k| !k.is_empty());
        Ok(Arc::from(llm::from_provider(&provider, key)))
    }
}

// ---------------------------------------------------------------- graph

/// Add the digest nodes to `graph` (`extract` ... `save_report`, ending at END). The caller connects
/// something to `extract`.
pub(crate) fn add_nodes(graph: GraphDef<TrendRun>, env: Arc<DigestEnv>) -> GraphDef<TrendRun> {
    let timeout = env.config.limits.extract_budget + NODE_MARGIN;
    graph
        .node("extract", {
            let env = env.clone();
            move |s| extract::extract(env.clone(), s)
        })
        .node("count", {
            let env = env.clone();
            move |s| count::count(env.clone(), s)
        })
        .node("synthesize", {
            let env = env.clone();
            move |s| synthesize::synthesize(env.clone(), s)
        })
        .node("enforce_citations", {
            let env = env.clone();
            move |s| cite::enforce_citations(env.clone(), s)
        })
        .node("save_report", {
            let env = env.clone();
            move |s| cite::save_report(env.clone(), s)
        })
        .edge("extract", "count")
        .edge("count", "synthesize")
        .edge("synthesize", "enforce_citations")
        .edge("enforce_citations", "save_report")
        .edge("save_report", END)
        .node_timeout("extract", timeout)
}

/// The digest alone: `START -> extract -> ... -> save_report`, on the items already stored.
pub fn digest_graph(store: Store, secrets: Secrets, config: DigestConfig) -> GraphDef<TrendRun> {
    let env = Arc::new(DigestEnv { store, secrets, config });
    add_nodes(GraphDef::new(DIGEST_GRAPH), env).edge(START, "extract")
}

// ---------------------------------------------------------------- the items of a period

/// The items the digest of `period` is about: published or fetched inside it, spread over the
/// sources (one after the other, newest first) up to `max`, then newest first.
pub(crate) fn period_items(store: &Store, period: &Period, max: usize) -> Result<Vec<TrendItem>> {
    let (start, end) = (period.start(), period.end());
    let inside = |t: DateTime<Utc>| t >= start && t < end;
    let found: Vec<TrendItem> = store
        .list_trend_items(None)?
        .into_iter()
        .filter(|i| i.published_at.is_some_and(inside) || inside(i.fetched_at))
        .collect();
    Ok(spread(found, max))
}

/// Take items round-robin over their sources (the input is newest first) until `max`, then order
/// the pick newest first. Report search has no source row: its items count as one source.
fn spread(items: Vec<TrendItem>, max: usize) -> Vec<TrendItem> {
    if items.len() <= max {
        return items;
    }
    let mut groups: Vec<VecDeque<TrendItem>> = Vec::new();
    let mut index: HashMap<Option<i64>, usize> = HashMap::new();
    for item in items {
        let at = *index.entry(item.source_id).or_insert_with(|| {
            groups.push(VecDeque::new());
            groups.len() - 1
        });
        groups[at].push_back(item);
    }
    let mut picked = Vec::with_capacity(max);
    while picked.len() < max && groups.iter().any(|g| !g.is_empty()) {
        for group in &mut groups {
            if picked.len() == max {
                break;
            }
            if let Some(item) = group.pop_front() {
                picked.push(item);
            }
        }
    }
    picked.sort_by_key(|i| std::cmp::Reverse((i.published_at.unwrap_or(i.fetched_at), i.id)));
    picked
}

/// The saved text of an item; empty when there is none or it cannot be read.
pub(crate) async fn read_text(item: &TrendItem) -> String {
    match &item.text_path {
        Some(path) => tokio::fs::read_to_string(path).await.unwrap_or_default(),
        None => String::new(),
    }
}
