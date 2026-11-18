//! The `count` node: what the market asks for, per month, counted in code.
//!
//! The market is the "Who is hiring" postings among the collected items: nobody filtered them for
//! the user. A document counts once for a skill however often it names it, and a skill's weight in
//! a month is its *share* of the documents read that month, so a busy month does not look like a
//! trend. The vocabulary is fixed (`BUILT_IN`) plus the skills of the user's profile, so the same
//! postings and the same profile always give the same numbers; the model has no part in it.
//!
//! What is counted:
//!
//! - **global**: every posting of the window; **local**: the postings of the user's place (a
//!   preferred location, or remote when the work mode allows it), when there is a place.
//! - a [`Direction`] per skill and scope ([`judge`]): the period's own months against the months
//!   before them (a step) and the trend over every month, each a test of the counts, all of a
//!   scope's tests under one control of false discoveries; `Unclear` when the data cannot tell.
//!   The window knows which months are the period's ([`Window`]).
//! - **the gap** ([`gap_bucket`]): the skills of the postings in the user's field (the role words
//!   of the profile), sorted by direction against "in the profile". It is judged on the local
//!   scope when each side of its window (before the period, the period) has enough postings, else
//!   on the global one.
//! - **terms not in the list**: the words of every item of the period that the vocabulary does not
//!   know, per kind of item, ranked by a test where the volumes allow one ([`super::terms`]).
//! - **coverage**: how much was read and what was not seen ([`Coverage`]).
//! - **the pipeline**: the user's own leads, counted apart. Discovery found them through the
//!   user's filters, so they never enter the market numbers.
//! - **wishes**: each wished or avoided skill and domain, measured in the gap's scope. Wishes are
//!   the lens applied last: they are never part of the market vocabulary, so they cannot change a
//!   market number, a direction or the gap.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use futures::StreamExt;
use regex::Regex;

use super::stats::{self, Evidence};
use super::terms::{Corpora, Side};
use super::{
    period_items_all, try_read_text, Coverage, DigestEnv, Direction, GapBucket, GapScope, GapSkill, ScopeCounts, SkillCount,
    SkillCounts, Stratum, WishKind, WishSignal,
};
use crate::discover::source::mentions_lowered;
use crate::prefs::Preferences;
use crate::store::{Lead, LeadState, Store, TrendItem};
use crate::trends::collect::TrendRun;
use crate::trends::frame::MarketFrame;
use crate::trends::period::Period;

/// Months before a week's or a month's own month that the window compares it with.
const BASELINE_MONTHS: usize = 5;
/// Months before a quarter's own months that the window compares them with.
const QUARTER_BASELINE_MONTHS: usize = 3;
/// Skills kept in a scope's list.
const MAX_SKILLS: usize = 30;
/// Characters of a lead's posting that are read.
const MAX_LEAD_CHARS: usize = 20_000;
/// Item texts read at once.
pub(super) const TEXT_READS: usize = 16;
/// Permalink of a Hacker News item: where hiring postings live.
pub(super) const HN_ITEM_PREFIX: &str = "https://news.ycombinator.com/item?id=";

/// The prior of the step test, worth this many documents pulled towards the pooled share (see
/// [`stats::log_odds_z`]): 1 lets the counts speak for themselves.
const STEP_PRIOR: f64 = 1.0;
/// Share of the field's postings (in per mille) from which a skill is "often asked for": it can
/// be built on or learnt next without a rising direction.
pub(super) const HIGH_SHARE_PERMILLE: u32 = 50;
/// Skills per gap bucket.
const MAX_PER_BUCKET: usize = 8;

/// `(name, aliases)`. An alias is matched as whole words, ignoring case; one that starts with `=`
/// must match exactly as written (`=Go`: the language, not the verb).
const BUILT_IN: &[(&str, &[&str])] = &[
    ("Rust", &["rust"]),
    ("Go", &["=Go", "golang"]),
    ("Python", &["python"]),
    ("Java", &["java"]),
    ("Kotlin", &["kotlin"]),
    ("Scala", &["scala"]),
    ("TypeScript", &["typescript"]),
    ("JavaScript", &["javascript"]),
    ("C++", &["c++"]),
    ("C#", &["c#"]),
    ("Swift", &["=Swift"]),
    ("Ruby", &["ruby"]),
    ("PHP", &["php"]),
    ("Elixir", &["elixir"]),
    ("Erlang", &["erlang"]),
    ("Haskell", &["haskell"]),
    ("Clojure", &["clojure"]),
    ("OCaml", &["ocaml"]),
    ("Zig", &["=Zig"]),
    ("Dart", &["=Dart"]),
    ("SQL", &["sql"]),
    ("React", &["react", "react.js", "reactjs"]),
    ("React Native", &["react native"]),
    ("Vue", &["vue", "vue.js", "vuejs"]),
    ("Angular", &["angular"]),
    ("Svelte", &["svelte"]),
    ("Next.js", &["next.js", "nextjs"]),
    ("Node.js", &["node.js", "nodejs"]),
    ("Django", &["django"]),
    ("Flask", &["flask"]),
    ("FastAPI", &["fastapi"]),
    ("Spring", &["=Spring"]),
    ("Rails", &["rails"]),
    ("Laravel", &["laravel"]),
    (".NET", &[".net", "asp.net", "dotnet"]),
    ("GraphQL", &["graphql"]),
    ("gRPC", &["grpc"]),
    ("Flutter", &["flutter"]),
    ("Android", &["android"]),
    ("iOS", &["ios"]),
    ("PostgreSQL", &["postgres", "postgresql"]),
    ("MySQL", &["mysql"]),
    ("MongoDB", &["mongodb", "mongo"]),
    ("Redis", &["redis"]),
    ("Kafka", &["kafka"]),
    ("Spark", &["=Spark", "pyspark"]),
    ("Flink", &["flink"]),
    ("Snowflake", &["snowflake"]),
    ("BigQuery", &["bigquery"]),
    ("Databricks", &["databricks"]),
    ("dbt", &["dbt"]),
    ("Airflow", &["airflow"]),
    ("Elasticsearch", &["elasticsearch", "opensearch"]),
    ("ClickHouse", &["clickhouse"]),
    ("Cassandra", &["cassandra"]),
    ("DynamoDB", &["dynamodb"]),
    ("SQLite", &["sqlite"]),
    ("AWS", &["aws"]),
    ("GCP", &["gcp", "google cloud"]),
    ("Azure", &["azure"]),
    ("Kubernetes", &["kubernetes", "k8s"]),
    ("Docker", &["docker"]),
    ("Terraform", &["terraform"]),
    ("Ansible", &["ansible"]),
    ("Helm", &["=Helm"]),
    ("Linux", &["linux"]),
    ("Prometheus", &["prometheus"]),
    ("Grafana", &["grafana"]),
    ("GitHub Actions", &["github actions"]),
    ("Nix", &["=Nix", "nixos"]),
    ("Machine Learning", &["machine learning"]),
    ("PyTorch", &["pytorch"]),
    ("TensorFlow", &["tensorflow"]),
    ("LLM", &["llm", "llms"]),
    ("Generative AI", &["generative ai", "genai"]),
    ("RAG", &["=RAG"]),
    ("LangChain", &["langchain"]),
    ("Computer Vision", &["computer vision"]),
    ("NLP", &["=NLP", "natural language processing"]),
    ("WebAssembly", &["wasm", "webassembly"]),
    ("Solidity", &["solidity"]),
    ("Blockchain", &["blockchain"]),
    ("Microservices", &["microservice", "microservices"]),
    ("Distributed Systems", &["distributed systems"]),
    ("Observability", &["observability"]),
];

/// Words of a text: runs of letters and digits that may contain `+ # . -` (`c++`, `node.js`,
/// `front-end`), or `.NET`. Trailing dots and dashes are cut by [`tokens`].
fn word_pattern() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| Regex::new(r"(?i)\.net\b|[a-z0-9][a-z0-9+#.\-]*").expect("static regex"))
}

/// `(as written, lower case)` per word.
pub(super) fn tokens(text: &str) -> Vec<(&str, String)> {
    word_pattern()
        .find_iter(text)
        .map(|m| {
            let word = m.as_str().trim_end_matches(['.', '-']);
            (word, word.to_lowercase())
        })
        .filter(|(word, _)| !word.is_empty())
        .collect()
}

struct Alias {
    skill: usize,
    /// Lower-case words after the first.
    rest: Vec<String>,
    /// The first word exactly as written, for aliases that must match case.
    exact: Option<String>,
}

