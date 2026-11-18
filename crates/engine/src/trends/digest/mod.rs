//! The digest: what the collected items say about where the market is going.
//!
//! ```text
//! (collection)       ... store_items -> wait_human --digest?--> count -> extract -> synthesize
//!                                                                  -> enforce_citations -> save_report -> END
//! START -> count -> extract -> ... -> save_report -> END           (the digest alone, on stored items)
//! ```
//!
//! - `count` ([`count`]): everything the code can measure, over EVERY stored item of the period and
//!   never over a sample. Skill frequency per month over hiring-thread postings and leads, counted
//!   against a fixed vocabulary (plus the user's own skills); the direction of a skill is tested
//!   in code too: the period's months against the months before them and the trend over all
//!   months, under one control of false discoveries per scope. New terms the vocabulary does not
//!   know come from the words of every item ([`terms`]), per kind of item ([`Stratum`]), ranked
//!   by a test where the volumes allow one. The model never counts. The counts also say what was
//!   not seen ([`Coverage`]); [`Coverage::lines`] puts it in words, the same for the prompt, the
//!   markdown, the PDF and the screen, next to each skill's level in the period's own months with
//!   its 95% range ([`level_text`]).
//! - `extract` ([`extract`]): the model reads a capped pick of the period's items, chosen so that
//!   it can cite the indicators `count` found ([`pick`]): per item `{ topics[], claims[{text,
//!   quote}] }`, in batched model calls with bounded concurrency. Item text is wrapped as
//!   UNTRUSTED data; a claim whose quote is not in the item's text is dropped on the spot. Answers
//!   are cached next to the item's text, so regenerating a digest asks the model only about new
//!   items.
//! - `synthesize` ([`synthesize`]): one model call: rising and falling skills, active domains,
//!   funding and hiring signals, and what it means for the user's preferences. Each point carries
//!   `citations[]` of item ids and may carry one verbatim `quote`. The evidence in the prompt stays
//!   within [`DigestLimits::evidence_chars`] (fewer claims per item, then fewer items), and only the
//!   items shown can be cited.
//! - `enforce_citations` ([`cite`]): in code, drops every point without a valid citation (an id
//!   that is an item of this digest) and every point whose quote is not in a cited item (a quote
//!   that is a cited item's whole title always counts). It also enforces the prompt's limits: at
//!   most 5 citations per point, 4 points per section, 20 in all.
//! - `save_report` ([`cite`]): one row in `reports` per period (`weekly` / `monthly` /
//!   `quarterly` / `yearly`): `body_md` is the readable digest (model-written text is escaped, the
//!   only links are the cited items' stored http(s) addresses), `citations_json` is [`DigestData`]
//!   (cited items, points, skill counts) for the view. Rerunning a period replaces its report (same
//!   id). [`crate::trends::pdf`] exports one as a PDF.
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
pub(crate) mod cite;
mod count;
mod extract;
mod pick;
mod stats;
mod synthesize;
mod terms;
#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::collect::TrendRun;
use super::period::Period;
use super::source::SourceKind;
use crate::graph::{GraphDef, END, START};
use crate::llm::{self, Llm};
use crate::pipeline::cv::Cv;
use crate::store::{SourcePurpose, TrendItem};
use crate::{Secrets, Store};

pub use api::{Digest, TrendChallenge};
pub use count::{is_hiring_posting, month_window, Vocabulary};
/// The statistics behind the counts, for the sample data of the shell to make real numbers with.
pub use stats::{centi, log_odds_z, trend_z, wilson};

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
    /// The gap: what the market means for this user's background (build on, learn next, at risk).
    ForYou,
    /// The user's wishes against the market, applied last.
    WishesCheck,
}

impl Section {
    /// In display order.
    pub const ALL: [Section; 7] = [
        Section::RisingSkills,
        Section::FallingSkills,
        Section::Domains,
        Section::Funding,
        Section::Hiring,
        Section::ForYou,
        Section::WishesCheck,
    ];

