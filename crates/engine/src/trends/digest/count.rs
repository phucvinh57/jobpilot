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
//! - a [`Direction`] per skill and scope ([`direction`]): the second half of the window against
//!   the first, `Unclear` when the data is thin.
//! - **the gap** ([`gap_bucket`]): the skills of the postings in the user's field (the role words
//!   of the profile), sorted by direction against "in the profile". It is judged on the local
//!   scope when that has enough postings, else on the global one.
//! - **terms not in the list**: frequent extracted topics the vocabulary does not know.
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

use super::{
    read_text, DigestEnv, Direction, Extraction, GapBucket, GapScope, GapSkill, ScopeCounts, SkillCount, SkillCounts,
    TermCount, WishKind, WishSignal,
};
use crate::discover::source::mentions_lowered;
use crate::prefs::Preferences;
use crate::store::{Lead, LeadState, TrendItem};
use crate::trends::collect::TrendRun;
use crate::trends::frame::MarketFrame;
use crate::trends::period::Period;

/// Months in the window, the period's month last.
const MONTHS: usize = 6;
/// Skills kept in a scope's list.
const MAX_SKILLS: usize = 30;
/// Characters of a lead's posting that are read.
const MAX_LEAD_CHARS: usize = 20_000;
/// Item texts read at once.
const TEXT_READS: usize = 16;
/// Permalink of a Hacker News item: where hiring postings live.
pub(super) const HN_ITEM_PREFIX: &str = "https://news.ycombinator.com/item?id=";