/// The skills that are counted and the words that stand for them.
pub struct Vocabulary {
    names: Vec<String>,
    by_first_word: HashMap<String, Vec<Alias>>,
}

impl Vocabulary {
    /// The built-in skills plus `extra` (the user's skills). An extra skill the vocabulary already
    /// knows adds nothing: "golang" or "Spring Boot" in a profile stand for the built-in Go and
    /// Spring, not for skills of their own. An extra skill of one or two letters or digits ("C",
    /// "R") is not counted: it would match "Series C" or "R&D" in every other posting, so its share
    /// would say nothing (`C#` and `C++` are fine).
    pub fn new(extra: &[String]) -> Self {
        let mut vocab = Vocabulary { names: Vec::new(), by_first_word: HashMap::new() };
        for (name, aliases) in BUILT_IN {
            vocab.add(name, aliases);
        }
        for skill in extra {
            let skill = skill.split_whitespace().collect::<Vec<_>>().join(" ");
            let ambiguous = skill.chars().count() <= 2 && skill.chars().all(char::is_alphanumeric);
            if !skill.is_empty() && !ambiguous && vocab.find(&skill).is_empty() {
                vocab.add(&skill, &[&skill.to_lowercase()]);
            }
        }
        vocab
    }

    fn add(&mut self, name: &str, aliases: &[&str]) {
        if self.names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            return;
        }
        let skill = self.names.len();
        for alias in aliases {
            let (alias, exact) = match alias.strip_prefix('=') {
                Some(plain) => (plain, true),
                None => (*alias, false),
            };
            let words = tokens(alias);
            let Some(((written, first), rest)) = words.split_first() else { continue };
            self.by_first_word.entry(first.clone()).or_default().push(Alias {
                skill,
                rest: rest.iter().map(|(_, lower)| lower.clone()).collect(),
                exact: exact.then(|| (*written).to_string()),
            });
        }
        self.names.push(name.to_string());
    }

    /// The skills a text names, by index into `names`. With `strict`, an alias that must match case
    /// does (postings: "we go to market" is not Go); without, case is ignored everywhere (a skill
    /// name or a topic label: "go" is Go).
    fn matches(&self, text: &str, strict: bool) -> BTreeSet<usize> {
        let words = tokens(text);
        let mut found = BTreeSet::new();
        for (i, (written, lower)) in words.iter().enumerate() {
            let Some(candidates) = self.by_first_word.get(lower) else { continue };
            for alias in candidates {
                let exact_ok = !strict || alias.exact.as_deref().is_none_or(|e| e == *written);
                let rest_ok = alias.rest.iter().enumerate().all(|(k, w)| words.get(i + 1 + k).is_some_and(|(_, l)| l == w));
                if exact_ok && rest_ok {
                    found.insert(alias.skill);
                }
            }
        }
        found
    }

    /// The skills a posting names.
    pub(super) fn skills_in(&self, text: &str) -> BTreeSet<usize> {
        self.matches(text, true)
    }

    /// The skills a short label stands for (a profile skill, a wish, a topic), the skill of that
    /// name first: "Golang" is Go, "Spring Boot" is Spring.
    pub(super) fn find(&self, label: &str) -> Vec<usize> {
        let label = label.trim();
        let mut found: Vec<usize> = self.names.iter().position(|n| n.eq_ignore_ascii_case(label)).into_iter().collect();
        for skill in self.matches(label, false) {
            if !found.contains(&skill) {
                found.push(skill);
            }
        }
        found
    }

    /// The name of the skill a label stands for, if any.
    pub fn canonical(&self, label: &str) -> Option<&str> {
        self.find(label).first().map(|&i| self.names[i].as_str())
    }

    /// Every skill some label of `labels` stands for.
    fn find_all(&self, labels: &[String]) -> HashSet<usize> {
        labels.iter().flat_map(|l| self.find(l)).collect()
    }
}

/// One counted document: the month it belongs to (`2026-11`) and its text.
pub(super) struct Doc {
    pub month: String,
    pub text: String,
    /// `text` in lower case, for the place, field and domain matches.
    pub lower: String,
}

impl Doc {
    pub fn new(month: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        Self { month: month.into(), lower: text.to_lowercase(), text }
    }
}

/// The months a digest counts, in two parts: the baseline, which the period's own months (the
/// target) are compared with, then the target itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Window {
    /// `2026-06` ... `2026-11`, oldest first: the baseline, then the target.
    pub months: Vec<String>,
    /// How many of the last `months` are the period's own; the months before them are the baseline.
    pub target: usize,
}

/// Months since year 0 (`year * 12 + month - 1`): months can be counted with plain arithmetic.
fn month_index(date: NaiveDate) -> i32 {
    date.year() * 12 + date.month0() as i32
}

/// The month of an index of [`month_index`], as `2026-11`.
fn month_label(index: i32) -> String {
    format!("{:04}-{:02}", index.div_euclid(12), index.rem_euclid(12) + 1)
}

/// `n` consecutive months ending with the month of index `last`, oldest first.
fn months_ending(last: i32, n: usize) -> Vec<String> {
    (0..n).rev().map(|back| month_label(last - back as i32)).collect()
}

/// The month index of the period's last day.
fn last_month(period: &Period) -> i32 {
    month_index((period.end() - Duration::days(1)).date_naive())
}

/// The window of a digest of `period` built on `today`. It ends with the period's last month, but
/// never after today's: the months that have not begun hold no postings, so a year or a quarter
/// in progress ends with its latest month, and the target holds the period's months that have
/// begun. (A period that has not begun at all keeps its first month: there is nothing to cut.)
///
/// - **week, month**: the target is the one month the period ends in, the baseline the 5 before;
/// - **quarter**: the target is the quarter's months that have begun (1 to 3), the baseline the 3
///   months before the quarter;
/// - **year**: the target is the year's months that have begun (1 to 12), the baseline as many
///   months before the year.
pub(crate) fn window_for(period: &Period, today: NaiveDate) -> Window {
    let first = month_index(period.start().date_naive());
    let end = last_month(period).min(month_index(today)).max(first);
    let begun = usize::try_from(end - first + 1).unwrap_or(1);
    let (target, baseline) = match period {
        Period::Week { .. } | Period::Month { .. } => (1, BASELINE_MONTHS),
        Period::Quarter { .. } => (begun, QUARTER_BASELINE_MONTHS),
        Period::Year { .. } => (begun, begun),
    };
    Window { months: months_ending(end, baseline + target), target }
}

/// `n` months ending with the month of the period's last day, oldest first (`2026-06` ...).
pub fn month_window(period: &Period, n: usize) -> Vec<String> {
    months_ending(last_month(period), n)
}

/// What the counts say about one row (a skill, a domain) over the window: its direction and the
/// numbers it rests on.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Reading {
    direction: Direction,
    /// Documents of the target months naming it.
    target: u32,
    /// `target` as a share of the target months' documents, in per mille.
    target_permille: u32,
    /// The 95% interval of that share, in per mille.
    low_permille: u32,
    high_permille: u32,
    /// [`stats::log_odds_z`]: the target months against the baseline.
    step_z: f64,
    /// [`stats::trend_z`]: the trend over every month of the window.
    trend_z: f64,
}

/// The sum of a row.
fn sum(values: &[u32]) -> u32 {
    values.iter().fold(0, |sum, &n| sum.saturating_add(n))
}

/// The sums of the two sides of a row: the baseline (every month before the last `target`), then
/// the target.
fn sides(values: &[u32], target: usize) -> (u32, u32) {
    let (baseline, target) = values.split_at(values.len().saturating_sub(target));
    (sum(baseline), sum(target))
}

/// The target's share of the documents minus the baseline's, in per mille; 0 when a side has no
/// documents.
fn change_permille(target: (u32, u32), baseline: (u32, u32)) -> i32 {
    let share = |(k, n): (u32, u32)| (n > 0).then(|| f64::from(k.min(n)) / f64::from(n));
    match (share(target), share(baseline)) {
        (Some(now), Some(before)) => ((now - before) * 1000.0).round() as i32,
        _ => 0,
    }
}