    /// As the model and the stored JSON spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Section::RisingSkills => "rising_skills",
            Section::FallingSkills => "falling_skills",
            Section::Domains => "domains",
            Section::Funding => "funding",
            Section::Hiring => "hiring",
            Section::ForYou => "for_you",
            Section::WishesCheck => "wishes_check",
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
            Section::ForYou => "Your gap: build on, learn next, at risk",
            Section::WishesCheck => "Your wishes vs the market",
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

/// Where a skill's share of the postings is going, decided in code by tests (`stats::call_direction`):
/// the period's months against the months before them, and the trend over every month. A movement
/// is called only when it survives the control of false discoveries; the rest is `Steady` when
/// there are enough postings to have seen a movement, `Unclear` when there are not.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// The data cannot tell: too few postings to have shown a movement, or the skill is in none.
    #[default]
    Unclear,
    Rising,
    Steady,
    Falling,
}

impl Direction {
    /// As the prompt, the markdown and the view spell it.
    pub fn as_str(self) -> &'static str {
        match self {
            Direction::Unclear => "unclear",
            Direction::Rising => "rising",
            Direction::Steady => "steady",
            Direction::Falling => "falling",
        }
    }
}

/// `part / whole` in per mille, rounded half up; 0 without a whole. The one rounding every surface
/// (prompt, markdown, view) uses, so a share reads the same everywhere.
pub fn permille(part: u64, whole: u64) -> u32 {
    if whole == 0 {
        return 0;
    }
    u32::try_from((part.saturating_mul(1000) + whole / 2) / whole).unwrap_or(u32::MAX)
}

/// A per mille as a percent with one decimal: `9.4%`.
pub fn permille_text(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

/// The level of a skill in the period's own months, in words: its share (`9.4%`) and the 95%
/// range of that share (`7.1% to 12.3%`), from the per mille numbers of a [`SkillCount`] or a
/// [`GapSkill`]. `None` in a report saved before the tests (`target_months` is 0), and when the
/// period's months held no document to take a share of (no range, not even an empty one). The one
/// place every surface (prompt, markdown, view) gets these words from.
pub fn level_text(target_months: u32, target_permille: u32, low_permille: u32, high_permille: u32) -> Option<(String, String)> {
    if target_months == 0 || high_permille == 0 {
        return None;
    }
    Some((permille_text(target_permille), format!("{} to {}", permille_text(low_permille), permille_text(high_permille))))
}

/// Where a skill stands for this user: the market's direction against their background (the
/// profile's skills), decided in code (`count::gap_bucket`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapBucket {
    /// In the profile, and rising or steadily asked for.
    BuildOn,
    /// Not in the profile, and rising or often asked for in the user's field.
    LearnNext,
    /// In the profile, and falling.
    AtRisk,
}

/// One skill's frequency over the window of months.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCount {
    pub skill: String,
    /// Documents mentioning the skill, aligned with [`SkillCounts::months`].
    pub by_month: Vec<u32>,
    pub total: u32,
    /// Where its share is going (older reports: `Unclear`).
    #[serde(default)]
    pub direction: Direction,
    /// Documents of the period's own months (the last [`SkillCounts::target_months`]) naming the
    /// skill; 0 in a report saved before the tests.
    #[serde(default)]
    pub target: u32,
    /// `target` as a share of the documents of those months, in per mille.
    #[serde(default)]
    pub target_permille: u32,
    /// The 95% interval of that share, in per mille (the Wilson interval): how far it can be
    /// trusted at this many postings.
    #[serde(default)]
    pub low_permille: u32,
    #[serde(default)]
    pub high_permille: u32,
    /// The period's months against the months before, as a z-score in hundredths (positive: the
    /// share is higher in the period). A direction needs the test to survive the control of false
    /// discoveries, not only a large score.
    #[serde(default)]
    pub z_centi: i32,
    /// The trend over every month of the window, as a z-score in hundredths (positive: growing).
    #[serde(default)]
    pub trend_z_centi: i32,
    /// The skill is in the user's profile.
    #[serde(default)]
    pub in_profile: bool,
    /// The skill is one the user wishes for (wanted, nice to have or avoided): measured like any
    /// other, never favoured.
    #[serde(default)]
    pub wished: bool,
}

/// The counts of one slice of the market's postings, aligned with [`SkillCounts::months`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScopeCounts {
    /// Documents read per month.
    pub documents: Vec<u32>,
    /// Most mentioned first, ties by name; only skills that appear at all.
    pub skills: Vec<SkillCount>,
}