/// A half of the window (the first `n/2` months, or the rest) needs at least this many documents
/// before a direction is called: below it, one posting more or less swings the share by several
/// points. The same bar decides whether the local scope is big enough to judge the gap on.
pub(super) const MIN_HALF_DOCS: u32 = 30;
/// A skill needs at least this many mentions over the window before a direction is called.
pub(super) const MIN_MENTIONS: u32 = 5;
/// A share must move by at least this much (in per mille of the documents: 10 = one percentage
/// point) to be rising or falling ...
const MIN_SHARE_CHANGE_PERMILLE: i128 = 10;
/// ... and by at least this part of the first half's share (in percent): a skill at 40% that
/// moves to 41% is steady, one at 2% that moves to 3% is rising.
const MIN_RELATIVE_CHANGE_PERCENT: i128 = 25;
/// Share of the field's postings (in per mille) from which a skill is "often asked for": it can
/// be built on or learnt next without a rising direction.
pub(super) const HIGH_SHARE_PERMILLE: u32 = 50;
/// Skills per gap bucket.
const MAX_PER_BUCKET: usize = 8;
/// An extracted topic must name at least this many items to be listed as a term not in the list.
const MIN_TERM_ITEMS: u32 = 3;
/// Terms not in the list that are kept.
const MAX_TERMS: usize = 10;
/// Topics this short ("ai", "ml", "go") are too generic to be listed as terms.
const MAX_GENERIC_CHARS: usize = 2;

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
fn tokens(text: &str) -> Vec<(&str, String)> {
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
    fn skills_in(&self, text: &str) -> BTreeSet<usize> {
        self.matches(text, true)
    }

    /// The skills a short label stands for (a profile skill, a wish, a topic), the skill of that
    /// name first: "Golang" is Go, "Spring Boot" is Spring.
    fn find(&self, label: &str) -> Vec<usize> {
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

/// The months of skill counts for a digest of `period` built on `today`: [`MONTHS`] ending with the
/// period's last month. The current year ends with its month that has begun: the months to come
/// hold no postings, and a window with an empty half has no direction.
pub(crate) fn window_for(period: &Period, today: NaiveDate) -> Vec<String> {
    match period {
        Period::Year { year } if today.year() == *year => {
            let month = Period::Month { year: *year, month: today.month() };
            month_window(&month, MONTHS)
        }
        _ => month_window(period, MONTHS),
    }
}

/// `n` months ending with the month of the period's last day, oldest first (`2026-06` ...).
pub fn month_window(period: &Period, n: usize) -> Vec<String> {
    let last = (period.end() - Duration::days(1)).date_naive();
    let (mut year, mut month) = (last.year(), last.month());
    let mut months = Vec::with_capacity(n);
    for _ in 0..n {
        months.push(format!("{year:04}-{month:02}"));
        if month == 1 {
            (year, month) = (year - 1, 12);
        } else {
            month -= 1;
        }
    }
    months.reverse();
    months
}

/// Every skill of a vocabulary over some documents: documents per month, and per skill and month
/// the documents naming it.
struct Tally {
    documents: Vec<u32>,
    table: Vec<Vec<u32>>,
}

impl Tally {
    /// Documents outside `months` are ignored.
    fn new<'a>(docs: impl IntoIterator<Item = &'a Doc>, vocab: &Vocabulary, months: &[String]) -> Self {
        let column: HashMap<&str, usize> = months.iter().enumerate().map(|(i, m)| (m.as_str(), i)).collect();
        let mut documents = vec![0u32; months.len()];
        let mut table: Vec<Vec<u32>> = vec![vec![0; months.len()]; vocab.names.len()];
        for doc in docs {
            let Some(&at) = column.get(doc.month.as_str()) else { continue };
            documents[at] += 1;
            for skill in vocab.skills_in(&doc.text) {
                table[skill][at] += 1;
            }
        }
        Self { documents, table }
    }

    fn total(&self, skill: usize) -> u32 {
        self.table.get(skill).map_or(0, |row| row.iter().sum())
    }

    fn direction(&self, skill: usize) -> Direction {
        self.table.get(skill).map_or(Direction::Unclear, |row| direction(row, &self.documents))
    }

    /// The scope's list: skills that appear at all, most mentioned first, ties by name, at most
    /// [`MAX_SKILLS`], each with its direction and marks.
    fn scope(&self, vocab: &Vocabulary, marks: &Marks) -> ScopeCounts {
        let mut skills: Vec<SkillCount> = (0..vocab.names.len())
            .filter(|&i| self.total(i) > 0)
            .map(|i| SkillCount {
                skill: vocab.names[i].clone(),
                by_month: self.table[i].clone(),
                total: self.total(i),
                direction: self.direction(i),
                in_profile: marks.profile.contains(&i),
                wished: marks.wished.contains(&i),
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

/// Skill frequency over `docs` (those outside `months` are ignored): per skill and month the
/// number of documents that name it. Most mentioned first, ties by name, at most [`MAX_SKILLS`].
#[cfg(test)]
pub(super) fn count_skills(docs: &[Doc], vocab: &Vocabulary, months: &[String]) -> SkillCounts {
    let ScopeCounts { documents, skills } = Tally::new(docs, vocab, months).scope(vocab, &Marks::default());
    SkillCounts { months: months.to_vec(), documents, skills, ..SkillCounts::default() }
}

/// The sums of the two halves of the window: the first `n/2` months, then the rest.
fn halves(values: &[u32]) -> (u64, u64) {
    let (first, second) = values.split_at(values.len() / 2);
    let sum = |v: &[u32]| v.iter().map(|&n| u64::from(n)).sum::<u64>();
    (sum(first), sum(second))
}

/// Does each half of the window hold at least `min` documents?
fn halves_have(documents: &[u32], min: u32) -> bool {
    let (first, second) = halves(documents);
    first >= u64::from(min) && second >= u64::from(min)
}

/// Where a skill's share of the documents is going: the share in the second half of the months
/// (`mentions / documents`) against the first half. `Unclear` when a half has fewer than
/// [`MIN_HALF_DOCS`] documents or the skill fewer than [`MIN_MENTIONS`] mentions in all. Rising
/// when the share grows by at least [`MIN_SHARE_CHANGE_PERMILLE`] per mille *and* by at least
/// [`MIN_RELATIVE_CHANGE_PERCENT`] percent of the first half's share (any growth from zero
/// counts as relative growth); falling when it drops by both; steady otherwise. Computed in
/// integers, so a change exactly at a threshold counts.
pub fn direction(by_month: &[u32], documents: &[u32]) -> Direction {
    let (d1, d2) = halves(documents);
    let (m1, m2) = halves(by_month);
    if d1 < u64::from(MIN_HALF_DOCS) || d2 < u64::from(MIN_HALF_DOCS) || m1 + m2 < u64::from(MIN_MENTIONS) {
        return Direction::Unclear;
    }
    let (d1, d2, m1, m2) = (i128::from(d1), i128::from(d2), i128::from(m1), i128::from(m2));
    // share2 - share1 = (m2*d1 - m1*d2) / (d1*d2); share1 = m1/d1.
    let change = m2 * d1 - m1 * d2;
    let big_enough = |change: i128| 1000 * change >= MIN_SHARE_CHANGE_PERMILLE * d1 * d2;
    let relative_enough = |change: i128| 100 * change >= MIN_RELATIVE_CHANGE_PERCENT * m1 * d2;
    if change > 0 && big_enough(change) && relative_enough(change) {
        Direction::Rising
    } else if change < 0 && big_enough(-change) && relative_enough(-change) {
        Direction::Falling
    } else {
        Direction::Steady
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
/// An `Unclear` direction (too little data to tell) is treated like steady: only the share rule
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
            let (direction, share_permille) = (field.direction(i), super::permille(u64::from(field.total(i)), documents));
            let bucket = gap_bucket(direction, marks.profile.contains(&i), share_permille)?;
            Some(GapSkill { skill: vocab.names[i].clone(), bucket, direction, share_permille })
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
/// posting. No mention at all is "no data" (0, `Unclear`).
fn wish_signals(wishes: &[(String, WishKind, bool)], docs: &[&Doc], profile_skills: &[String], months: &[String]) -> Vec<WishSignal> {
    let wished: Vec<String> = wishes.iter().filter(|(_, kind, _)| *kind == WishKind::Skill).map(|(w, _, _)| w.clone()).collect();
    let vocab = Vocabulary::new(&[profile_skills, &wished[..]].concat());
    let tally = Tally::new(docs.iter().copied(), &vocab, months);
    wishes
        .iter()
        .map(|(wish, kind, avoided)| {
            let (direction, mentions) = match kind {
                WishKind::Skill => match vocab.find(wish).first() {
                    Some(&i) => (tally.direction(i), tally.total(i)),
                    None => (Direction::Unclear, 0),
                },
                WishKind::Domain => {
                    let column: HashMap<&str, usize> = months.iter().enumerate().map(|(i, m)| (m.as_str(), i)).collect();
                    let mut by_month = vec![0u32; months.len()];
                    for doc in docs {
                        if let Some(&at) = column.get(doc.month.as_str()) {
                            if mentions_lowered(&doc.lower, wish) {
                                by_month[at] += 1;
                            }
                        }
                    }
                    (direction(&by_month, &tally.documents), by_month.iter().sum())
                }
            };
            let direction = if mentions == 0 { Direction::Unclear } else { direction };
            WishSignal { wish: wish.clone(), kind: *kind, avoided: *avoided, direction, mentions }
        })
        .collect()
}

/// Topics the extraction found in at least [`MIN_TERM_ITEMS`] items that the vocabulary does not
/// know (nor are they generic short words): most items first, ties by term, at most [`MAX_TERMS`].
fn uncatalogued(extractions: &[Extraction], vocab: &Vocabulary) -> Vec<TermCount> {
    let mut items: HashMap<String, HashSet<i64>> = HashMap::new();
    for extraction in extractions {
        for topic in &extraction.topics {
            let term = topic.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
            if term.chars().count() <= MAX_GENERIC_CHARS || !vocab.find(&term).is_empty() {
                continue;
            }
            items.entry(term).or_default().insert(extraction.item_id);
        }
    }
    let mut terms: Vec<TermCount> = items
        .into_iter()
        .map(|(term, ids)| TermCount { term, items: u32::try_from(ids.len()).unwrap_or(u32::MAX) })
        .filter(|t| t.items >= MIN_TERM_ITEMS)
        .collect();
    terms.sort_by(|a, b| b.items.cmp(&a.items).then_with(|| a.term.cmp(&b.term)));
    terms.truncate(MAX_TERMS);
    terms
}

/// Everything [`count`] computes, from what it read: the market's `postings`, the user's `leads`
/// (both already limited to the window), the extraction of the period's items.
pub(super) fn market_counts(
    frame: &MarketFrame,
    prefs: &Preferences,
    postings: &[Doc],
    leads: &[Doc],
    extractions: &[Extraction],
    months: &[String],
) -> SkillCounts {
    // The market vocabulary knows the background, never the wishes.
    let vocab = Vocabulary::new(&frame.skills);
    let wishes = wish_list(prefs);
    let wished: Vec<String> = wishes.iter().filter(|(_, kind, _)| *kind == WishKind::Skill).map(|(w, _, _)| w.clone()).collect();
    let marks = Marks { profile: vocab.find_all(&frame.skills), wished: vocab.find_all(&wished) };

    let all: Vec<&Doc> = postings.iter().collect();
    let global = Tally::new(all.iter().copied(), &vocab, months).scope(&vocab, &marks);
    let local_docs: Option<Vec<&Doc>> = frame.has_place().then(|| postings.iter().filter(|d| frame.is_local(&d.lower)).collect());
    let local = local_docs.as_ref().map(|docs| Tally::new(docs.iter().copied(), &vocab, months).scope(&vocab, &marks));
    let (gap_scope, scope_docs) = match (&local_docs, &local) {
        (Some(docs), Some(counts)) if halves_have(&counts.documents, MIN_HALF_DOCS) => (GapScope::Local, docs.as_slice()),
        _ => (GapScope::Global, all.as_slice()),
    };
    let field = Tally::new(scope_docs.iter().copied().filter(|d| frame.in_field(&d.lower)), &vocab, months);
    let pipeline = (!leads.is_empty()).then(|| Tally::new(leads, &vocab, months).scope(&vocab, &marks));

    SkillCounts {
        version: super::MARKET_SCAN_VERSION,
        months: months.to_vec(),
        documents: global.documents,
        skills: global.skills,
        local,
        place: frame.place_label(),
        field: frame.role_words.clone(),
        pipeline,
        gap_scope,
        gap: gap_list(&field, &vocab, &marks),
        uncatalogued: uncatalogued(extractions, &vocab),
        wishes: wish_signals(&wishes, scope_docs, &frame.skills, months),
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

pub(super) async fn count(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    let months = window_for(&s.period, Utc::now().date_naive());
    let in_window = |month: &str| months.iter().any(|m| m == month);

    // A backfilled Hacker News source holds thousands of postings: read a few at a time.
    let candidates: Vec<(String, TrendItem)> = env
        .store
        .list_trend_items(None)?
        .into_iter()
        .map(|item| (month_of(item.published_at.unwrap_or(item.fetched_at)), item))
        .filter(|(month, item)| item.url.starts_with(HN_ITEM_PREFIX) && in_window(month))
        .collect();
    let postings: Vec<Doc> = futures::stream::iter(candidates)
        .map(|(month, item)| async move { (month, read_text(&item).await) })
        .buffered(TEXT_READS)
        .filter_map(|(month, text)| std::future::ready(is_hiring_posting(&text).then(|| Doc::new(month, text))))
        .collect()
        .await;
    let mut leads = Vec::new();
    for state in [LeadState::New, LeadState::Promoted, LeadState::Dismissed] {
        for lead in env.store.list_leads(state)? {
            let month = month_of(lead.found_at);
            if in_window(&month) {
                leads.push(Doc::new(month, lead_text(&lead).await));
            }
        }
    }

    let prefs = env.store.preferences().unwrap_or_else(|_| {
        tracing::warn!("the saved preferences could not be read; the digest has no place and no wishes");
        Preferences::default()
    });
    let frame = MarketFrame::new(env.config.profile.as_ref(), &prefs);
    let counts = market_counts(&frame, &prefs, &postings, &leads, &s.digest.extractions, &months);
    let sum = |docs: &[u32]| docs.iter().sum::<u32>();
    tracing::info!(
        documents = sum(&counts.documents),
        local_documents = counts.local.as_ref().map_or(0, |l| sum(&l.documents)),
        leads = leads.len(),
        skills = counts.skills.len(),
        gap_local = counts.gap_scope == GapScope::Local,
        gap = counts.gap.len(),
        uncatalogued = counts.uncatalogued.len(),
        wishes = counts.wishes.len(),
        "skill frequency counted"
    );
    s.digest.counts = Some(counts);
    Ok(s)
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

    fn months() -> Vec<String> {
        ["2026-09", "2026-10", "2026-11"].map(String::from).to_vec()
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
        let counts = count_skills(&docs, &Vocabulary::new(&[]), &months());

        assert_eq!(counts.months, months());
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
        let a = count_skills(&docs, &Vocabulary::new(&[]), &months());
        let b = count_skills(&docs, &Vocabulary::new(&[]), &months());
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
        let counts = count_skills(&docs, &vocab, &months());
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
        let counts = count_skills(&docs, &vocab, &months());
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
    fn the_window_ends_with_the_month_of_the_period() {
        let w46 = Period::parse("2026-W46").unwrap();
        assert_eq!(month_window(&w46, 3), ["2026-09", "2026-10", "2026-11"]);
        let w1 = Period::parse("2026-W01").unwrap();
        assert_eq!(month_window(&w1, 3), ["2025-11", "2025-12", "2026-01"], "W01 ends on 2026-01-04");
        let q4 = Period::parse("2026-Q4").unwrap();
        assert_eq!(month_window(&q4, 6), ["2026-07", "2026-08", "2026-09", "2026-10", "2026-11", "2026-12"]);
        let today = NaiveDate::from_ymd_opt(2026, 3, 18).unwrap();
        let nov = Period::parse("2026-M11").unwrap();
        assert_eq!(window_for(&nov, today), ["2026-06", "2026-07", "2026-08", "2026-09", "2026-10", "2026-11"]);
        let year = Period::parse("2026").unwrap();
        assert_eq!(
            window_for(&year, today),
            ["2025-10", "2025-11", "2025-12", "2026-01", "2026-02", "2026-03"],
            "the current year ends with the month that has begun"
        );
        let past = Period::parse("2025").unwrap();
        assert_eq!(window_for(&past, today).last().map(String::as_str), Some("2025-12"), "a past year ends with December");
    }

    #[test]
    fn hiring_postings_are_recognised_by_their_header() {
        assert!(is_hiring_posting("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\nAcme | Rust"));
        assert!(!is_hiring_posting("# Rust 2.0 released\n\n450 points"));
        assert!(!is_hiring_posting("# Hacker News, Ask HN: Who wants to be hired?\n\n..."));
        assert!(!is_hiring_posting(""));
    }

    // ------------------------------------------------------------ direction and gap

    #[test]
    fn thin_data_has_no_direction() {
        use Direction::*;
        // Below MIN_HALF_DOCS documents in a half.
        assert_eq!(direction(&[0, 0, 0, 5, 5, 5], &[9, 9, 9, 10, 10, 10]), Unclear, "27 documents in the first half");
        assert_eq!(direction(&[0, 20], &[29, 100]), Unclear);
        assert_eq!(direction(&[0, 20], &[30, 100]), Rising, "exactly MIN_HALF_DOCS is enough");
        // Below MIN_MENTIONS mentions.
        assert_eq!(direction(&[0, 4], &[100, 100]), Unclear, "4 mentions");
        assert_eq!(direction(&[0, 5], &[100, 100]), Rising, "exactly MIN_MENTIONS is enough");
        // An odd number of months: the first half is the shorter one.
        assert_eq!(direction(&[0, 10, 10], &[40, 20, 20]), Rising, "40 against 40 documents");
        assert_eq!(direction(&[0, 10, 10], &[29, 40, 40]), Unclear);
        // Nothing at all.
        assert_eq!(direction(&[], &[]), Unclear);
        assert_eq!(direction(&[0, 0], &[0, 0]), Unclear);
    }

    #[test]
    fn direction_compares_shares_not_counts() {
        use Direction::*;
        // Growth from zero is rising.
        assert_eq!(direction(&[0, 0, 0, 2, 2, 2], &[10, 10, 10, 10, 10, 10]), Rising);
        // Twice the postings, the same share: steady, not rising.
        assert_eq!(direction(&[10, 20], &[100, 200]), Steady);
        // More mentions but a smaller share: falling.
        assert_eq!(direction(&[20, 25], &[100, 400]), Falling);
        // Exactly one point and exactly 25%: rising (4% -> 5%); falling the other way (4% -> 3%).
        assert_eq!(direction(&[4, 5], &[100, 100]), Rising);
        assert_eq!(direction(&[4, 3], &[100, 100]), Falling);
        // One point but only 2.5% of a big share: steady (40% -> 41%, and back).
        assert_eq!(direction(&[40, 41], &[100, 100]), Steady);
        assert_eq!(direction(&[41, 40], &[100, 100]), Steady);
        // A big relative change below one point: steady (1.0% -> 1.9%); exactly one point rises.
        assert_eq!(direction(&[10, 19], &[1000, 1000]), Steady);
        assert_eq!(direction(&[10, 20], &[1000, 1000]), Rising);
        // A drop of 20% of the share: steady (5% -> 4%).
        assert_eq!(direction(&[5, 4], &[100, 100]), Steady);
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

    /// Per month of six: `local` backend postings in Singapore, and 4 elsewhere. In the first three
    /// months Python is in 8 of 12 local ones and Rust in 2; in the last three the other way round.
    /// Kafka is in half of them throughout, Gleam (a wish only) in 3, Kotlin in 1, fintech in 2.
    fn market(local: usize) -> (Vec<String>, Vec<Doc>) {
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
        (months, docs)
    }

    fn counts_of(prefs: &Preferences, postings: &[Doc], leads: &[Doc], extractions: &[Extraction], months: &[String]) -> SkillCounts {
        market_counts(&MarketFrame::new(Some(&cv()), prefs), prefs, postings, leads, extractions, months)
    }

    fn skill<'a>(skills: &'a [SkillCount], name: &str) -> &'a SkillCount {
        skills.iter().find(|s| s.skill == name).unwrap_or_else(|| panic!("no {name} in {skills:?}"))
    }

    #[test]
    fn the_market_is_counted_globally_and_locally_with_directions() {
        let (months, postings) = market(12);
        let counts = counts_of(&wishes(), &postings, &[], &[], &months);

        assert_eq!(counts.documents, [16; 6]);
        let local = counts.local.as_ref().expect("a place is set");
        assert_eq!(local.documents, [12; 6]);
        assert_eq!(counts.place, "Singapore");
        assert_eq!(counts.field, ["backend", "engineer"]);
        assert_eq!(counts.gap_scope, GapScope::Local, "36 local postings in each half");

        assert_eq!(skill(&local.skills, "Rust").direction, Direction::Rising);
        assert_eq!(skill(&local.skills, "Python").direction, Direction::Falling);
        assert_eq!(skill(&local.skills, "Kafka").direction, Direction::Steady);
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
        assert!(counts.pipeline.is_none());
    }

    #[test]
    fn a_thin_local_market_leaves_the_gap_global() {
        let (months, postings) = market(9);
        let counts = counts_of(&place(), &postings, &[], &[], &months);
        assert_eq!(counts.local.as_ref().map(|l| l.documents.clone()), Some(vec![9; 6]));
        assert_eq!(counts.gap_scope, GapScope::Global, "27 local postings in a half");
        assert!(counts.local.as_ref().unwrap().skills.iter().all(|s| s.direction == Direction::Unclear));
        assert!(counts.gap.iter().any(|g| g.skill == "Go"), "Berlin's postings count in the global gap");

        // No place at all: no local scope.
        let nowhere = Preferences { remote: Remote::Onsite, ..Preferences::default() };
        let counts = counts_of(&nowhere, &postings, &[], &[], &months);
        assert!(counts.local.is_none());
        assert_eq!((counts.gap_scope, counts.place.as_str()), (GapScope::Global, ""));
    }

    #[test]
    fn leads_never_change_the_market_counts() {
        let (months, postings) = market(12);
        let leads: Vec<Doc> = (0..40).map(|n| doc(&months[n % 6], "Initech | Backend Engineer | Singapore | Elixir, Elixir, Kafka")).collect();
        let without = counts_of(&wishes(), &postings, &[], &[], &months);
        let with = counts_of(&wishes(), &postings, &leads, &[], &months);

        assert_eq!((&with.documents, &with.skills, &with.local), (&without.documents, &without.skills, &without.local));
        assert_eq!((&with.gap_scope, &with.gap, &with.wishes), (&without.gap_scope, &without.gap, &without.wishes));
        let pipeline = with.pipeline.expect("the leads are counted apart");
        assert_eq!(pipeline.documents.iter().sum::<u32>(), 40);
        assert_eq!(skill(&pipeline.skills, "Elixir").total, 40);
    }

    #[test]
    fn wishes_are_measured_in_order_and_never_shape_the_market() {
        let (months, postings) = market(12);
        let counts = counts_of(&wishes(), &postings, &[], &[], &months);
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
        let plain = counts_of(&other, &postings, &[], &[], &months);
        let unmark = |skills: &[SkillCount]| skills.iter().map(|s| SkillCount { wished: false, ..s.clone() }).collect::<Vec<_>>();
        assert_eq!(unmark(&plain.skills), unmark(&counts.skills));
        assert_eq!(plain.local.as_ref().map(|l| unmark(&l.skills)), counts.local.as_ref().map(|l| unmark(&l.skills)));
        assert_eq!((&plain.documents, plain.gap_scope, &plain.gap), (&counts.documents, counts.gap_scope, &counts.gap));
        assert_ne!(plain.wishes, counts.wishes);
    }

    #[test]
    fn frequent_topics_outside_the_vocabulary_are_listed() {
        let extraction = |id: i64, topics: &[&str]| Extraction { item_id: id, topics: topics.iter().map(|t| t.to_string()).collect(), claims: Vec::new() };
        let extractions = [
            extraction(1, &["Mojo", "rust", "ai", "Platform Engineering", "mojo"]),
            extraction(2, &["mojo", "golang", "platform engineering", "htmx"]),
            extraction(3, &[" MOJO ", "platform  engineering", "htmx", "apache spark"]),
            extraction(4, &["htmx", "go"]),
            extraction(5, &["only twice", "Gleam"]),
            extraction(6, &["only twice", "Gleam"]),
            extraction(7, &["Gleam"]),
        ];
        let vocab = Vocabulary::new(&[]);
        let terms: Vec<(String, u32)> = uncatalogued(&extractions, &vocab).into_iter().map(|t| (t.term, t.items)).collect();
        assert_eq!(
            terms,
            [("gleam".to_string(), 3), ("htmx".to_string(), 3), ("mojo".to_string(), 3), ("platform engineering".to_string(), 3)],
            "known skills (rust, golang, go, apache spark), short words and rare terms are left out; an item counts once"
        );
        let with_profile = Vocabulary::new(&["Gleam".into()]);
        assert!(uncatalogued(&extractions, &with_profile).iter().all(|t| t.term != "gleam"), "a profile skill is in the vocabulary");
    }
}