/// Where each row's share is going, and how sure the counts are. `rows` hold the documents naming
/// each thing per month, `documents` the documents read per month (both oldest first), `target` the
/// last months that are the period's own.
///
/// A row gets two tests, the target against the baseline ([`stats::log_odds_z`]) and the trend over
/// every month ([`stats::trend_z`]). The tests of all the rows that are named at all are one family
/// under [`stats::FALSE_DISCOVERY_RATE`]; [`stats::call_direction`] turns what survives into a direction.
/// A row named in no document is `Unclear` and takes no part in the family.
fn judge(rows: &[&[u32]], documents: &[u32], target: usize) -> Vec<Reading> {
    struct Row {
        baseline: u32,
        now: u32,
        step_z: f64,
        trend_z: f64,
    }
    let (documents_baseline, documents_now) = sides(documents, target);
    let tested: Vec<Row> = rows
        .iter()
        .map(|row| {
            let (baseline, now) = sides(row, target);
            Row {
                baseline,
                now,
                step_z: stats::log_odds_z(now, documents_now, baseline, documents_baseline, STEP_PRIOR),
                trend_z: stats::trend_z(row, documents),
            }
        })
        .collect();
    let named = |row: &Row| row.baseline > 0 || row.now > 0;
    let family: Vec<f64> = tested.iter().filter(|row| named(row)).flat_map(|row| [row.step_z, row.trend_z]).collect();
    let mut survives = stats::benjamini_hochberg(&family, stats::FALSE_DISCOVERY_RATE).into_iter();
    tested
        .iter()
        .map(|row| {
            let direction = if named(row) {
                let evidence = Evidence {
                    step_z: row.step_z,
                    trend_z: row.trend_z,
                    step_survives: survives.next().unwrap_or(false),
                    trend_survives: survives.next().unwrap_or(false),
                    change_permille: change_permille((row.now, documents_now), (row.baseline, documents_baseline)),
                    min_side_documents: documents_baseline.min(documents_now),
                };
                stats::call_direction(&evidence)
            } else {
                Direction::Unclear
            };
            let (low_permille, high_permille) = stats::wilson(row.now, documents_now);
            Reading {
                direction,
                target: row.now,
                target_permille: super::permille(u64::from(row.now), u64::from(documents_now)),
                low_permille,
                high_permille,
                step_z: row.step_z,
                trend_z: row.trend_z,
            }
        })
        .collect()
}

/// Every skill of a vocabulary over some documents: documents per month, per skill and month the
/// documents naming it, and what the counts say about each skill (all of them one family of tests).
struct Tally {
    documents: Vec<u32>,
    table: Vec<Vec<u32>>,
    /// The last months of the window that are the period's own.
    target: usize,
    /// Per skill, aligned with `table`.
    readings: Vec<Reading>,
}

impl Tally {
    /// Documents outside the window's months are ignored.
    fn new<'a>(docs: impl IntoIterator<Item = &'a Doc>, vocab: &Vocabulary, window: &Window) -> Self {
        let column: HashMap<&str, usize> = window.months.iter().enumerate().map(|(i, m)| (m.as_str(), i)).collect();
        let mut documents = vec![0u32; window.months.len()];
        let mut table: Vec<Vec<u32>> = vec![vec![0; window.months.len()]; vocab.names.len()];
        for doc in docs {
            let Some(&at) = column.get(doc.month.as_str()) else { continue };
            documents[at] += 1;
            for skill in vocab.skills_in(&doc.text) {
                table[skill][at] += 1;
            }
        }
        let rows: Vec<&[u32]> = table.iter().map(Vec::as_slice).collect();
        let readings = judge(&rows, &documents, window.target);
        Self { documents, table, target: window.target, readings }
    }

    fn total(&self, skill: usize) -> u32 {
        self.table.get(skill).map_or(0, |row| sum(row))
    }

    fn direction(&self, skill: usize) -> Direction {
        self.readings.get(skill).map_or(Direction::Unclear, |reading| reading.direction)
    }

    /// Does each side of the window (before the period, the period) hold at least
    /// [`stats::MIN_SIDE_DOCS`] documents: is the scope big enough to judge a movement on?
    fn has_both_sides(&self) -> bool {
        let (baseline, target) = sides(&self.documents, self.target);
        baseline >= stats::MIN_SIDE_DOCS && target >= stats::MIN_SIDE_DOCS
    }

    /// The scope's list: skills that appear at all, most mentioned first, ties by name, at most
    /// [`MAX_SKILLS`], each with its direction, numbers and marks.
    fn scope(&self, vocab: &Vocabulary, marks: &Marks) -> ScopeCounts {
        let mut skills: Vec<SkillCount> = (0..vocab.names.len())
            .filter(|&i| self.total(i) > 0)
            .filter_map(|i| {
                let reading = self.readings.get(i)?;
                Some(SkillCount {
                    skill: vocab.names[i].clone(),
                    by_month: self.table[i].clone(),
                    total: self.total(i),
                    direction: reading.direction,
                    target: reading.target,
                    target_permille: reading.target_permille,
                    low_permille: reading.low_permille,
                    high_permille: reading.high_permille,
                    z_centi: stats::centi(reading.step_z),
                    trend_z_centi: stats::centi(reading.trend_z),
                    in_profile: marks.profile.contains(&i),
                    wished: marks.wished.contains(&i),
                })
            })
            .collect();
        skills.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.skill.cmp(&b.skill)));
        skills.truncate(MAX_SKILLS);
        ScopeCounts { documents: self.documents.clone(), skills }
    }
}

/// Which skills of the vocabulary are the user's: in the profile, or wished for.
#[derive(Default)]
struct Marks {
    profile: HashSet<usize>,
    wished: HashSet<usize>,
}

/// Skill frequency over `docs` (those outside the window are ignored): per skill and month the
/// number of documents that name it. Most mentioned first, ties by name, at most [`MAX_SKILLS`].
#[cfg(test)]
pub(super) fn count_skills(docs: &[Doc], vocab: &Vocabulary, window: &Window) -> SkillCounts {
    let ScopeCounts { documents, skills } = Tally::new(docs, vocab, window).scope(vocab, &Marks::default());
    SkillCounts {
        months: window.months.clone(),
        target_months: u32::try_from(window.target).unwrap_or(u32::MAX),
        documents,
        skills,
        ..SkillCounts::default()
    }
}

/// Where a skill goes in the user's gap, from the market's direction, whether the profile has it,
/// and its share of the field's postings (per mille):
///
/// - in the profile and falling: **at risk**;
/// - in the profile and rising, or steady with a share of at least [`HIGH_SHARE_PERMILLE`]:
///   **build on**;
/// - not in the profile, not falling, and rising or at a high share: **learn next**;
/// - otherwise not in the gap.
///
/// An `Unclear` direction (the data cannot tell) is treated like steady: only the share rule
/// applies, so a skill that is often asked for still shows, but nothing is called rising or at
/// risk on thin data.
pub fn gap_bucket(direction: Direction, in_profile: bool, share_permille: u32) -> Option<GapBucket> {
    let high = share_permille >= HIGH_SHARE_PERMILLE;
    match (in_profile, direction) {
        (true, Direction::Falling) => Some(GapBucket::AtRisk),
        (true, Direction::Rising) => Some(GapBucket::BuildOn),
        (true, Direction::Steady | Direction::Unclear) => high.then_some(GapBucket::BuildOn),
        (false, Direction::Falling) => None,
        (false, Direction::Rising) => Some(GapBucket::LearnNext),
        (false, Direction::Steady | Direction::Unclear) => high.then_some(GapBucket::LearnNext),
    }
}

fn bucket_rank(bucket: GapBucket) -> u8 {
    match bucket {
        GapBucket::BuildOn => 0,
        GapBucket::LearnNext => 1,
        GapBucket::AtRisk => 2,
    }
}


/// The gap over the field's postings: every skill of the vocabulary that falls in a bucket, by
/// bucket (build on, learn next, at risk), then share, then name; at most [`MAX_PER_BUCKET`] each.
fn gap_list(field: &Tally, vocab: &Vocabulary, marks: &Marks) -> Vec<GapSkill> {
    let documents: u64 = field.documents.iter().map(|&n| u64::from(n)).sum();
    let mut gap: Vec<GapSkill> = (0..vocab.names.len())
        .filter_map(|i| {
            let reading = field.readings.get(i)?;
            let share_permille = super::permille(u64::from(field.total(i)), documents);
            let bucket = gap_bucket(reading.direction, marks.profile.contains(&i), share_permille)?;
            Some(GapSkill {
                skill: vocab.names[i].clone(),
                bucket,
                direction: reading.direction,
                share_permille,
                target_permille: reading.target_permille,
                low_permille: reading.low_permille,
                high_permille: reading.high_permille,
            })
        })
        .collect();
    gap.sort_by(|a, b| {
        bucket_rank(a.bucket)
            .cmp(&bucket_rank(b.bucket))
            .then_with(|| b.share_permille.cmp(&a.share_permille))
            .then_with(|| a.skill.cmp(&b.skill))
    });
    let mut per_bucket: HashMap<u8, usize> = HashMap::new();
    gap.retain(|g| {
        let n = per_bucket.entry(bucket_rank(g.bucket)).or_default();
        *n += 1;
        *n <= MAX_PER_BUCKET
    });
    gap
}