/// Which slice of the market the gap was judged on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GapScope {
    #[default]
    Global,
    Local,
}

/// One skill of the gap: background against the market.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GapSkill {
    pub skill: String,
    pub bucket: GapBucket,
    pub direction: Direction,
    /// Share of the field's postings naming the skill over the whole window, in per mille.
    pub share_permille: u32,
    /// Its share of the field's postings in the period's own months, in per mille (0 in a report
    /// saved before the tests).
    #[serde(default)]
    pub target_permille: u32,
    /// The 95% interval of that share, in per mille.
    #[serde(default)]
    pub low_permille: u32,
    #[serde(default)]
    pub high_permille: u32,
}

/// The kind of item a document is. Items of different kinds are never counted together: a mix of
/// stories, companies, postings and feed entries changes from one period to the next, and a change
/// of mix alone would make hundreds of words look like movements. Each kind is a *stratum*, with
/// its own counts and its own tests.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stratum {
    /// A "Who is hiring" posting ([`is_hiring_posting`]).
    HiringPosting,
    /// Any other item of a Hacker News source: a story.
    HnStory,
    /// A company of the YC directory.
    Yc,
    /// An entry of a feed or a page (the `rss` and `page` sources).
    Feed,
    /// An item whose source is gone or unknown. Also what a term of a report saved before the
    /// strata is: its topics came from a mix of kinds, so no kind can be claimed for it.
    #[default]
    Other,
}

impl Stratum {
    /// In display order: postings first.
    pub const ALL: [Stratum; 5] = [Stratum::HiringPosting, Stratum::HnStory, Stratum::Yc, Stratum::Feed, Stratum::Other];

    /// The items of this kind, as the prompt and the markdown name them.
    pub fn plural(self) -> &'static str {
        match self {
            Stratum::HiringPosting => "hiring postings",
            Stratum::HnStory => "Hacker News stories",
            Stratum::Yc => "YC companies",
            Stratum::Feed => "feed and page items",
            Stratum::Other => "other items",
        }
    }
}

/// A term of the items' own words that the skill vocabulary does not know: a possible new skill
/// the fixed list would hide. Found in code in every item of a [`Stratum`] (never in a sample),
/// by [`terms`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermCount {
    pub term: String,
    /// Documents of the period's side (the period's own months, for postings) naming it.
    pub items: u32,
    /// The kind of item it was found in (a report saved before the strata: `Other`).
    #[serde(default)]
    pub stratum: Stratum,
    /// Its share of the documents on the period's side against the side before, as a z-score in
    /// hundredths (positive: more of them name it now); 0 when it was not tested.
    #[serde(default)]
    pub z_centi: i32,
    /// The test survived the control of false discoveries of its stratum, and it is a rise.
    #[serde(default)]
    pub rising: bool,
}

/// What the counts saw and did not see: how much was read, what could not be, and which kinds of
/// item were too thin or too uneven to be compared. Everything the report says about direction
/// holds only for this much data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Coverage {
    /// Hiring postings of the window that were counted.
    pub postings_counted: u32,
    /// Hacker News items of the window whose stored text could not be read (a file that is missing
    /// or unreadable, or no file at all): any of them may have been a posting.
    pub postings_unreadable: u32,
    /// Items of the period counted across every stratum (postings of the period, and the other
    /// items whose text could be read).
    pub items_counted: u32,
    /// Items the sources of this run exposed but left unread (the sum of what each cap cut); `None`
    /// when no collection ran, the digest having been built from stored items alone.
    pub left_unread_at_source: Option<u32>,
    /// Months of the window in which no posting was counted (a thread not collected, or none yet).
    pub months_without_postings: Vec<String>,
    /// Kinds of item with items in the period that were not tested against the side before: too few
    /// documents on a side, or a volume that changed too much. Their terms are listed by level only.
    pub not_comparable: Vec<Stratum>,
}

