//! New terms, found in code over every item: the words the skill vocabulary does not know.
//!
//! The vocabulary only counts what it already knows. A skill nobody listed (a new database, a new
//! practice) shows up here: its words are counted in the documents of every stored item, never in a
//! sample, and ranked by a test.
//!
//! - **a term** is a run of one to three words ([`MAX_WORDS`]) of a document, in lower case, counted
//!   once per document. It cannot cross a punctuation mark or a line, cannot start or end with a
//!   stopword ([`STOPWORDS`]), is not a number, has at least [`MIN_TERM_CHARS`] characters, and is
//!   not one the skill vocabulary knows (nor contains a known skill: those are counted as skills).
//! - **a document is its own words** ([`body`]): the lines the collectors wrote (the thread title of
//!   a posting, the points of a story, a YC company's batch, size and flags) and every address are
//!   dropped first, so the adapters' own wording cannot become a "trend".
//! - **strata are never pooled.** A document belongs to one [`Stratum`] and a term is compared only
//!   against the same kind of document: a month with more stories and fewer postings must not make
//!   the words of stories look new. Each stratum has two sides: for postings the window's baseline
//!   months against its target months (the sides of the skill counts), for every other kind the
//!   previous period's items against this period's.
//! - **a stratum that cannot be compared is not tested.** With fewer than [`MIN_STRATUM_DOCS`]
//!   documents on a side, or a volume (documents per month, per period) that differs by more than
//!   [`MAX_VOLUME_RATIO`] between the sides, a change of mix is what the test would find: it lists
//!   no term at all (its most frequent words would only be its most common ones), and the report
//!   says it was not compared.
//! - **a term is tested** with at least [`MIN_TERM_DOCS`] documents across both sides, by the log-odds
//!   test of the skills ([`stats::log_odds_z`]) with an informative prior of [`TERM_PRIOR`]
//!   documents, so a word seen in a handful of documents cannot win. The tests of a stratum are one
//!   family under [`FALSE_DISCOVERY_RATE`]; a term that survives with a positive score is
//!   `rising`.
//! - **a shorter term is dropped** when a longer one containing it has the same counts on both
//!   sides: "quantum" and "quantum networking" in the same documents is one finding.
//!
//! The words of the documents are interned and counted level by level (all terms of one word, then
//! those of two words whose words are both frequent, then three): a term can only be in as many
//! documents as its rarest part, so a year of postings never holds millions of one-off phrases.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use regex::Regex;

use super::count::{tokens, Vocabulary};
use super::stats::{self, FALSE_DISCOVERY_RATE};
use super::{Stratum, TermCount};

/// A side of a stratum needs at least this many documents to be compared.
pub(super) const MIN_STRATUM_DOCS: u32 = 20;
/// The volumes of the two sides (documents per month, for postings; per period, for the rest) may
/// differ by at most this factor for a stratum to be compared.
pub(super) const MAX_VOLUME_RATIO: f64 = 2.0;
/// A term is tested only when this many documents, across both sides, name it.
pub(super) const MIN_TERM_DOCS: u32 = 8;
/// The prior of the term test, worth this many documents pulled towards the pooled share (see
/// [`stats::log_odds_z`]): informative on purpose, since a term in a few documents is not a finding.
pub(super) const TERM_PRIOR: f64 = 20.0;
/// Terms listed per stratum.
pub(super) const MAX_TERMS_PER_STRATUM: usize = 10;
/// Longest term, in words.
const MAX_WORDS: usize = 3;
/// A term shorter than this ("ai", "ml") is too generic to list.
const MIN_TERM_CHARS: usize = 3;

/// Which of a stratum's two sides a document is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    /// What the period is compared with: the window's months before the period's own, or the
    /// previous period.
    Baseline = 0,
    /// The period itself.
    Target = 1,
}

// ---------------------------------------------------------------- the words of a document

/// The lines of a document the collector wrote, not its author.
const DISCUSSION_MARK: &str = " on Hacker News. Discussion: ";

fn url_pattern() -> &'static Regex {
    static P: OnceLock<Regex> = OnceLock::new();
    P.get_or_init(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+").expect("static regex"))
}

/// The item's own words: its saved `text` without what the adapters of `stratum` add (read from
/// `trends::source`, which this must follow) and without any address.
///
/// - a posting: the header `# Hacker News, <thread title>`;
/// - a story: the line `<n> points and <m> comments on Hacker News. Discussion: <url>`;
/// - a YC company: the sentence `Y Combinator company, <batch>.`, the labels of the facts, and the
///   facts that are the directory's flags and numbers (team size, status, hiring, website);
/// - a feed or page item, and the rest: nothing but the `# ` that marks the title line.
pub(super) fn body(text: &str, stratum: Stratum) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first = true;
    for line in text.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        let kept = if std::mem::take(&mut first) { header_line(line, stratum) } else { fixed_line(line, stratum) };
        if let Some(kept) = kept.filter(|kept| !kept.trim().is_empty()) {
            out.push_str(&url_pattern().replace_all(kept, " "));
            out.push('\n');
        }
    }
    out
}