/// The wishes of the preferences in their order: wanted and nice-to-have skills, avoided skills,
/// wanted domains, avoided domains; the first of two that differ only in case wins.
fn wish_list(prefs: &Preferences) -> Vec<(String, WishKind, bool)> {
    let lists: [(&[String], WishKind, bool); 5] = [
        (&prefs.must_skills, WishKind::Skill, false),
        (&prefs.nice_skills, WishKind::Skill, false),
        (&prefs.avoid_skills, WishKind::Skill, true),
        (&prefs.domains, WishKind::Domain, false),
        (&prefs.avoid_domains, WishKind::Domain, true),
    ];
    let mut seen: HashSet<(WishKind, String)> = HashSet::new();
    let mut wishes = Vec::new();
    for (list, kind, avoided) in lists {
        for wish in list {
            let wish = wish.split_whitespace().collect::<Vec<_>>().join(" ");
            if !wish.is_empty() && seen.insert((kind, wish.to_lowercase())) {
                wishes.push((wish, kind, avoided));
            }
        }
    }
    wishes
}

/// Each wish measured on the gap's scope (`docs`): a skill through a vocabulary that knows the
/// wished skills too (only here: the market lists never use it), a domain as a word in the
/// posting. No mention at all is "no data" (0, `Unclear`). The skills are one family of tests, as
/// in the market lists; the domains of the call are another.
fn wish_signals(wishes: &[(String, WishKind, bool)], docs: &[&Doc], profile_skills: &[String], window: &Window) -> Vec<WishSignal> {
    let wished: Vec<String> = wishes.iter().filter(|(_, kind, _)| *kind == WishKind::Skill).map(|(w, _, _)| w.clone()).collect();
    let vocab = Vocabulary::new(&[profile_skills, &wished[..]].concat());
    let tally = Tally::new(docs.iter().copied(), &vocab, window);

    let column: HashMap<&str, usize> = window.months.iter().enumerate().map(|(i, m)| (m.as_str(), i)).collect();
    let domain_rows: Vec<Vec<u32>> = wishes
        .iter()
        .filter(|(_, kind, _)| *kind == WishKind::Domain)
        .map(|(wish, _, _)| {
            let mut by_month = vec![0u32; window.months.len()];
            for doc in docs {
                if let Some(&at) = column.get(doc.month.as_str()) {
                    if mentions_lowered(&doc.lower, wish) {
                        by_month[at] += 1;
                    }
                }
            }
            by_month
        })
        .collect();
    let rows: Vec<&[u32]> = domain_rows.iter().map(Vec::as_slice).collect();
    let mut domains = judge(&rows, &tally.documents, window.target).into_iter().zip(&domain_rows);

    wishes
        .iter()
        .map(|(wish, kind, avoided)| {
            let (direction, mentions) = match kind {
                WishKind::Skill => match vocab.find(wish).first() {
                    Some(&i) => (tally.direction(i), tally.total(i)),
                    None => (Direction::Unclear, 0),
                },
                WishKind::Domain => domains.next().map_or((Direction::Unclear, 0), |(reading, row)| (reading.direction, sum(row))),
            };
            let direction = if mentions == 0 { Direction::Unclear } else { direction };
            WishSignal { wish: wish.clone(), kind: *kind, avoided: *avoided, direction, mentions }
        })
        .collect()
}

/// Everything [`count`] computes about skills, from what it read: the market's `postings`, the
/// user's `leads` (both already limited to the window). The terms and the coverage come from
/// [`count`] itself, which has read the other items.
pub(super) fn market_counts(frame: &MarketFrame, prefs: &Preferences, postings: &[Doc], leads: &[Doc], window: &Window) -> SkillCounts {
    // The market vocabulary knows the background, never the wishes.
    let vocab = Vocabulary::new(&frame.skills);
    let wishes = wish_list(prefs);
    let wished: Vec<String> = wishes.iter().filter(|(_, kind, _)| *kind == WishKind::Skill).map(|(w, _, _)| w.clone()).collect();
    let marks = Marks { profile: vocab.find_all(&frame.skills), wished: vocab.find_all(&wished) };

    let all: Vec<&Doc> = postings.iter().collect();
    let global = Tally::new(all.iter().copied(), &vocab, window).scope(&vocab, &marks);
    let local_docs: Option<Vec<&Doc>> = frame.has_place().then(|| postings.iter().filter(|d| frame.is_local(&d.lower)).collect());
    let local_tally = local_docs.as_ref().map(|docs| Tally::new(docs.iter().copied(), &vocab, window));
    let local = local_tally.as_ref().map(|tally| tally.scope(&vocab, &marks));
    let (gap_scope, scope_docs) = match (&local_docs, &local_tally) {
        (Some(docs), Some(tally)) if tally.has_both_sides() => (GapScope::Local, docs.as_slice()),
        _ => (GapScope::Global, all.as_slice()),
    };
    let field = Tally::new(scope_docs.iter().copied().filter(|d| frame.in_field(&d.lower)), &vocab, window);
    let pipeline = (!leads.is_empty()).then(|| Tally::new(leads, &vocab, window).scope(&vocab, &marks));

    SkillCounts {
        version: super::MARKET_SCAN_VERSION,
        months: window.months.clone(),
        target_months: u32::try_from(window.target).unwrap_or(u32::MAX),
        documents: global.documents,
        skills: global.skills,
        local,
        place: frame.place_label(),
        field: frame.role_words.clone(),
        pipeline,
        gap_scope,
        gap: gap_list(&field, &vocab, &marks),
        wishes: wish_signals(&wishes, scope_docs, &frame.skills, window),
        ..SkillCounts::default()
    }
}

/// A "Who is hiring" posting as the Hacker News adapter writes it: its text starts with
/// `# Hacker News, Ask HN: Who is hiring? (...)`.
pub fn is_hiring_posting(text: &str) -> bool {
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_lowercase();
    first.starts_with("# hacker news,") && first.contains("who is hiring")
}

fn month_of(time: DateTime<Utc>) -> String {
    time.format("%Y-%m").to_string()
}

/// The market frame and the preferences it was made from (the saved ones, or none when they cannot
/// be read: logged).
pub(super) fn market_frame(env: &DigestEnv) -> (MarketFrame, Preferences) {
    let prefs = env.store.preferences().unwrap_or_else(|_| {
        tracing::warn!("the saved preferences could not be read; the digest has no place and no wishes");
        Preferences::default()
    });
    (MarketFrame::new(env.config.profile.as_ref(), &prefs), prefs)
}