impl Coverage {
    /// What the counts saw and did not see, as plain sentences: the one every surface (the prompt,
    /// the markdown, the PDF and the view) says, so they never disagree. `months` are the months of
    /// the window, oldest first. The first sentence always comes: how many hiring postings were
    /// counted, over which months. Each other fact comes only when it applies: postings that could
    /// not be read, what the sources left unread in this run (stories and feed entries, not postings), months
    /// without postings, and the kinds of item that were not compared with the period before.
    pub fn lines(&self, months: &[String]) -> Vec<String> {
        let over = match (months.first(), months.last()) {
            (Some(first), Some(last)) if first != last => format!(" over {first} to {last}"),
            (Some(only), _) => format!(" in {only}"),
            _ => String::new(),
        };
        let mut lines = vec![match self.postings_counted {
            0 => format!("No hiring postings were counted{over}."),
            1 => format!("Counted 1 hiring posting{over}."),
            n => format!("Counted {n} hiring postings{over}."),
        }];
        match self.postings_unreadable {
            0 => {}
            1 => lines.push("1 Hacker News item could not be read; it may have been a posting.".to_string()),
            n => lines.push(format!("{n} Hacker News items could not be read; some may have been postings.")),
        }
        match self.left_unread_at_source {
            Some(1) => lines.push("In this run the sources left 1 item unread.".to_string()),
            Some(n) if n > 1 => lines.push(format!("In this run the sources left {n} items unread.")),
            _ => {}
        }
        // When no posting was counted at all, the first sentence has said it.
        if self.postings_counted > 0 && !self.months_without_postings.is_empty() {
            lines.push(format!("No hiring postings were counted in {}.", self.months_without_postings.join(", ")));
        }
        if !self.not_comparable.is_empty() {
            let kinds: Vec<&str> = self.not_comparable.iter().map(|kind| kind.plural()).collect();
            lines.push(format!("Not compared with the period before (too few items or too uneven): {}.", kinds.join(", ")));
        }
        lines
    }
}

/// What a wish is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WishKind {
    Skill,
    Domain,
}

/// One of the user's wishes, measured against the market (the lens applied last).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WishSignal {
    pub wish: String,
    pub kind: WishKind,
    /// The user wants to avoid it (an avoided skill or domain) rather than have it.
    #[serde(default)]
    pub avoided: bool,
    /// Its direction in the gap's scope.
    pub direction: Direction,
    /// Postings of the gap's scope naming it over the window; 0 means no data.
    pub mentions: u32,
}

/// Skill frequency in the market's hiring postings per month, counted in code. The top-level
/// `documents` and `skills` are the global scope (every hiring posting); leads, which discovery
/// found through the user's own filters, are never mixed in (they are `pipeline`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkillCounts {
    /// `2026-06` ... `2026-11`, oldest first: the baseline months, then the period's own.
    pub months: Vec<String>,
    /// How many of the last `months` are the period's own (the others are what they are compared
    /// with); 0 in a report saved before the tests.
    pub target_months: u32,
    /// Hiring postings read per month, aligned with `months` (older reports also counted leads
    /// here).
    pub documents: Vec<u32>,
    /// Most mentioned first, ties by name; only skills that appear at all.
    pub skills: Vec<SkillCount>,
    /// The postings of the user's place (a preferred location, or remote when the work mode
    /// allows it); `None` without a place.
    pub local: Option<ScopeCounts>,
    /// How the place reads: "Ho Chi Minh City, Singapore or remote".
    pub place: String,
    /// The role words of the user's field, from the profile ("backend", "engineer").
    pub field: Vec<String>,
    /// The user's own leads, for comparison only; `None` when there are none.
    pub pipeline: Option<ScopeCounts>,
    /// The scope the gap and the wishes were judged on: local when it has enough postings.
    pub gap_scope: GapScope,
    /// The gap, by bucket then share.
    pub gap: Vec<GapSkill>,
    /// Terms outside the vocabulary found in the words of every item of the period, per
    /// [`Stratum`] (postings first): the tested risers of a stratum first, then its most frequent.
    pub uncatalogued: Vec<TermCount>,
    /// The user's wishes against the market, in the order the preferences list them.
    pub wishes: Vec<WishSignal>,
    /// How these counts were made: 0 for reports saved before the market scan, else
    /// [`MARKET_SCAN_VERSION`]. The view tells old reports from new ones by it, not by guessing
    /// from which fields happen to be filled.
    pub version: u32,
    /// What was counted and what was not seen.
    #[serde(default)]
    pub coverage: Coverage,
}