/// The first line of a document: `# <title>`, or a posting's `# Hacker News, <thread title>`.
fn header_line(line: &str, stratum: Stratum) -> Option<&str> {
    if stratum == Stratum::HiringPosting && line.to_lowercase().starts_with("# hacker news,") {
        return None;
    }
    Some(line.strip_prefix("# ").unwrap_or(line))
}

/// Any later line.
fn fixed_line(line: &str, stratum: Stratum) -> Option<&str> {
    match stratum {
        Stratum::HnStory if line.contains(DISCUSSION_MARK) => None,
        Stratum::Yc => yc_line(line),
        _ => Some(line),
    }
}

/// A line of a YC company's text (`yc::company_item`): the first sentence names the batch, the last
/// lines are facts, each with a label.
fn yc_line(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("Y Combinator company,") {
        // `Y Combinator company, <batch>.` and then the company's one-liner, on the same line.
        return Some(rest.split_once('.').map_or("", |(_, one_liner)| one_liner));
    }
    for label in ["Sector:", "Tags:", "Location:", "Stage:"] {
        if let Some(value) = line.strip_prefix(label) {
            return Some(value);
        }
    }
    let flag = ["Team size:", "Status:", "Hiring:", "Website:"].iter().any(|label| line.starts_with(label));
    (!flag).then_some(line)
}

/// Stretches of a text a term cannot cross: lines, punctuation that separates one thing from the
/// next (`Acme | Rust engineer | Berlin`), and sentences.
fn segments(text: &str) -> impl Iterator<Item = &str> {
    let is_break = |c: char| {
        matches!(
            c,
            '\n' | '\r' | '|' | ',' | ';' | ':' | '(' | ')' | '[' | ']' | '{' | '}' | '!' | '?' | '"' | '\'' | '*'
                | '\u{2018}' | '\u{2019}' | '\u{201c}' | '\u{201d}' | '\u{2013}' | '\u{2014}' | '\u{2022}'
        )
    };
    text.split(is_break).flat_map(|part| part.split(". "))
}

/// Function words and the filler of recruiting and web pages: no term starts or ends with one (a
/// term may hold one inside: "ruby on rails").
const STOPWORDS: &str = "a about above across after again against all almost along already also although always am \
among an and another any anyone anything are around as at away be because been before being below between both but \
by can cannot could did do does doing done down during each either else enough etc even ever every few for from \
further get gets getting give go goes going got had has have having he her here hers herself him himself his how \
however i if in into is it its itself just keep let like made make makes many may me might more most much must my \
myself need needs neither never no nor not now of off often on once one only onto or other others otherwise our \
ours ourselves out over own per perhaps please plus rather really same see seem several shall she should since so \
some something sometimes still such take than that the their theirs them themselves then there therefore these they \
thing things this those though through throughout thus to together too toward towards under unless until up upon us \
use used uses using very via want wants was way ways we well were what whatever when whenever where whether which \
while who whom whose why will with within without would yet you your yours yourself yourselves \
ability able apply applying application applications candidate candidates company contact currently day days \
email experience experienced experiences expertise good great help helping hire hired hires hiring hn include \
includes including interested job jobs join joining knowledge less looking month months opportunity opportunities \
position positions required requirement requirements resume responsibilities role roles send skill skills strong \
team teams work worked working works year years \
aren couldn didn doesn don hasn haven isn ll re shouldn ve wasn weren won wouldn \
click comment comments cookies privacy read share subscribe";

fn stopwords() -> &'static HashSet<&'static str> {
    static SET: OnceLock<HashSet<&'static str>> = OnceLock::new();
    SET.get_or_init(|| STOPWORDS.split_whitespace().collect())
}

/// How many words the stoplist holds.
#[cfg(test)]
fn stoplist_size() -> usize {
    stopwords().len()
}

/// A distinct word of a stratum's documents.
struct Word {
    text: String,
    /// May stand at either end of a term: not a stopword, not a single character, not a number
    /// (`2026`, `5+`: a term that starts or ends with one says nothing).
    edge: bool,
}

impl Word {
    fn new(text: String) -> Self {
        let numeric = text.chars().any(|c| c.is_ascii_digit()) && text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | '#'));
        let edge = !numeric && !stopwords().contains(text.as_str()) && text.chars().count() > 1;
        Self { text, edge }
    }
}

/// Separates segments in a document's word ids.
const BREAK: u32 = u32::MAX;

/// Up to [`MAX_WORDS`] word ids, the unused ones [`BREAK`]: a term as a small key.
type Gram = [u32; MAX_WORDS];

fn gram_of(words: &[u32]) -> Gram {
    let mut gram = [BREAK; MAX_WORDS];
    gram[..words.len()].copy_from_slice(words);
    gram
}

// ---------------------------------------------------------------- one stratum

/// The documents of one stratum on its two sides, kept as word ids.
struct Corpus {
    stratum: Stratum,
    /// What each side spans, for the volume check: months for postings, 1 (a period) for the rest.
    units: [u32; 2],
    /// Documents per side.
    docs: [u32; 2],
    ids: HashMap<String, u32>,
    words: Vec<Word>,
    /// `(side, word ids)` per document, [`BREAK`] after each segment.
    texts: Vec<(usize, Vec<u32>)>,
}