pub(super) async fn count(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    let window = window_for(&s.period, Utc::now().date_naive());
    let in_window = |month: &str| window.months.iter().any(|m| m == month);

    // A backfilled Hacker News source holds thousands of postings: read a few at a time. A text that
    // cannot be read is not "not a posting": it is counted apart (`Coverage::postings_unreadable`).
    let candidates: Vec<(String, TrendItem)> = env
        .store
        .list_trend_items(None)?
        .into_iter()
        .map(|item| (month_of(item.published_at.unwrap_or(item.fetched_at)), item))
        .filter(|(month, item)| item.url.starts_with(HN_ITEM_PREFIX) && in_window(month))
        .collect();
    let mut texts = futures::stream::iter(candidates)
        .map(|(month, item)| async move { (month, try_read_text(&item).await) })
        .buffered(TEXT_READS);
    let mut postings: Vec<Doc> = Vec::new();
    let mut postings_unreadable = 0u32;
    while let Some((month, text)) = texts.next().await {
        match text {
            None => postings_unreadable += 1,
            Some(text) if is_hiring_posting(&text) => postings.push(Doc::new(month, text)),
            Some(_) => {}
        }
    }
    drop(texts);
    let mut leads = Vec::new();
    for state in [LeadState::New, LeadState::Promoted, LeadState::Dismissed] {
        for lead in env.store.list_leads(state)? {
            let month = month_of(lead.found_at);
            if in_window(&month) {
                leads.push(Doc::new(month, lead_text(&lead).await));
            }
        }
    }

    let (frame, prefs) = market_frame(&env);
    let mut counts = market_counts(&frame, &prefs, &postings, &leads, &window);

    // The words of every item, per kind of item: postings over the window's months, every other
    // kind over this period against the one before.
    let baseline_months = window.months.len().saturating_sub(window.target);
    let mut corpora = Corpora::new(u32::try_from(baseline_months).unwrap_or(u32::MAX), u32::try_from(window.target).unwrap_or(u32::MAX));
    for doc in &postings {
        let side = if window.months.iter().position(|m| *m == doc.month).is_some_and(|at| at >= baseline_months) { Side::Target } else { Side::Baseline };
        corpora.add(Stratum::HiringPosting, side, &doc.text);
    }
    let period_postings_and_others = add_period_items(&env.store, &s.period, &mut corpora).await?;
    let terms = corpora.report(&Vocabulary::new(&frame.skills));
    counts.uncatalogued = terms.terms;
    counts.coverage = Coverage {
        postings_counted: u32::try_from(postings.len()).unwrap_or(u32::MAX),
        postings_unreadable,
        items_counted: period_postings_and_others,
        left_unread_at_source: left_unread_at_source(&s),
        months_without_postings: counts.months.iter().zip(&counts.documents).filter(|(_, n)| **n == 0).map(|(month, _)| month.clone()).collect(),
        not_comparable: terms.not_comparable,
    };

    let sum = |docs: &[u32]| docs.iter().sum::<u32>();
    tracing::info!(
        documents = sum(&counts.documents),
        local_documents = counts.local.as_ref().map_or(0, |l| sum(&l.documents)),
        leads = leads.len(),
        skills = counts.skills.len(),
        gap_local = counts.gap_scope == GapScope::Local,
        gap = counts.gap.len(),
        terms = counts.uncatalogued.len(),
        rising_terms = counts.uncatalogued.iter().filter(|t| t.rising).count(),
        wishes = counts.wishes.len(),
        postings = counts.coverage.postings_counted,
        postings_unreadable = counts.coverage.postings_unreadable,
        items = counts.coverage.items_counted,
        left_unread = counts.coverage.left_unread_at_source.map_or(-1, i64::from),
        months_without_postings = counts.coverage.months_without_postings.len(),
        not_comparable = counts.coverage.not_comparable.len(),
        "skill frequency counted"
    );
    s.digest.counts = Some(counts);
    Ok(s)
}

/// What the sources of this run left unread (the sum of what their caps cut); `None` when no
/// collection ran (the digest alone, on stored items).
fn left_unread_at_source(s: &TrendRun) -> Option<u32> {
    (!s.outcomes.is_empty()).then(|| u32::try_from(s.outcomes.iter().map(|o| o.cut).sum::<usize>()).unwrap_or(u32::MAX))
}

/// Add every item of `period` that is not a posting to its stratum, and those of the period before
/// to the baseline side; the postings are counted from the window, not from here. An item's side
/// is decided by when it was published (when it was fetched, if it does not say): an entry of last
/// week that a feed added today brings is last week's, and one older than both periods is on no
/// side. Returns how many items of the period were read: its postings, and the other items whose
/// text could be read.
async fn add_period_items(store: &Store, period: &Period, corpora: &mut Corpora) -> Result<u32> {
    let target = period_items_all(store, period).await?;
    let in_target: HashSet<i64> = target.iter().map(|(item, _)| item.id).collect();
    let previous = period.previous();
    let before = period_items_all(store, &previous).await?;
    let side_of = |item: &TrendItem| {
        let at = item.published_at.unwrap_or(item.fetched_at);
        let within = |p: &Period| at >= p.start() && at < p.end();
        if within(period) {
            Some(Side::Target)
        } else if within(&previous) {
            Some(Side::Baseline)
        } else {
            None
        }
    };

    let mut counted = target.iter().filter(|(_, stratum)| *stratum == Stratum::HiringPosting).count();
    // `(of the period, side, stratum, item)`, each item once.
    let work: Vec<(bool, Option<Side>, Stratum, TrendItem)> = target
        .into_iter()
        .map(|(item, stratum)| (true, item, stratum))
        .chain(before.into_iter().filter(|(item, _)| !in_target.contains(&item.id)).map(|(item, stratum)| (false, item, stratum)))
        .filter(|(_, _, stratum)| *stratum != Stratum::HiringPosting)
        .map(|(of_period, item, stratum)| (of_period, side_of(&item), stratum, item))
        .collect();
    let mut texts = futures::stream::iter(work)
        .map(|(of_period, side, stratum, item)| async move { (of_period, side, stratum, try_read_text(&item).await) })
        .buffered(TEXT_READS);
    while let Some((of_period, side, stratum, text)) = texts.next().await {
        let Some(text) = text else { continue };
        if let Some(side) = side {
            corpora.add(stratum, side, &text);
        }
        if of_period {
            counted += 1;
        }
    }
    Ok(u32::try_from(counted).unwrap_or(u32::MAX))
}