/// [`SkillCounts::version`] of counts made from the user's background and place.
pub const MARKET_SCAN_VERSION: u32 = 1;

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
    /// Items a weekly digest reads (spread over the sources, newest first); a longer period reads
    /// more ([`for_period`](Self::for_period), which also grows `evidence_chars` and `extract_budget`).
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
        Self { max_items: 240, batch_items: 6, concurrency: 3, item_chars: 4_000, evidence_chars: 40_000, extract_budget: Duration::from_secs(1_200) }
    }
}

impl DigestLimits {
    /// These limits for a digest of `period`: a longer period reads more items, so the item cap,
    /// the extraction budget and the evidence grow together by [`period_factor`]. The limits as
    /// configured are those of a week. The evidence stops growing at [`MAX_EVIDENCE_CHARS`] (or
    /// the configured size when that is larger) so the prompt still fits one model call: past it,
    /// the claims per item shrink and the last items are counted but not cited.
    pub fn for_period(&self, period: &Period) -> Self {
        let factor = period_factor(period).max(1);
        Self {
            max_items: self.max_items.saturating_mul(factor),
            evidence_chars: self.evidence_chars.saturating_mul(factor).min(MAX_EVIDENCE_CHARS.max(self.evidence_chars)),
            extract_budget: self.extract_budget.saturating_mul(u32::try_from(factor).unwrap_or(u32::MAX)),
            ..self.clone()
        }
    }
}

/// The most evidence a longer period grows to: about 40k tokens.
const MAX_EVIDENCE_CHARS: usize = 160_000;

/// How many times the weekly limits a digest of `period` gets. Less than the calendar ratio: the
/// evidence must still fit one prompt, and a longer period has more repeats to leave out.
fn period_factor(period: &Period) -> usize {
    match period {
        Period::Week { .. } => 1,
        Period::Month { .. } => 2,
        Period::Quarter { .. } => 4,
        Period::Year { .. } => 8,
    }
}

/// What the digest nodes need besides the store: the model and the limits.
#[derive(Clone, Default)]
pub struct DigestConfig {
    /// The model. `None`: the default AI provider, looked up when a node needs it (tests pass a fake).
    pub llm: Option<Arc<dyn Llm>>,
    pub limits: DigestLimits,
    /// The profile the market frame reads the user's background from (`Core` fills it in with
    /// the stored profile). `None`: no background (no profile yet, or tests).
    pub profile: Option<Cv>,
}

impl fmt::Debug for DigestConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DigestConfig")
            .field("llm", &self.llm.as_ref().map(|_| ".."))
            .field("limits", &self.limits)
            .field("profile", &self.profile.as_ref().map(|_| ".."))
            .finish()
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
        let client = llm::client_for(&self.secrets, &provider, key);
        Ok(Arc::from(llm::persisting_json_mode(self.store.clone(), &provider, client)))
    }
}

// ---------------------------------------------------------------- graph

/// Add the digest nodes to `graph` (`count`, `extract` ... `save_report`, ending at END). The caller
/// connects something to `count`. The node names are the ones saved checkpoints know.
pub(crate) fn add_nodes(graph: GraphDef<TrendRun>, env: Arc<DigestEnv>) -> GraphDef<TrendRun> {
    // The graph is built before the period is known: the timeout covers the longest one.
    let timeout = env.config.limits.for_period(&Period::Year { year: 0 }).extract_budget + NODE_MARGIN;
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
        .edge("count", "extract")
        .edge("extract", "synthesize")
        .edge("synthesize", "enforce_citations")
        .edge("enforce_citations", "save_report")
        .edge("save_report", END)
        .node_timeout("extract", timeout)
}

/// The digest alone: `START -> count -> extract -> ... -> save_report`, on the items already stored.
pub fn digest_graph(store: Store, secrets: Secrets, config: DigestConfig) -> GraphDef<TrendRun> {
    let env = Arc::new(DigestEnv { store, secrets, config });
    add_nodes(GraphDef::new(DIGEST_GRAPH), env).edge(START, "count")
}

// ---------------------------------------------------------------- the items of a period

/// Item headers read at once when looking for hiring postings.
const HEADER_READS: usize = 16;
/// Bytes of an item's text read to recognise a hiring posting by its header line.
const HEADER_BYTES: u64 = 512;