impl Corpus {
    fn new(stratum: Stratum, units: [u32; 2]) -> Self {
        Self { stratum, units, docs: [0; 2], ids: HashMap::new(), words: Vec::new(), texts: Vec::new() }
    }

    fn intern(&mut self, word: String) -> u32 {
        if let Some(&id) = self.ids.get(&word) {
            return id;
        }
        let id = u32::try_from(self.words.len()).unwrap_or(BREAK - 1).min(BREAK - 1);
        self.ids.insert(word.clone(), id);
        self.words.push(Word::new(word));
        id
    }

    fn add(&mut self, side: Side, text: &str) {
        let text = body(text, self.stratum);
        let mut words = Vec::new();
        for segment in segments(&text) {
            let before = words.len();
            for (_, lower) in tokens(segment) {
                words.push(self.intern(lower));
            }
            if words.len() > before {
                words.push(BREAK);
            }
        }
        self.docs[side as usize] += 1;
        self.texts.push((side as usize, words));
    }

    /// Can the two sides be tested against each other (see the module docs)?
    fn comparable(&self) -> bool {
        let [before, now] = self.docs;
        if before < MIN_STRATUM_DOCS || now < MIN_STRATUM_DOCS {
            return false;
        }
        let rate = |docs: u32, units: u32| f64::from(docs) / f64::from(units.max(1));
        let (before, now) = (rate(before, self.units[0]), rate(now, self.units[1]));
        before.max(now) <= MAX_VOLUME_RATIO * before.min(now)
    }

    /// Per word run of one to three words, the documents naming it on each side, for the runs in
    /// at least [`MIN_TERM_DOCS`] documents. A run of more words is counted only when both its
    /// shorter halves are frequent: it cannot be in more documents than they are.
    fn frequent_runs(&self) -> HashMap<Gram, [u32; 2]> {
        let mut found: HashMap<Gram, [u32; 2]> = HashMap::new();
        let mut shorter: HashSet<Gram> = HashSet::new();
        let mut seen: Vec<Gram> = Vec::new();
        for n in 1..=MAX_WORDS {
            let mut counts: HashMap<Gram, [u32; 2]> = HashMap::new();
            for (side, words) in &self.texts {
                seen.clear();
                for run in words.windows(n).filter(|run| !run.contains(&BREAK)) {
                    if n > 1 && !(shorter.contains(&gram_of(&run[..n - 1])) && shorter.contains(&gram_of(&run[1..]))) {
                        continue;
                    }
                    seen.push(gram_of(run));
                }
                seen.sort_unstable();
                seen.dedup();
                for gram in &seen {
                    counts.entry(*gram).or_default()[*side] += 1;
                }
            }
            counts.retain(|_, k| k[0] + k[1] >= MIN_TERM_DOCS);
            shorter = counts.keys().copied().collect();
            found.extend(counts);
        }
        found
    }

    /// May the word run stand as a term?
    fn is_term(&self, run: &[usize]) -> bool {
        let (Some(&first), Some(&last)) = (run.first(), run.last()) else { return false };
        self.words[first].edge && self.words[last].edge
    }

    /// The terms in enough documents that the vocabulary does not know, with their counts
    /// `[baseline, target]`, by term; a term whose counts equal those of a longer term containing it
    /// is left out.
    fn candidates(&self, vocab: &Vocabulary) -> Vec<(String, [u32; 2])> {
        let mut rows: HashMap<String, [u32; 2]> = HashMap::new();
        for (gram, k) in self.frequent_runs() {
            let run: Vec<usize> = gram.iter().take_while(|&&i| i != BREAK).map(|&i| i as usize).collect();
            if !self.is_term(&run) {
                continue;
            }
            let term = run.iter().map(|&i| self.words[i].text.as_str()).collect::<Vec<_>>().join(" ");
            if term.chars().count() >= MIN_TERM_CHARS && vocab.find(&term).is_empty() {
                rows.insert(term, k);
            }
        }
        let mut subsumed: HashSet<String> = HashSet::new();
        for (term, k) in &rows {
            let words: Vec<&str> = term.split(' ').collect();
            for len in 1..words.len() {
                for start in 0..=words.len() - len {
                    let part = words[start..start + len].join(" ");
                    if rows.get(&part) == Some(k) {
                        subsumed.insert(part);
                    }
                }
            }
        }
        let mut rows: Vec<(String, [u32; 2])> = rows.into_iter().filter(|(term, _)| !subsumed.contains(term)).collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows
    }