/// A lead's role and posting text.
async fn lead_text(lead: &Lead) -> String {
    let body = match &lead.jd_path {
        Some(path) => tokio::fs::read_to_string(path).await.unwrap_or_default(),
        None => String::new(),
    };
    format!("{}\n{}", lead.role, crate::trends::text::cap(&body, MAX_LEAD_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::cv::{Cv, Experience, SkillGroup};
    use crate::prefs::Remote;

    fn doc(month: &str, text: &str) -> Doc {
        Doc::new(month, text)
    }

    /// Three months, the last one the period's own.
    fn window() -> Window {
        Window { months: ["2026-09", "2026-10", "2026-11"].map(String::from).to_vec(), target: 1 }
    }

    fn total(counts: &SkillCounts, skill: &str) -> Option<Vec<u32>> {
        counts.skills.iter().find(|s| s.skill == skill).map(|s| s.by_month.clone())
    }

    #[test]
    fn skills_are_counted_once_per_document_and_month() {
        let docs = [
            doc("2026-09", "Acme | Rust, Rust and more Rust | We use Kafka"),
            doc("2026-10", "Globex | Senior Rust engineer | Go, Kubernetes (K8s) and Postgres"),
            doc("2026-10", "Initech | PostgreSQL DBA | AWS, docker"),
            doc("2026-11", "Hooli | We go to market fast. Rust! Machine Learning, node.js and C++/C#."),
            doc("2026-12", "Outside the window: Rust Rust"),
        ];
        let counts = count_skills(&docs, &Vocabulary::new(&[]), &window());

        assert_eq!((counts.months.as_slice(), counts.target_months), (window().months.as_slice(), 1));
        assert_eq!(counts.documents, [1, 2, 1], "the December document is ignored");
        assert_eq!(total(&counts, "Rust"), Some(vec![1, 1, 1]), "once per document");
        assert_eq!(total(&counts, "Kafka"), Some(vec![1, 0, 0]));
        assert_eq!(total(&counts, "Go"), Some(vec![0, 1, 0]), "the language, not the verb in 'we go to market'");
        assert_eq!(total(&counts, "Kubernetes"), Some(vec![0, 1, 0]), "K8s counts as Kubernetes, once");
        assert_eq!(total(&counts, "PostgreSQL"), Some(vec![0, 2, 0]), "Postgres and PostgreSQL");
        assert_eq!(total(&counts, "Docker"), Some(vec![0, 1, 0]), "ignoring case");
        assert_eq!(total(&counts, "Machine Learning"), Some(vec![0, 0, 1]), "two words");
        assert_eq!(total(&counts, "Node.js"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "C++"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "C#"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "Java"), None, "absent skills are not listed");
        // Most mentioned first, ties by name.
        assert_eq!(counts.skills[0].skill, "Rust");
        assert_eq!(counts.skills[1].skill, "PostgreSQL");
        let tied: Vec<&str> = counts.skills.iter().filter(|s| s.total == 1).map(|s| s.skill.as_str()).collect();
        let mut sorted = tied.clone();
        sorted.sort_unstable();
        assert_eq!(tied, sorted);
    }

    #[test]
    fn counting_is_deterministic_and_does_not_confuse_similar_words() {
        let docs = [doc("2026-11", "JavaScript and TypeScript, not Java. Trust us. Mongo, MongoDB. Spark of joy; Apache Spark.")];
        let a = count_skills(&docs, &Vocabulary::new(&[]), &window());
        let b = count_skills(&docs, &Vocabulary::new(&[]), &window());
        assert_eq!(a, b);
        assert_eq!(total(&a, "Rust"), None, "'Trust' is not Rust");
        assert_eq!(total(&a, "Java"), Some(vec![0, 0, 1]), "JavaScript is not Java");
        assert_eq!(total(&a, "JavaScript"), Some(vec![0, 0, 1]));
        assert_eq!(total(&a, "MongoDB"), Some(vec![0, 0, 1]));
        assert_eq!(total(&a, "Spark"), Some(vec![0, 0, 1]));
    }

    #[test]
    fn the_users_own_skills_are_counted_too() {
        let docs = [doc("2026-11", "We need Gleam and Crystal developers, and rust and golang too")];
        let vocab = Vocabulary::new(&["Gleam".into(), "crystal".into(), "rust".into(), "Golang".into(), "  ".into()]);
        let counts = count_skills(&docs, &vocab, &window());
        assert_eq!(total(&counts, "Gleam"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "crystal"), Some(vec![0, 0, 1]), "named as the user wrote it");
        assert_eq!(counts.skills.iter().filter(|s| s.skill.eq_ignore_ascii_case("rust")).count(), 1, "no duplicate of a built-in");
        assert_eq!(total(&counts, "Go"), Some(vec![0, 0, 1]), "golang is the built-in Go");
        assert_eq!(total(&counts, "Golang"), None, "not a skill of its own");
    }

    #[test]
    fn a_skill_of_one_or_two_letters_is_not_counted() {
        let docs = [doc("2026-11", "Series C startup, strong R&D team, we use C# and C++")];
        let vocab = Vocabulary::new(&["C".into(), "R".into(), "C#".into(), "C++".into()]);
        let counts = count_skills(&docs, &vocab, &window());
        assert_eq!(total(&counts, "C"), None, "Series C is not the language");
        assert_eq!(total(&counts, "R"), None, "R&D is not the language");
        assert_eq!(total(&counts, "C#"), Some(vec![0, 0, 1]));
        assert_eq!(total(&counts, "C++"), Some(vec![0, 0, 1]));
    }

    #[test]
    fn a_label_stands_for_the_skill_it_names() {
        let vocab = Vocabulary::new(&[]);
        assert_eq!(vocab.canonical("go"), Some("Go"), "a label ignores case");
        assert_eq!(vocab.canonical("Golang"), Some("Go"));
        assert_eq!(vocab.canonical("spring boot"), Some("Spring"));
        assert_eq!(vocab.canonical(" kubernetes "), Some("Kubernetes"));
        assert_eq!(vocab.canonical("pottery"), None);
    }

    #[test]
    fn month_window_ends_with_the_month_of_the_periods_last_day() {
        let w46 = Period::parse("2026-W46").unwrap();
        assert_eq!(month_window(&w46, 3), ["2026-09", "2026-10", "2026-11"]);
        let w1 = Period::parse("2026-W01").unwrap();
        assert_eq!(month_window(&w1, 3), ["2025-11", "2025-12", "2026-01"], "W01 ends on 2026-01-04");
        let q4 = Period::parse("2026-Q4").unwrap();
        assert_eq!(month_window(&q4, 6), ["2026-07", "2026-08", "2026-09", "2026-10", "2026-11", "2026-12"]);
    }

    /// The window of the period `label` on the day `(year, month, day)`: its months and its target.
    fn window_on(label: &str, today: (i32, u32, u32)) -> (Vec<String>, usize) {
        let today = NaiveDate::from_ymd_opt(today.0, today.1, today.2).unwrap();
        let window = window_for(&Period::parse(label).unwrap(), today);
        (window.months, window.target)
    }

    /// Every month from `first` to `last`, both `YYYY-MM`.
    fn months_of(first: &str, last: &str) -> Vec<String> {
        let index = |m: &str| m[..4].parse::<i32>().unwrap() * 12 + m[5..].parse::<i32>().unwrap() - 1;
        (index(first)..=index(last)).map(month_label).collect()
    }

    #[test]
    fn a_week_or_a_month_is_one_month_against_the_five_before() {
        assert_eq!(window_on("2026-W46", (2026, 11, 19)), (months_of("2026-06", "2026-11"), 1));
        assert_eq!(window_on("2026-M11", (2026, 11, 19)), (months_of("2026-06", "2026-11"), 1));
        // A past month, and one that has not begun, keep their own month.
        assert_eq!(window_on("2026-M03", (2026, 11, 19)), (months_of("2025-10", "2026-03"), 1));
        assert_eq!(window_on("2026-M11", (2026, 3, 18)), (months_of("2026-06", "2026-11"), 1));
        // 2026-W49 is Nov 30 - Dec 6: it ends in the month that has begun.
        assert_eq!(window_on("2026-W49", (2026, 11, 30)), (months_of("2026-06", "2026-11"), 1), "December has not begun");
        assert_eq!(window_on("2026-W49", (2026, 12, 3)), (months_of("2026-07", "2026-12"), 1));
        assert_eq!(window_on("2026-W01", (2026, 1, 2)), (months_of("2025-08", "2026-01"), 1), "W01 ends on 2026-01-04");
    }

    #[test]
    fn a_quarter_is_its_months_that_have_begun_against_the_three_before() {
        // In progress: October and November have begun, December has not.
        assert_eq!(window_on("2026-Q4", (2026, 11, 19)), (months_of("2026-07", "2026-11"), 2));
        assert_eq!(window_on("2026-Q4", (2026, 10, 2)), (months_of("2026-07", "2026-10"), 1), "the quarter's first month");
        // Over: all three months.
        assert_eq!(window_on("2026-Q4", (2027, 1, 5)), (months_of("2026-07", "2026-12"), 3));
        assert_eq!(window_on("2026-Q2", (2026, 11, 19)), (months_of("2026-01", "2026-06"), 3), "a past quarter");
        assert_eq!(window_on("2026-Q1", (2026, 3, 31)), (months_of("2025-10", "2026-03"), 3), "the last day of the quarter");
    }

    #[test]
    fn a_year_is_its_months_that_have_begun_against_as_many_before() {
        // The current year ends with the month that has begun.
        assert_eq!(window_on("2026", (2026, 3, 18)), (months_of("2025-10", "2026-03"), 3));
        assert_eq!(window_on("2026", (2026, 11, 19)), (months_of("2025-02", "2026-11"), 11));
        // A past year: twelve months against the twelve before.
        assert_eq!(window_on("2025", (2026, 11, 19)), (months_of("2024-01", "2025-12"), 12));
        // January, the year's first month: one against one.
        assert_eq!(window_on("2026", (2026, 1, 9)), (months_of("2025-12", "2026-01"), 1));
    }

    #[test]
    fn a_period_that_has_not_begun_keeps_its_first_month() {
        // Nothing can be cut: the window is the period's own, and it will hold no postings.
        assert_eq!(window_on("2027-Q2", (2026, 11, 19)), (months_of("2027-01", "2027-04"), 1));
        assert_eq!(window_on("2027", (2026, 11, 19)), (months_of("2026-12", "2027-01"), 1));
    }

    #[test]
    fn hiring_postings_are_recognised_by_their_header() {
        assert!(is_hiring_posting("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme | Rust"));
        assert!(!is_hiring_posting("# Rust 2.0 released\n\n450 points"));
        assert!(!is_hiring_posting("# Hacker News, Ask HN: Who wants to be hired?\n\n..."));
        assert!(!is_hiring_posting(""));
    }

    // ------------------------------------------------------------ direction and gap

    /// The direction of one skill alone: `by_month` of `documents`, the last `target` months the
    /// period's own. It is its own family of two tests.
    fn direction_of(by_month: &[u32], documents: &[u32], target: usize) -> Direction {
        judge(&[by_month], documents, target)[0].direction
    }

    #[test]
    fn thin_data_is_unclear_not_steady() {
        use Direction::*;
        // Nothing at all, and a skill that is in no posting.
        assert_eq!(direction_of(&[], &[], 0), Unclear);
        assert_eq!(direction_of(&[0; 6], &[100; 6], 3), Unclear, "named nowhere");
        // The same share all along: with 27 documents on each side a movement could have hidden.
        assert_eq!(direction_of(&[3; 6], &[9; 6], 3), Unclear, "27 documents a side");
        assert_eq!(direction_of(&[3; 6], &[10, 10, 10, 10, 10, 9], 3), Unclear, "29 in the target");
        assert_eq!(direction_of(&[3; 6], &[10; 6], 3), Steady, "exactly MIN_SIDE_DOCS a side is enough");
        // A small movement on few documents is not a finding either way.
        assert_eq!(direction_of(&[2, 2, 2, 3, 3, 3], &[9; 6], 3), Unclear);
        // A large one is: the tests already count how few the documents are.
        assert_eq!(direction_of(&[0, 0, 0, 6, 6, 6], &[9; 6], 3), Rising);
        // A few mentions in a big sample are a share, not a trend: the skill is steady.
        assert_eq!(direction_of(&[1, 0, 1, 0, 1, 0], &[100; 6], 3), Steady);
    }

    #[test]
    fn direction_compares_shares_not_counts() {
        use Direction::*;
        // The same share on plenty of documents, or noise around it: steady.
        assert_eq!(direction_of(&[50; 6], &[250; 6], 3), Steady);
        assert_eq!(direction_of(&[48, 53, 50, 47, 52, 50], &[250; 6], 3), Steady);
        // Twice the documents and twice the mentions at the same share: no change, however many more.
        assert_eq!(direction_of(&[20, 20, 20, 40, 40, 40], &[100, 100, 100, 200, 200, 200], 3), Steady);
        // The reading says so: no movement of the share at all.
        let reading = judge(&[&[20, 20, 20, 40, 40, 40]], &[100, 100, 100, 200, 200, 200], 3)[0];
        assert!(reading.step_z.abs() < 1e-9 && reading.trend_z.abs() < 1e-9, "{reading:?}");
        // More mentions but a smaller share is falling.
        assert_eq!(direction_of(&[60, 60, 60, 70, 70, 70], &[300, 300, 300, 600, 600, 600], 3), Falling);
        // A large change on a few hundred documents: 10% to 20% of 300 a side.
        assert_eq!(direction_of(&[10, 10, 10, 20, 20, 20], &[100; 6], 3), Rising);
        assert_eq!(direction_of(&[20, 20, 20, 10, 10, 10], &[100; 6], 3), Falling);
    }

    #[test]
    fn a_slow_drift_is_called_through_the_trend_even_when_the_last_step_is_small() {
        // 40% down to 32% over six months, 250 documents each month: the last month against the five
        // before is no evidence by itself, the trend over all six is.
        let (k, n) = ([100, 96, 92, 88, 84, 80], [250; 6]);
        let reading = judge(&[&k], &n, 1)[0];
        assert!(reading.step_z.abs() < 1.645, "the step alone would not be called: {reading:?}");
        assert!(reading.trend_z < -1.96, "{reading:?}");
        assert_eq!(reading.direction, Direction::Falling);
        // The same months in another order are no drift.
        assert_eq!(direction_of(&[92, 80, 100, 84, 96, 88], &n, 1), Direction::Steady);
        // The other way.
        let rising: Vec<u32> = k.iter().rev().copied().collect();
        assert_eq!(direction_of(&rising, &n, 1), Direction::Rising);
    }

    #[test]
    fn one_skill_moving_among_many_unchanged_ones_survives_the_family_control() {
        // Forty skills at a steady 20% of 100 documents a month, and one at 10% then 27%.
        let flat = [20u32; 6];
        let mover = [10, 10, 10, 27, 27, 27];
        let mut rows: Vec<&[u32]> = vec![&flat; 40];
        rows.push(&mover);
        let read = judge(&rows, &[100; 6], 3);
        assert_eq!(read[40].direction, Direction::Rising, "{:?}", read[40]);
        assert!(read[..40].iter().all(|r| r.direction == Direction::Steady));

        // A movement that alone passes at the usual 5% is not called among that many tests.
        let borderline = [20, 20, 20, 27, 27, 27];
        let alone = judge(&[&borderline], &[100; 6], 3)[0];
        assert!(alone.step_z > 1.96, "{alone:?}");
        assert_eq!(alone.direction, Direction::Rising);
        rows[40] = &borderline;
        assert_eq!(judge(&rows, &[100; 6], 3)[40].direction, Direction::Steady, "chance among 82 tests");
    }

    #[test]
    fn a_skill_in_no_document_is_unclear_and_takes_no_part_in_the_family() {
        // The same borderline movement, with a thousand unnamed skills around it: they add no tests.
        let (silent, borderline) = ([0u32; 6], [20, 20, 20, 27, 27, 27]);
        let mut rows: Vec<&[u32]> = vec![&silent; 1000];
        rows.push(&borderline);
        let read = judge(&rows, &[100; 6], 3);
        assert!(read[..1000].iter().all(|r| r.direction == Direction::Unclear && r.target == 0));
        assert_eq!(read[1000], judge(&[&borderline], &[100; 6], 3)[0]);
    }

    #[test]
    fn the_interval_holds_the_share_and_narrows_with_more_documents() {
        // 6 in 10 of the documents of each of the last three months, then ten times as many.
        let small = judge(&[&[1, 1, 1, 6, 6, 6]], &[10; 6], 3)[0];
        let big = judge(&[&[10, 10, 10, 60, 60, 60]], &[100; 6], 3)[0];
        assert_eq!((small.target, small.target_permille), (18, 600));
        assert_eq!((big.target, big.target_permille), (180, 600));
        for reading in [small, big] {
            assert!(reading.low_permille <= reading.target_permille && reading.target_permille <= reading.high_permille, "{reading:?}");
        }
        assert!(big.high_permille - big.low_permille < small.high_permille - small.low_permille);
        assert_eq!((small.low_permille, small.high_permille), stats::wilson(18, 30), "the target months only");
    }

    #[test]
    fn the_gap_bucket_follows_direction_profile_and_share() {
        use Direction::*;
        use GapBucket::*;
        let high = HIGH_SHARE_PERMILLE;
        let low = HIGH_SHARE_PERMILLE - 1;
        // In the profile.
        assert_eq!(gap_bucket(Falling, true, high), Some(AtRisk), "a falling skill you have is at risk");
        assert_eq!(gap_bucket(Rising, true, 1), Some(BuildOn));
        assert_eq!(gap_bucket(Steady, true, high), Some(BuildOn));
        assert_eq!(gap_bucket(Steady, true, low), None);
        assert_eq!(gap_bucket(Unclear, true, high), Some(BuildOn), "unclear is judged on the share alone");
        assert_eq!(gap_bucket(Unclear, true, low), None);
        // Not in the profile.
        assert_eq!(gap_bucket(Rising, false, 1), Some(LearnNext), "a rising skill you lack is to learn next");
        assert_eq!(gap_bucket(Steady, false, high), Some(LearnNext));
        assert_eq!(gap_bucket(Steady, false, low), None);
        assert_eq!(gap_bucket(Unclear, false, high), Some(LearnNext));
        assert_eq!(gap_bucket(Unclear, false, low), None);
        assert_eq!(gap_bucket(Falling, false, 900), None, "never learn a falling skill");
    }

    // ------------------------------------------------------------ the market counts

    fn cv() -> Cv {
        let mut cv = Cv::default();
        cv.basics.headline = "Backend Engineer".into();
        cv.experience = vec![Experience { position: "Backend Engineer".into(), company: "Acme".into(), ..Experience::default() }];
        cv.skills = vec![SkillGroup { category: "Languages".into(), items: vec!["Python".into(), "Golang".into(), "Rust".into()], hidden: false }];
        cv
    }

    fn place() -> Preferences {
        Preferences { locations: vec!["Singapore".into()], remote: Remote::Onsite, ..Preferences::default() }
    }

    fn wishes() -> Preferences {
        Preferences {
            must_skills: vec!["Kotlin".into(), "Gleam".into()],
            nice_skills: vec!["kotlin".into(), "Zig".into()],
            avoid_skills: vec!["PHP".into()],
            domains: vec!["fintech".into()],
            avoid_domains: vec!["gambling".into(), "Fintech".into()],
            roles: vec!["Product Manager".into()],
            ..place()
        }
    }

    /// Per month of six (the last three are the period's own): `local` backend postings in
    /// Singapore, and 4 elsewhere. In the first three months Python is in 8 of 12 local ones and Rust
    /// in 2; in the last three the other way round. Kafka is in half of them throughout, Gleam (a
    /// wish only) in 3, Kotlin in 1, fintech in 2.
    fn market(local: usize) -> (Window, Vec<Doc>) {
        let months: Vec<String> = ["2026-06", "2026-07", "2026-08", "2026-09", "2026-10", "2026-11"].map(String::from).to_vec();
        let mut docs = Vec::new();
        for (m, month) in months.iter().enumerate() {
            let late = m >= 3;
            for n in 0..local {
                let mut skills = Vec::new();
                if (!late && n < 8) || (late && n < 2) {
                    skills.push("Python");
                }
                if (late && n < 8) || (!late && n < 2) {
                    skills.push("Rust");
                }
                if n % 2 == 0 {
                    skills.push("Kafka");
                }
                if n < 3 {
                    skills.push("Gleam");
                }
                if n == 5 {
                    skills.push("Kotlin");
                }
                if n < 2 {
                    skills.push("fintech payments");
                }
                docs.push(doc(month, &format!("Acme{n} | Backend Engineer | Singapore | {}", skills.join(", "))));
            }
            for n in 0..4 {
                docs.push(doc(month, &format!("Globex{n} | Backend Engineer | Berlin | Go, Kubernetes")));
            }
        }
        (Window { months, target: 3 }, docs)
    }

    fn counts_of(prefs: &Preferences, postings: &[Doc], leads: &[Doc], window: &Window) -> SkillCounts {
        market_counts(&MarketFrame::new(Some(&cv()), prefs), prefs, postings, leads, window)
    }

    fn skill<'a>(skills: &'a [SkillCount], name: &str) -> &'a SkillCount {
        skills.iter().find(|s| s.skill == name).unwrap_or_else(|| panic!("no {name} in {skills:?}"))
    }

    #[test]
    fn the_market_is_counted_globally_and_locally_with_directions() {
        let (window, postings) = market(12);
        let counts = counts_of(&wishes(), &postings, &[], &window);

        assert_eq!(counts.documents, [16; 6]);
        let local = counts.local.as_ref().expect("a place is set");
        assert_eq!(local.documents, [12; 6]);
        assert_eq!(counts.place, "Singapore");
        assert_eq!(counts.field, ["backend", "engineer"]);
        assert_eq!(counts.target_months, 3);
        assert_eq!(counts.gap_scope, GapScope::Local, "36 local postings in the baseline and in the target");

        assert_eq!(skill(&local.skills, "Rust").direction, Direction::Rising);
        assert_eq!(skill(&local.skills, "Python").direction, Direction::Falling);
        assert_eq!(skill(&local.skills, "Kafka").direction, Direction::Steady);
        // The numbers behind the calls: the target is the last three months, the scores carry the sign.
        let rust = skill(&local.skills, "Rust");
        assert_eq!((rust.target, rust.target_permille), (24, 667), "8 of 12 in each of the last three months");
        assert!(rust.z_centi > 196 && rust.trend_z_centi > 196, "{rust:?}");
        let python = skill(&local.skills, "Python");
        assert_eq!((python.target, python.target_permille), (6, 167));
        assert!(python.z_centi < -196 && python.trend_z_centi < -196, "{python:?}");
        let kafka = skill(&local.skills, "Kafka");
        assert_eq!((kafka.target, kafka.target_permille, kafka.z_centi, kafka.trend_z_centi), (18, 500, 0, 0));
        // Every share lies inside its own interval, in every scope.
        for s in counts.skills.iter().chain(&local.skills) {
            assert!(s.low_permille <= s.target_permille && s.target_permille <= s.high_permille, "{s:?}");
        }
        assert_eq!(skill(&counts.skills, "Go").direction, Direction::Steady, "global only");
        assert!(local.skills.iter().all(|s| s.skill != "Go"), "Berlin is not local");
        // Marks: Golang in the profile marks Go; Kotlin is wished (and counted, it is built in).
        assert!(skill(&counts.skills, "Go").in_profile && skill(&counts.skills, "Rust").in_profile);
        assert!(!skill(&counts.skills, "Kafka").in_profile);
        assert!(skill(&counts.skills, "Kotlin").wished && !skill(&counts.skills, "Kafka").wished);
        // A wished skill that is not built in is never part of the market lists.
        assert!(counts.skills.iter().chain(&local.skills).all(|s| s.skill != "Gleam"));

        // The gap, on the local scope's backend postings.
        let gap: Vec<(&str, GapBucket, Direction)> = counts.gap.iter().map(|g| (g.skill.as_str(), g.bucket, g.direction)).collect();
        assert_eq!(
            gap,
            [
                ("Rust", GapBucket::BuildOn, Direction::Rising),
                ("Kafka", GapBucket::LearnNext, Direction::Steady),
                ("Kotlin", GapBucket::LearnNext, Direction::Steady),
                ("Python", GapBucket::AtRisk, Direction::Falling),
            ],
            "Kotlin is in 1 of 12 postings: often enough to learn, whether wished or not"
        );
        assert_eq!(skill(&local.skills, "Kafka").total, 36);
        assert_eq!(counts.gap[1].share_permille, 500);
        for g in &counts.gap {
            assert!(g.low_permille <= g.target_permille && g.target_permille <= g.high_permille, "{g:?}");
        }
        assert_eq!(counts.gap[0].target_permille, 667, "Rust in 24 of the 36 local postings of the target");
        assert!(counts.pipeline.is_none());
    }

    #[test]
    fn a_thin_local_market_leaves_the_gap_global() {
        let (window, postings) = market(9);
        let counts = counts_of(&place(), &postings, &[], &window);
        assert_eq!(counts.local.as_ref().map(|l| l.documents.clone()), Some(vec![9; 6]));
        assert_eq!(counts.gap_scope, GapScope::Global, "27 local postings on each side");
        // On 27 postings a side a steady share could hide a movement, so it is unclear, not steady;
        // a movement as large as Python's and Rust's is called all the same.
        let local = counts.local.as_ref().unwrap();
        assert_eq!(skill(&local.skills, "Kafka").direction, Direction::Unclear);
        assert_eq!(skill(&local.skills, "Python").direction, Direction::Falling);
        assert_eq!(skill(&local.skills, "Rust").direction, Direction::Rising);
        assert!(counts.gap.iter().any(|g| g.skill == "Go"), "Berlin's postings count in the global gap");

        // No place at all: no local scope.
        let nowhere = Preferences { remote: Remote::Onsite, ..Preferences::default() };
        let counts = counts_of(&nowhere, &postings, &[], &window);
        assert!(counts.local.is_none());
        assert_eq!((counts.gap_scope, counts.place.as_str()), (GapScope::Global, ""));
    }

    #[test]
    fn leads_never_change_the_market_counts() {
        let (window, postings) = market(12);
        let leads: Vec<Doc> = (0..40).map(|n| doc(&window.months[n % 6], "Initech | Backend Engineer | Singapore | Elixir, Elixir, Kafka")).collect();
        let without = counts_of(&wishes(), &postings, &[], &window);
        let with = counts_of(&wishes(), &postings, &leads, &window);

        assert_eq!((&with.documents, &with.skills, &with.local), (&without.documents, &without.skills, &without.local));
        assert_eq!((&with.gap_scope, &with.gap, &with.wishes), (&without.gap_scope, &without.gap, &without.wishes));
        let pipeline = with.pipeline.expect("the leads are counted apart");
        assert_eq!(pipeline.documents.iter().sum::<u32>(), 40);
        assert_eq!(skill(&pipeline.skills, "Elixir").total, 40);
    }

    #[test]
    fn wishes_are_measured_in_order_and_never_shape_the_market() {
        let (window, postings) = market(12);
        let counts = counts_of(&wishes(), &postings, &[], &window);
        let wishes: Vec<(&str, WishKind, bool, Direction, u32)> =
            counts.wishes.iter().map(|w| (w.wish.as_str(), w.kind, w.avoided, w.direction, w.mentions)).collect();
        assert_eq!(
            wishes,
            [
                ("Kotlin", WishKind::Skill, false, Direction::Steady, 6),
                ("Gleam", WishKind::Skill, false, Direction::Steady, 18),
                ("Zig", WishKind::Skill, false, Direction::Unclear, 0),
                ("PHP", WishKind::Skill, true, Direction::Unclear, 0),
                ("fintech", WishKind::Domain, false, Direction::Steady, 12),
                ("gambling", WishKind::Domain, true, Direction::Unclear, 0),
            ],
            "in the preferences' order, the second Kotlin and Fintech dropped"
        );

        // The same market with other wishes: every market number and the gap stay.
        let other = Preferences { must_skills: vec!["Elixir".into()], domains: vec!["robotics".into()], roles: vec!["Data Scientist".into()], ..place() };
        let plain = counts_of(&other, &postings, &[], &window);
        let unmark = |skills: &[SkillCount]| skills.iter().map(|s| SkillCount { wished: false, ..s.clone() }).collect::<Vec<_>>();
        assert_eq!(unmark(&plain.skills), unmark(&counts.skills));
        assert_eq!(plain.local.as_ref().map(|l| unmark(&l.skills)), counts.local.as_ref().map(|l| unmark(&l.skills)));
        assert_eq!((&plain.documents, plain.gap_scope, &plain.gap), (&counts.documents, counts.gap_scope, &counts.gap));
        assert_ne!(plain.wishes, counts.wishes);
    }
}