/// How old a Hacker News item fetched in the period may be and still count as the period's:
/// about one monthly hiring thread.
const CURRENT_THREAD_DAYS: i64 = 31;

/// Every item the digest of `period` is about, newest first, each with its [`Stratum`]: published
/// or fetched inside the period, with no cap. `count` counts all of them; `extract` reads a pick
/// ([`pick`]). Recognising a posting reads the first bytes of the text of each Hacker News item of
/// the period (no other item's); an item of a source that is gone is `Other`.
pub(crate) async fn period_items_all(store: &Store, period: &Period) -> Result<Vec<(TrendItem, Stratum)>> {
    let (start, end) = (period.start(), period.end());
    let inside = |t: DateTime<Utc>| t >= start && t < end;
    let kinds = source_kinds(store)?;
    let found: Vec<TrendItem> = store
        .list_trend_items(None)?
        .into_iter()
        .filter(|i| match i.published_at {
            // A Hacker News item carries its real time. Fetched in the period, it counts when it is
            // at most a month old (this month's hiring thread); a posting backfilled from an older
            // thread belongs to its own month, not to the run that fetched it.
            Some(at) if i.url.starts_with(count::HN_ITEM_PREFIX) => {
                inside(at) || (inside(i.fetched_at) && at >= end - chrono::Duration::days(CURRENT_THREAD_DAYS))
            }
            at => at.is_some_and(inside) || inside(i.fetched_at),
        })
        .collect();
    let items: Vec<(TrendItem, Stratum)> = futures::stream::iter(found)
        .map(|item| {
            let kinds = &kinds;
            async move {
                let posting = is_posting_item(&item).await;
                let stratum = stratum_of(&item, posting, kinds);
                (item, stratum)
            }
        })
        .buffered(HEADER_READS)
        .collect()
        .await;
    tracing::debug!(
        items = items.len(),
        postings = items.iter().filter(|(_, s)| *s == Stratum::HiringPosting).count(),
        "items of the period listed"
    );
    Ok(items)
}

/// The adapter of each trend source (a source of a kind this version does not know is left out).
fn source_kinds(store: &Store) -> Result<HashMap<i64, SourceKind>> {
    Ok(store.list_sources(SourcePurpose::Trends)?.into_iter().filter_map(|s| Some((s.id, SourceKind::parse(&s.kind)?))).collect())
}

/// The kind of an item: a posting by its header, any other by the adapter of its source.
fn stratum_of(item: &TrendItem, posting: bool, kinds: &HashMap<i64, SourceKind>) -> Stratum {
    if posting {
        return Stratum::HiringPosting;
    }
    match item.source_id.and_then(|id| kinds.get(&id)) {
        Some(SourceKind::HackerNews) => Stratum::HnStory,
        Some(SourceKind::Yc) => Stratum::Yc,
        Some(SourceKind::Rss | SourceKind::Page) => Stratum::Feed,
        None => Stratum::Other,
    }
}

/// Is the item a "Who is hiring" posting: a Hacker News item whose text starts with the thread's
/// header ([`is_hiring_posting`])? Only the first [`HEADER_BYTES`] are read.
async fn is_posting_item(item: &TrendItem) -> bool {
    use tokio::io::AsyncReadExt;
    let Some(path) = item.text_path.as_ref().filter(|_| item.url.starts_with(count::HN_ITEM_PREFIX)) else { return false };
    let Ok(file) = tokio::fs::File::open(path).await else { return false };
    let mut head = Vec::new();
    if file.take(HEADER_BYTES).read_to_end(&mut head).await.is_err() {
        return false;
    }
    is_hiring_posting(&String::from_utf8_lossy(&head))
}

/// Take items round-robin over their sources (the input is newest first) until `max`, then order
/// the pick newest first. Items without a source (of a removed source) count as one source.
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

/// The saved text of an item; `None` when there is no text file or it cannot be read (which is
/// not the same as a text that is empty).
pub(crate) async fn try_read_text(item: &TrendItem) -> Option<String> {
    tokio::fs::read_to_string(item.text_path.as_ref()?).await.ok()
}

/// The saved text of an item; empty when there is none or it cannot be read.
pub(crate) async fn read_text(item: &TrendItem) -> String {
    try_read_text(item).await.unwrap_or_default()
}