    /// The stratum's terms to show (at most [`MAX_TERMS_PER_STRATUM`]) and whether it was compared.
    /// A compared stratum shows its tested risers first, by score, then, to fill, the terms that
    /// grew most without passing the test (not marked rising). Never the most frequent ones: those
    /// are the common words of the kind of item ("engineer", "remote"), in every period. A stratum
    /// that was not compared shows none.
    fn terms(&self, vocab: &Vocabulary) -> (Vec<TermCount>, bool) {
        struct Row {
            term: String,
            k: [u32; 2],
            z: f64,
            rising: bool,
        }
        let comparable = self.comparable();
        let [before, now] = self.docs;
        let candidates = self.candidates(vocab);
        let rows: Vec<Row> = if comparable {
            let z: Vec<f64> = candidates.iter().map(|(_, k)| stats::log_odds_z(k[1], now, k[0], before, TERM_PRIOR)).collect();
            let survives = stats::benjamini_hochberg(&z, FALSE_DISCOVERY_RATE);
            candidates.into_iter().zip(z).zip(survives).map(|(((term, k), z), survives)| Row { term, k, z, rising: survives && z > 0.0 }).collect()
        } else {
            Vec::new()
        };

        let mut risers: Vec<&Row> = rows.iter().filter(|r| r.rising).collect();
        risers.sort_by(|a, b| b.z.total_cmp(&a.z).then_with(|| a.term.cmp(&b.term)));
        risers.truncate(MAX_TERMS_PER_STRATUM);
        let mut level: Vec<&Row> = rows.iter().filter(|r| !r.rising && r.z > 0.0).collect();
        level.sort_by(|a, b| b.z.total_cmp(&a.z).then_with(|| a.term.cmp(&b.term)));
        let room = MAX_TERMS_PER_STRATUM - risers.len();
        let terms = risers
            .into_iter()
            .chain(level.into_iter().take(room))
            .map(|r| TermCount { term: r.term.clone(), items: r.k[1], stratum: self.stratum, z_centi: stats::centi(r.z), rising: r.rising })
            .collect();
        (terms, comparable)
    }
}

// ---------------------------------------------------------------- every stratum

/// What the terms say: the terms to show, postings first, and the strata that had items this
/// period but could not be compared.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct TermReport {
    pub terms: Vec<TermCount>,
    pub not_comparable: Vec<Stratum>,
}

/// The documents of every [`Stratum`], each kept apart.
pub(super) struct Corpora {
    /// One per stratum, in the order of [`Stratum::ALL`].
    corpora: Vec<Corpus>,
}

impl Corpora {
    /// For a window of `baseline_months` months before the period's own `target_months`: the
    /// postings' sides span that many months each, every other stratum's side is one period.
    pub fn new(baseline_months: u32, target_months: u32) -> Self {
        let corpora = Stratum::ALL
            .iter()
            .map(|&stratum| {
                let units = if stratum == Stratum::HiringPosting { [baseline_months, target_months] } else { [1, 1] };
                Corpus::new(stratum, units)
            })
            .collect();
        Self { corpora }
    }

    /// Count a document (its saved text, as the adapter wrote it) on a side of its stratum.
    pub fn add(&mut self, stratum: Stratum, side: Side, text: &str) {
        self.corpora[stratum as usize].add(side, text);
    }

    /// Documents of a stratum on a side.
    #[cfg(test)]
    pub fn documents(&self, stratum: Stratum, side: Side) -> u32 {
        self.corpora[stratum as usize].docs[side as usize]
    }

    /// The terms of every stratum, in the order of [`Stratum::ALL`], and the strata not compared.
    pub fn report(&self, vocab: &Vocabulary) -> TermReport {
        let mut report = TermReport::default();
        for corpus in &self.corpora {
            let (terms, comparable) = corpus.terms(vocab);
            report.terms.extend(terms);
            if !comparable && corpus.docs[Side::Target as usize] > 0 {
                report.not_comparable.push(corpus.stratum);
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vocab() -> Vocabulary {
        Vocabulary::new(&[])
    }

    // The texts as the adapters write them (`trends::source`): the digest test
    // `collector_boilerplate_never_reaches_the_terms` runs the real adapters against these.

    fn posting(text: &str) -> String {
        format!("# Hacker News, Ask HN: Who is hiring? (November 2026)\n\n{text}")
    }

    fn story(title: &str, points: u32, comments: u32, body: &str) -> String {
        let mut text = format!("# {title}\n\n{points} points and {comments} comments on Hacker News. Discussion: https://news.ycombinator.com/item?id=1");
        if !body.is_empty() {
            text.push_str(&format!("\n\n{body}"));
        }
        text
    }

    fn company(name: &str, one_liner: &str, long: &str) -> String {
        format!(
            "# {name}\n\nY Combinator company, Fall 2026. {one_liner}\n\n{long}\n\nSector: B2B (Fintech)\nTags: payments, api\nTeam size: 12\nLocation: San Francisco, CA, USA\nStage: Early\nStatus: Active\nHiring: yes\nWebsite: https://{name}.test"
        )
    }

    fn feed_item(title: &str, body: &str) -> String {
        format!("# {title}\n\n{body}")
    }

    fn words_of(text: &str, stratum: Stratum) -> Vec<String> {
        segments(&body(text, stratum)).flat_map(tokens).map(|(_, lower)| lower).collect()
    }

    #[test]
    fn what_the_collectors_wrote_is_not_the_items_own_words() {
        // A posting: the thread's header goes, the posting stays; addresses go.
        let text = posting("Acme | Senior Rust Engineer | https://acme.test/jobs?id=7 | see www.acme.test too");
        let own = body(&text, Stratum::HiringPosting);
        assert!(own.contains("Senior Rust Engineer") && own.contains("see"), "{own}");
        for gone in ["Hacker", "hiring", "Ask HN", "November", "https", "acme.test"] {
            assert!(!own.contains(gone), "{gone:?} in {own:?}");
        }

        // A story: its title and its own text stay; the points line and the discussion address go.
        let text = story("Rust 2.0 released", 450, 210, "Async closures are stable now.");
        let own = body(&text, Stratum::HnStory);
        assert_eq!(own, "Rust 2.0 released\nAsync closures are stable now.\n");

        // A YC company: no batch sentence, no labels, no flags or numbers; the directory's sector,
        // tags and place, and the company's own words, stay.
        let text = company("Hirey", "Hiring tools for small teams.", "We match engineers with startups.");
        let own = body(&text, Stratum::Yc);
        for gone in ["Y Combinator", "Fall 2026", "Team size", "Status", "Active", "Hiring:", "yes", "Website", "https", "Sector:", "Tags:", "Location:", "Stage:"] {
            assert!(!own.contains(gone), "{gone:?} in {own:?}");
        }
        for kept in ["Hirey", "Hiring tools for small teams.", "We match engineers with startups.", "B2B (Fintech)", "payments, api", "San Francisco", "Early"] {
            assert!(own.contains(kept), "{kept:?} missing in {own:?}");
        }

        // A feed entry or a page: its title line without the marker.
        let own = body(&feed_item("Scaling Postgres & Kafka", "We moved to Postgres 19.\n\nRead https://blog.test/more"), Stratum::Feed);
        assert_eq!(own, "Scaling Postgres & Kafka\nWe moved to Postgres 19.\nRead  \n");
        assert_eq!(body("# Title\n\nbody", Stratum::Other), "Title\nbody\n");

        // Each kind loses only its own boilerplate: a story is not stripped like a posting.
        assert!(body(&posting("x"), Stratum::Other).contains("Hacker News"), "the rule follows the stratum");
        assert_eq!(words_of(&story("T", 1, 2, ""), Stratum::HiringPosting).join(" "), "t 1 points and 2 comments on hacker news discussion", "no header, so nothing is dropped");
    }

    #[test]
    fn terms_are_runs_of_words_that_do_not_cross_punctuation() {
        let text = "Acme | Rust engineer, Berlin | Ruby on Rails (and C++) & node.js. Full-time!\nNext line";
        let runs: Vec<Vec<String>> =
            segments(text).map(|s| tokens(s).into_iter().map(|(_, l)| l).collect::<Vec<_>>()).filter(|run| !run.is_empty()).collect();
        assert_eq!(
            runs,
            [
                vec!["acme"],
                vec!["rust", "engineer"],
                vec!["berlin"],
                vec!["ruby", "on", "rails"],
                vec!["and", "c++"],
                vec!["node.js"],
                vec!["full-time"],
                vec!["next", "line"],
            ]
        );
    }

    /// `n` documents `0..n` of one stratum: each is the text `f(i)` makes.
    fn fill(corpora: &mut Corpora, stratum: Stratum, side: Side, n: usize, f: impl Fn(usize) -> String) {
        for i in 0..n {
            corpora.add(stratum, side, &f(i));
        }
    }

    fn feed(i: usize, extra: &str) -> String {
        feed_item(&format!("Post {i}"), &format!("Weekly notes on shipping. Item {i}. {extra}"))
    }

    fn of(report: &TermReport, stratum: Stratum) -> Vec<&TermCount> {
        report.terms.iter().filter(|t| t.stratum == stratum).collect()
    }

    #[test]
    fn a_term_new_on_the_target_side_ranks_first_and_rises() {
        let mut corpora = Corpora::new(1, 1);
        // 40 documents a side. "quantum networking" is in 30 of the target's and none of the baseline's;
        // "garden tools" is in 20 of each; "cloud pricing" in 10 of the baseline's and 25 of the target's.
        fill(&mut corpora, Stratum::Feed, Side::Baseline, 40, |i| feed(i, &if i < 20 { "Garden tools. ".into() } else if i < 30 { "Cloud pricing. ".into() } else { String::new() }));
        fill(&mut corpora, Stratum::Feed, Side::Target, 40, |i| {
            let mut extra = String::new();
            if i < 30 {
                extra.push_str("Quantum networking. ");
            }
            if i >= 20 {
                extra.push_str("Garden tools. ");
            }
            if i < 25 {
                extra.push_str("Cloud pricing. ");
            }
            feed(i, &extra)
        });
        let report = corpora.report(&vocab());
        assert!(report.not_comparable.is_empty(), "{report:?}");

        let feed_terms = of(&report, Stratum::Feed);
        let first = feed_terms[0];
        assert_eq!((first.term.as_str(), first.items, first.rising), ("quantum networking", 30, true), "{feed_terms:?}");
        assert!(first.z_centi > 196, "{first:?}");
        // The unigrams of a term that always comes with its neighbour are one finding, not three.
        assert!(feed_terms.iter().all(|t| t.term != "quantum" && t.term != "networking"), "{feed_terms:?}");

        let cloud = feed_terms.iter().find(|t| t.term == "cloud pricing").expect("a riser");
        assert!(cloud.rising && cloud.z_centi > 0 && cloud.items == 25, "{cloud:?}");
        assert!(first.z_centi > cloud.z_centi, "by score");

        // The same share on both sides did not move: however frequent, it is not listed.
        assert!(feed_terms.iter().all(|t| t.term != "garden tools" && t.term != "weekly notes"), "{feed_terms:?}");
        // Risers come before what only fills the list (terms that grew without passing the test).
        let first_fill = feed_terms.iter().position(|t| !t.rising).unwrap_or(feed_terms.len());
        assert!(feed_terms[..first_fill].iter().all(|t| t.rising) && feed_terms[first_fill..].iter().all(|t| !t.rising && t.z_centi > 0), "{feed_terms:?}");
        assert!(feed_terms.len() <= MAX_TERMS_PER_STRATUM);
    }

    #[test]
    fn a_term_in_both_sides_equally_never_rises_whatever_the_noise() {
        let mut corpora = Corpora::new(1, 1);
        fill(&mut corpora, Stratum::Feed, Side::Baseline, 60, |i| feed(i, if i % 2 == 0 { "Edge caching." } else { "" }));
        fill(&mut corpora, Stratum::Feed, Side::Target, 60, |i| feed(i, if i % 2 == 1 { "Edge caching." } else { "" }));
        let report = corpora.report(&vocab());
        let candidates: HashMap<String, [u32; 2]> = corpora.corpora[Stratum::Feed as usize].candidates(&vocab()).into_iter().collect();
        assert_eq!(candidates.get("edge caching"), Some(&[30, 30]), "30 of 60 on both sides");
        // Counted, tested, and neither rising nor listed: it did not move.
        assert!(of(&report, Stratum::Feed).is_empty(), "{report:?}");
    }

    #[test]
    fn strata_are_never_pooled() {
        let postings = |corpora: &mut Corpora, new_in_target: usize| {
            // 30 postings a month: baseline 5 months, target 1. "event sourcing" in 6 of the target's
            // 30 and in 3 of 150 baseline postings.
            fill(corpora, Stratum::HiringPosting, Side::Baseline, 150, |i| posting(&format!("Company {i} | Backend | {}", if i % 50 == 0 { "event sourcing" } else { "" })));
            fill(corpora, Stratum::HiringPosting, Side::Target, 30, |i| posting(&format!("Company {i} | Backend | {}", if i < new_in_target { "event sourcing" } else { "" })));
        };
        let mut alone = Corpora::new(5, 1);
        postings(&mut alone, 12);
        let mut with_stories = Corpora::new(5, 1);
        postings(&mut with_stories, 12);
        // Stories jump from 20 to 136 a period, and "event sourcing" is in all of the new ones.
        fill(&mut with_stories, Stratum::HnStory, Side::Baseline, 20, |i| story(&format!("Story {i}"), 10, 1, ""));
        fill(&mut with_stories, Stratum::HnStory, Side::Target, 136, |i| story(&format!("Story {i}"), 10, 1, "Event sourcing everywhere."));

        let (a, b) = (alone.report(&vocab()), with_stories.report(&vocab()));
        let sourcing = |r: &TermReport| of(r, Stratum::HiringPosting).into_iter().find(|t| t.term == "event sourcing").cloned().expect("a posting term");
        assert_eq!(sourcing(&a), sourcing(&b), "a volume jump in stories does not touch the postings' numbers");
        assert!(sourcing(&a).rising, "{:?}", sourcing(&a));

        // The stories were not compared at all (6.8 times the volume, and 20 documents is the minimum)...
        assert_eq!(b.not_comparable, [Stratum::HnStory]);
        // ... so none of their words is listed, not even the new phrase.
        assert!(of(&b, Stratum::HnStory).is_empty());
        // Nothing is pooled: the stories' words are not among the postings' terms.
        assert!(of(&b, Stratum::HiringPosting).iter().all(|t| !t.term.contains("everywhere")));
    }

    #[test]
    fn a_stratum_past_the_volume_ratio_or_under_the_minimum_lists_no_term() {
        let new_term = |i: usize| feed(i, "Zero trust. ");
        // Volumes 2.5 apart: 20 against 50 documents.
        let mut uneven = Corpora::new(1, 1);
        fill(&mut uneven, Stratum::Feed, Side::Baseline, 20, |i| feed(i, ""));
        fill(&mut uneven, Stratum::Feed, Side::Target, 50, new_term);
        let report = uneven.report(&vocab());
        assert_eq!(report.not_comparable, [Stratum::Feed]);
        assert!(of(&report, Stratum::Feed).is_empty());

        // Exactly twice the volume is still compared.
        let mut twice = Corpora::new(1, 1);
        fill(&mut twice, Stratum::Feed, Side::Baseline, 25, |i| feed(i, ""));
        fill(&mut twice, Stratum::Feed, Side::Target, 50, new_term);
        let report = twice.report(&vocab());
        assert!(report.not_comparable.is_empty());
        assert!(of(&report, Stratum::Feed).iter().any(|t| t.term == "zero trust" && t.rising));

        // 19 documents on a side is not enough, however even the volumes.
        let mut thin = Corpora::new(1, 1);
        fill(&mut thin, Stratum::Feed, Side::Baseline, 19, |i| feed(i, ""));
        fill(&mut thin, Stratum::Feed, Side::Target, 40, new_term);
        assert_eq!(thin.report(&vocab()).not_comparable, [Stratum::Feed]);
        let mut empty_baseline = Corpora::new(1, 1);
        fill(&mut empty_baseline, Stratum::Feed, Side::Target, 40, new_term);
        let report = empty_baseline.report(&vocab());
        assert_eq!(report.not_comparable, [Stratum::Feed], "nothing before to compare with");
        assert!(of(&report, Stratum::Feed).iter().all(|t| !t.rising));
        // A stratum with no document this period is nothing to report.
        let mut gone = Corpora::new(1, 1);
        fill(&mut gone, Stratum::Yc, Side::Baseline, 30, |i| company(&format!("Co{i}"), "x", "y"));
        assert!(gone.report(&vocab()).not_comparable.is_empty());
    }

    #[test]
    fn postings_compare_their_volume_per_month() {
        // 5 baseline months of 10 postings and 1 target month of 40: 4 times the rate.
        let mut uneven = Corpora::new(5, 1);
        fill(&mut uneven, Stratum::HiringPosting, Side::Baseline, 50, |i| posting(&format!("Co{i} | Backend")));
        fill(&mut uneven, Stratum::HiringPosting, Side::Target, 40, |i| posting(&format!("Co{i} | Backend")));
        assert_eq!(uneven.report(&vocab()).not_comparable, [Stratum::HiringPosting]);
        // The same months with the same rate (10 a month, 50 and 10 postings) are too few on the target side...
        let mut thin = Corpora::new(5, 1);
        fill(&mut thin, Stratum::HiringPosting, Side::Baseline, 50, |i| posting(&format!("Co{i}")));
        fill(&mut thin, Stratum::HiringPosting, Side::Target, 10, |i| posting(&format!("Co{i}")));
        assert_eq!(thin.report(&vocab()).not_comparable, [Stratum::HiringPosting]);
        // ... and at 30 a month they are compared although 150 against 30 documents differ 5 times.
        let mut even = Corpora::new(5, 1);
        fill(&mut even, Stratum::HiringPosting, Side::Baseline, 150, |i| posting(&format!("Co{i}")));
        fill(&mut even, Stratum::HiringPosting, Side::Target, 30, |i| posting(&format!("Co{i}")));
        assert!(even.report(&vocab()).not_comparable.is_empty());
    }

    #[test]
    fn a_term_needs_eight_documents_to_be_tested() {
        let mut corpora = Corpora::new(1, 1);
        // "feature flags" in 8 documents (all the target's), "dark launches" in 7.
        fill(&mut corpora, Stratum::Feed, Side::Baseline, 30, |i| feed(i, ""));
        fill(&mut corpora, Stratum::Feed, Side::Target, 30, |i| feed(i, &format!("{}{}", if i < 8 { "Feature flags. " } else { "" }, if i >= 23 { "Dark launches. " } else { "" })));
        let report = corpora.report(&vocab());
        let terms = of(&report, Stratum::Feed);
        let flags = terms.iter().find(|t| t.term == "feature flags").expect("8 documents are enough");
        assert!(flags.items == 8 && flags.z_centi > 0, "{flags:?}");
        assert!(terms.iter().all(|t| t.term != "dark launches"), "7 documents are not: {terms:?}");

        // Split across the sides, 8 documents in all are enough too; 7 are not.
        let mut split = Corpora::new(1, 1);
        fill(&mut split, Stratum::Feed, Side::Baseline, 30, |i| feed(i, if i < 4 { "Feature flags. Dark launches." } else { "" }));
        fill(&mut split, Stratum::Feed, Side::Target, 30, |i| feed(i, if i < 4 { "Feature flags." } else if i < 7 { "Dark launches." } else { "" }));
        let candidates: HashMap<String, [u32; 2]> = split.corpora[Stratum::Feed as usize].candidates(&vocab()).into_iter().collect();
        assert_eq!(candidates.get("feature flags"), Some(&[4, 4]), "8 documents across both sides");
        assert!(!candidates.contains_key("dark launches"), "4 and 3 are 7 documents: {candidates:?}");
    }

    #[test]
    fn known_skills_stopword_edges_numbers_and_sub_terms_are_skipped() {
        let mut corpora = Corpora::new(1, 1);
        let text = |i: usize| {
            feed(
                i,
                "We use Rust and Kafka. Looking for strong experience with event sourcing in the team. \
                 Hiring for 2026 and 2027 plus 5+ years. Proof of concept first. Ruby on rails. ML ops. Gleam is fun.",
            )
        };
        fill(&mut corpora, Stratum::Feed, Side::Baseline, 25, text);
        fill(&mut corpora, Stratum::Feed, Side::Target, 25, text);
        // The terms that are counted at all (the same on both sides, so none is listed as moving).
        let feed_corpus = &corpora.corpora[Stratum::Feed as usize];
        let counted = feed_corpus.candidates(&vocab());
        let terms: Vec<&str> = counted.iter().map(|(term, _)| term.as_str()).collect();

        // A skill of the user's profile is known too: no longer a new term.
        // Unknown to the built-in list it is new (as the longest term that holds it) ...
        assert!(terms.contains(&"gleam is fun"), "unknown to the built-in list: {terms:?}");
        // ... and with it in the profile, neither it nor a term holding it is.
        let with_profile = feed_corpus.candidates(&Vocabulary::new(&["Gleam".into()]));
        assert!(with_profile.iter().all(|(term, _)| !term.contains("gleam")));
        // Skills the vocabulary knows (built in, or the user's) are not new, nor is a term holding one.
        for known in ["rust", "kafka", "rust and kafka", "ruby on rails", "rails"] {
            assert!(!terms.contains(&known), "{known} is a known skill: {terms:?}");
        }
        // No stopword at either end, no number, nothing of two characters.
        for bad in ["the", "of the", "proof of", "for strong experience", "strong experience", "years", "plus 5", "2026", "2026 and 2027", "ml", "5+", "in the team", "with event sourcing", "event sourcing in"] {
            assert!(!terms.contains(&bad), "{bad} is not a term: {terms:?}");
        }
        // A stopword inside is fine.
        assert!(terms.contains(&"proof of concept"), "{terms:?}");
        // Every sub-term of a longer term has the same counts here: only the longest stays.
        assert!(terms.contains(&"event sourcing") && !terms.contains(&"event") && !terms.contains(&"sourcing"), "{terms:?}");
        assert!(!terms.contains(&"proof") && !terms.contains(&"concept"), "{terms:?}");
        assert!(terms.contains(&"ml ops") && !terms.contains(&"ops"), "{terms:?}");
    }

    #[test]
    fn a_document_counts_once_however_often_it_names_a_term() {
        let mut corpora = Corpora::new(1, 1);
        fill(&mut corpora, Stratum::Feed, Side::Baseline, 20, |i| feed(i, ""));
        fill(&mut corpora, Stratum::Feed, Side::Target, 20, |i| {
            if i < 9 { feed(i, &"Vector search. ".repeat(10)) } else { feed(i, "") }
        });
        assert_eq!((corpora.documents(Stratum::Feed, Side::Baseline), corpora.documents(Stratum::Feed, Side::Target)), (20, 20));
        let report = corpora.report(&vocab());
        let found = of(&report, Stratum::Feed).into_iter().find(|t| t.term == "vector search").expect("frequent").clone();
        assert_eq!(found.items, 9, "nine documents, ten mentions each");
    }

    #[test]
    fn the_report_lists_postings_first_then_the_other_strata_in_order() {
        let mut corpora = Corpora::new(1, 1);
        for stratum in [Stratum::Other, Stratum::Feed, Stratum::HnStory, Stratum::HiringPosting, Stratum::Yc] {
            let text = |words: &'static str| {
                move |i: usize| match stratum {
                    Stratum::HiringPosting => posting(&format!("Co{i} | {words}")),
                    Stratum::Yc => company(&format!("Co{i}"), words, ""),
                    _ => feed_item(&format!("Item {i}"), words),
                }
            };
            // New in the period, in every kind of item.
            fill(&mut corpora, stratum, Side::Baseline, 25, text("Staff"));
            fill(&mut corpora, stratum, Side::Target, 25, text("Platform engineering"));
        }
        let report = corpora.report(&vocab());
        let order: Vec<Stratum> = report.terms.iter().map(|t| t.stratum).collect();
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(order, sorted);
        assert_eq!(order.first(), Some(&Stratum::HiringPosting));
        assert!(Stratum::ALL.iter().all(|s| order.contains(s)), "{order:?}");
        // The terms are the same words in every stratum, counted apart.
        let same: Vec<&TermCount> = report.terms.iter().filter(|t| t.term == "platform engineering").collect();
        assert_eq!(same.len(), 5, "once per stratum");
        assert!(same.iter().all(|t| t.items == 25));
    }

    #[test]
    fn the_stoplist_is_compact() {
        assert!(stoplist_size() > 150 && stoplist_size() < 400, "{}", stoplist_size());
        assert!(!stopwords().contains("remote") && !stopwords().contains("open"), "topical words are terms");
    }

    #[test]
    fn many_postings_are_counted_in_reasonable_time() {
        // A year's postings: 10,000 documents of about 120 words each, mostly one-off phrases.
        let mut corpora = Corpora::new(12, 12);
        let mut seed = 12345u64;
        let mut word = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            format!("w{}", (seed >> 33) % 4000)
        };
        let started = std::time::Instant::now();
        for i in 0..10_000 {
            let text: String = (0..120).map(|_| word()).collect::<Vec<_>>().join(" ");
            corpora.add(Stratum::HiringPosting, if i < 5_000 { Side::Baseline } else { Side::Target }, &posting(&text));
        }
        let report = corpora.report(&vocab());
        let elapsed = started.elapsed();
        assert!(report.not_comparable.is_empty());
        assert!(elapsed < std::time::Duration::from_secs(60), "{elapsed:?}");
    }
}
