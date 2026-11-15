//! `enforce_citations` and `save_report`: what the model wrote is checked in code before it is
//! shown, then saved as a `reports` row.
//!
//! Besides the citations, a point must agree with what the app counted ([`CodeFacts`]): a rising
//! or falling skill must not contradict the direction code computed, a `for_you` point is about a
//! skill of the gap, a `wishes_check` point about one of the user's wishes.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;

use super::count::Vocabulary;
use super::{read_text, CitedItem, DigestData, DigestEnv, DigestState, Direction, GapBucket, GapScope, Point, Section, SkillCounts, WishKind};
use crate::engine::store::{NewReport, Report};
use crate::engine::trends::collect::TrendRun;
use crate::engine::trends::period::Period;
use crate::engine::Store;

/// A quote shorter than this (after [`normalize`]) proves nothing: one word is in every text.
/// (A whole title is the exception: see [`Searchable::has`].)
const MIN_QUOTE_CHARS: usize = 10;
/// The limits of the prompt, enforced here because the model may ignore them: citations kept per
/// point, points per section, points in all.
pub(super) const MAX_CITATIONS: usize = 5;
pub(super) const MAX_POINTS_PER_SECTION: usize = 4;
pub(super) const MAX_POINTS: usize = 20;
/// Skills in the markdown table of `body_md`.
const TABLE_SKILLS: usize = 10;

/// Lower case, every run of non-alphanumeric characters one space: quotes survive the model's
/// changes of quote marks, dashes, line breaks and case, nothing else.
pub(super) fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut gap = true;
    for c in text.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            gap = false;
        } else if !gap {
            out.push(' ');
            gap = true;
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out
}

/// The text a quote is searched in: normalised and padded, so a match starts and ends at a word.
pub(super) fn haystack(text: &str) -> String {
    format!(" {} ", normalize(text))
}

/// Does `quote` appear in the item's text (`haystack` from [`haystack`])?
pub(super) fn quote_found(quote: &str, haystack: &str) -> bool {
    let quote = normalize(quote);
    quote.chars().count() >= MIN_QUOTE_CHARS && haystack.contains(&format!(" {quote} "))
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// What a quote of a cited item is checked against.
struct Searchable {
    /// [`haystack`] of the title and the text.
    haystack: String,
    /// [`normalize`]d title.
    title: String,
}

impl Searchable {
    /// The quote is in the item's text, or is the item's whole title (which is a quote however short).
    fn has(&self, quote: &str) -> bool {
        let wanted = normalize(quote);
        (!wanted.is_empty() && wanted == self.title) || quote_found(quote, &self.haystack)
    }
}

/// What the app counted that a point must not contradict. Topics are matched ignoring case, and a
/// skill also by the name the vocabulary gives it ("golang" is Go).
struct CodeFacts {
    /// The direction of each skill in the scope the gap was judged on (lower-case name).
    directions: HashMap<String, Direction>,
    /// The skills of the gap (lower case).
    gap: HashSet<String>,
    /// The wishes (lower case).
    wishes: HashSet<String>,
    vocab: Vocabulary,
}

impl CodeFacts {
    fn new(counts: &SkillCounts) -> Self {
        let skills = match (counts.gap_scope, &counts.local) {
            (GapScope::Local, Some(local)) => &local.skills,
            _ => &counts.skills,
        };
        Self {
            directions: skills.iter().map(|s| (key(&s.skill), s.direction)).collect(),
            gap: counts.gap.iter().map(|g| key(&g.skill)).collect(),
            wishes: counts.wishes.iter().map(|w| key(&w.wish)).collect(),
            vocab: Vocabulary::new(&[]),
        }
    }

    /// The keys a topic may stand for: as written, and the vocabulary's name for it.
    fn keys(&self, topic: &str) -> Vec<String> {
        let mut keys = vec![key(topic)];
        if let Some(name) = self.vocab.canonical(topic) {
            keys.push(key(name));
        }
        keys
    }

    fn direction(&self, topic: &str) -> Option<Direction> {
        self.keys(topic).iter().find_map(|k| self.directions.get(k).copied())
    }

    /// Does the point agree with the counts? A skill whose direction is unclear or that was not
    /// counted keeps its rising or falling point: it rests on its cited evidence alone.
    fn agree(&self, point: &Point) -> bool {
        let topic = point.topic.as_str();
        match point.section {
            Section::RisingSkills => !matches!(self.direction(topic), Some(Direction::Falling | Direction::Steady)),
            Section::FallingSkills => !matches!(self.direction(topic), Some(Direction::Rising | Direction::Steady)),
            Section::ForYou => topic.is_empty() || self.keys(topic).iter().any(|k| self.gap.contains(k)),
            Section::WishesCheck => self.wishes.contains(&key(topic)),
            Section::Domains | Section::Funding | Section::Hiring => true,
        }
    }
}

fn key(text: &str) -> String {
    one_line(text).to_lowercase()
}

/// Drop every point without a valid citation (an id of an item this digest was built from), every
/// point whose quote is in none of its cited items, and every point that contradicts the counts
/// ([`CodeFacts::agree`]). Invalid ids are removed from the points that stay, at most
/// [`MAX_CITATIONS`] are kept. Then the first [`MAX_POINTS_PER_SECTION`] points of a section and
/// the first [`MAX_POINTS`] in all stay. Points dropped for the first three reasons are counted in
/// `dropped`.
pub(super) async fn enforce_citations(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    let valid: HashSet<i64> = s.digest.items.iter().copied().collect();
    let facts = CodeFacts::new(&s.digest.counts.clone().unwrap_or_default());
    let drafted = std::mem::take(&mut s.digest.points);
    let total = drafted.len();
    let mut searchable: HashMap<i64, Searchable> = HashMap::new();
    let (mut uncited, mut fake_quotes, mut empty, mut contradicts, mut over_limit) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut kept: Vec<Point> = Vec::new();

    for mut point in drafted {
        point.text = one_line(&point.text);
        point.topic = one_line(&point.topic);
        point.quote = one_line(&point.quote);
        if point.text.is_empty() {
            empty += 1;
            continue;
        }
        if !facts.agree(&point) {
            contradicts += 1;
            continue;
        }
        let mut ids: Vec<i64> = Vec::new();
        for id in &point.citations {
            if valid.contains(id) && !ids.contains(id) && ids.len() < MAX_CITATIONS {
                ids.push(*id);
            }
        }
        if ids.is_empty() {
            uncited += 1;
            continue;
        }
        if !point.quote.is_empty() {
            let mut found = false;
            for id in &ids {
                if !searchable.contains_key(id) {
                    let entry = match env.store.get_trend_item(*id)? {
                        Some(item) => Searchable {
                            haystack: haystack(&format!("{}\n{}", item.title, read_text(&item).await)),
                            title: normalize(&item.title),
                        },
                        None => Searchable { haystack: String::new(), title: String::new() },
                    };
                    searchable.insert(*id, entry);
                }
                if searchable.get(id).is_some_and(|entry| entry.has(&point.quote)) {
                    found = true;
                    break;
                }
            }
            if !found {
                fake_quotes += 1;
                continue;
            }
        }
        if kept.iter().any(|k| k.section == point.section && k.text == point.text) {
            continue;
        }
        if kept.len() >= MAX_POINTS || kept.iter().filter(|k| k.section == point.section).count() >= MAX_POINTS_PER_SECTION {
            over_limit += 1;
            continue;
        }
        point.citations = ids;
        kept.push(point);
    }
    kept.sort_by_key(|p| Section::ALL.iter().position(|s| *s == p.section));

    tracing::info!(drafted = total, kept = kept.len(), uncited, fake_quotes, empty, contradicts, over_limit, "digest citations enforced");
    s.digest.dropped = u32::try_from(uncited + fake_quotes + empty + contradicts).unwrap_or(u32::MAX);
    s.digest.points = kept;
    Ok(s)
}

/// Save the digest as a `reports` row (once per run: a resumed thread finds its row).
pub(super) async fn save_report(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    if let Some(id) = s.digest.report_id {
        if env.store.get_report(id)?.is_some() {
            return Ok(s);
        }
    }
    let data = build_data(&env.store, &s.digest)?;
    let report = save(&env.store, &s.period, &data)?;
    tracing::info!(
        report_id = report.id,
        period = %s.period.label(),
        points = data.points.len(),
        cited_items = data.items.len(),
        dropped = data.dropped,
        "digest saved"
    );
    s.digest.report_id = Some(report.id);
    Ok(s)
}

/// The kept points with their cited items resolved. A cited item that vanished from the store
/// since `enforce_citations` is dropped from its point, and a point left without one is dropped.
fn build_data(store: &Store, digest: &DigestState) -> Result<DigestData> {
    let mut cited: HashMap<i64, CitedItem> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();
    let mut dropped = digest.dropped as usize;
    let mut points = Vec::new();
    for point in &digest.points {
        let mut point = point.clone();
        point.citations.retain(|id| {
            if cited.contains_key(id) {
                return true;
            }
            match store.get_trend_item(*id) {
                Ok(Some(item)) => {
                    cited.insert(*id, CitedItem { id: *id, title: item.title, url: item.url });
                    order.push(*id);
                    true
                }
                _ => false,
            }
        });
        if point.citations.is_empty() {
            dropped += 1;
        } else {
            points.push(point);
        }
    }
    Ok(DigestData {
        items: order.iter().filter_map(|id| cited.remove(id)).collect(),
        points,
        counts: digest.counts.clone().unwrap_or_default(),
        items_read: u32::try_from(digest.items.len()).unwrap_or(u32::MAX),
        dropped: u32::try_from(dropped).unwrap_or(u32::MAX),
    })
}

/// Heading of a report: `Weekly digest 2026-W46`, `Quarterly summary 2026-Q4`.
pub fn report_title(period: &Period) -> String {
    match period {
        Period::Week { .. } => format!("Weekly digest {}", period.label()),
        Period::Quarter { .. } => format!("Quarterly summary {}", period.label()),
    }
}

/// Store `data` as the newest report of `period`'s kind: `body_md` is [`render_markdown`], and
/// `citations_json` the data itself.
pub fn save(store: &Store, period: &Period, data: &DigestData) -> Result<Report> {
    store.add_report(&NewReport {
        kind: period.kind().into(),
        period: period.label(),
        title: report_title(period),
        body_md: render_markdown(period, data),
        citations_json: serde_json::to_string(data)?,
    })
}

/// Text written by the model (or the user), safe inside markdown: one line, and nothing in it can
/// become emphasis, a link, an image, HTML or a heading. An address in it is broken up too
/// (`https\://`, `www\.`, `\@`) because some renderers link bare addresses.
fn escape_md(text: &str) -> String {
    let line = one_line(text);
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        let before = &chars[..i];
        let escape = match c {
            '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '|' | '~' | '&' | '@' => true,
            ':' => chars[i + 1..].starts_with(&['/', '/']),
            '.' => {
                let www = before.len() >= 3 && before[before.len() - 3..].iter().all(|c| c.eq_ignore_ascii_case(&'w'));
                www || (i > 0 && before.iter().all(char::is_ascii_digit))
            }
            '#' | '-' | '+' => i == 0,
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A stored address as a markdown link destination: only http(s), normalised, and with the
/// characters that could end the link or start markup percent-encoded. `None` for anything else.
fn link_destination(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok().filter(|u| matches!(u.scheme(), "http" | "https"))?;
    let mut out = String::with_capacity(url.len());
    for c in parsed.as_str().chars() {
        // (`Url` already encodes whitespace, controls and non-ASCII characters.)
        if matches!(c, '(' | ')' | '[' | ']' | '<' | '>' | '\\' | '`' | '"' | '\'' | '*') {
            let _ = write!(out, "%{:02X}", c as u32);
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// The readable digest: sections of points, each point followed by its sources as links, the
/// quote under it, a table of the most mentioned skills, and what code computed for the user (the
/// gap by bucket, the wishes against the market, the terms not in the list). What the model wrote
/// (and the extracted terms) is escaped, and the only links are the stored addresses of the cited
/// items.
pub fn render_markdown(period: &Period, data: &DigestData) -> String {
    let url_of: HashMap<i64, &str> = data.items.iter().map(|i| (i.id, i.url.as_str())).collect();
    let mut md = format!("# {}\n\n", report_title(period));
    for section in Section::ALL {
        let points: Vec<&Point> = data.points.iter().filter(|p| p.section == section).collect();
        if points.is_empty() {
            continue;
        }
        let _ = write!(md, "## {}\n\n", section.title());
        for point in points {
            if point.topic.trim().is_empty() {
                let _ = write!(md, "- {}", escape_md(&point.text));
            } else {
                let _ = write!(md, "- **{}**: {}", escape_md(&point.topic), escape_md(&point.text));
            }
            let links: Vec<String> = point
                .citations
                .iter()
                .map(|id| match url_of.get(id).and_then(|url| link_destination(url)) {
                    Some(destination) => format!("[{id}]({destination})"),
                    None => format!("[{id}]"),
                })
                .collect();
            let _ = writeln!(md, " (sources: {})", links.join(", "));
            if !point.quote.trim().is_empty() {
                let _ = writeln!(md, "  > {}", escape_md(&point.quote));
            }
        }
        md.push('\n');
    }
    if data.points.is_empty() {
        md.push_str("No point could be tied to a source this time.\n\n");
    }
    let counts = &data.counts;
    if !counts.skills.is_empty() {
        let _ = write!(md, "## Skill frequency\n\nDocuments mentioning the skill, per month (counted by the app).\n\n| Skill |");
        for month in &counts.months {
            let _ = write!(md, " {month} |");
        }
        md.push_str(" Total |\n|---|");
        md.push_str(&"---|".repeat(counts.months.len() + 1));
        md.push('\n');
        for skill in counts.skills.iter().take(TABLE_SKILLS) {
            let _ = write!(md, "| {} |", escape_md(&skill.skill));
            for n in &skill.by_month {
                let _ = write!(md, " {n} |");
            }
            let _ = writeln!(md, " {} |", skill.total);
        }
        md.push('\n');
    }
    render_lens(&mut md, counts);
    if data.dropped > 0 {
        let _ = writeln!(
            md,
            "_{} point(s) were left out: no valid source, a quote that is not in the source, or a claim against the app's numbers._",
            data.dropped
        );
    }
    md
}

fn direction_word(direction: Direction) -> &'static str {
    match direction {
        Direction::Rising => "rising",
        Direction::Steady => "steady",
        Direction::Falling => "falling",
        Direction::Unclear => "unclear",
    }
}

/// Per mille as a percentage with one decimal: `125` is `12.5%`.
fn permille_text(permille: u32) -> String {
    format!("{}.{}%", permille / 10, permille % 10)
}

/// The parts of `body_md` computed for the user, from the counts alone (reports saved before they
/// existed have none of them, and render as before).
fn render_lens(md: &mut String, counts: &SkillCounts) {
    if !counts.gap.is_empty() {
        let scope = match counts.gap_scope {
            GapScope::Local => "local",
            GapScope::Global => "global",
        };
        let _ = write!(md, "## Gap, sorted by the app\n\nSkills of the {scope} postings of your field against your profile; share of those postings over the window.\n\n");
        for (bucket, label) in [(GapBucket::BuildOn, "Build on"), (GapBucket::LearnNext, "Learn next"), (GapBucket::AtRisk, "At risk")] {
            let skills: Vec<String> = counts
                .gap
                .iter()
                .filter(|g| g.bucket == bucket)
                .map(|g| format!("{} ({}, {})", escape_md(&g.skill), direction_word(g.direction), permille_text(g.share_permille)))
                .collect();
            if !skills.is_empty() {
                let _ = writeln!(md, "- {label}: {}", skills.join(", "));
            }
        }
        md.push('\n');
    }
    if !counts.wishes.is_empty() {
        md.push_str("## Wishes against the market, measured by the app\n\n| Wish | Kind | Avoid? | Direction | Mentions |\n|---|---|---|---|---|\n");
        for wish in &counts.wishes {
            let kind = match wish.kind {
                WishKind::Skill => "skill",
                WishKind::Domain => "domain",
            };
            let direction = if wish.mentions == 0 { "no data" } else { direction_word(wish.direction) };
            let avoid = if wish.avoided { "yes" } else { "no" };
            let _ = writeln!(md, "| {} | {kind} | {avoid} | {direction} | {} |", escape_md(&wish.wish), wish.mentions);
        }
        md.push('\n');
    }
    if !counts.uncatalogued.is_empty() {
        let terms: Vec<String> = counts.uncatalogued.iter().map(|t| format!("{} ({})", escape_md(&t.term), t.items)).collect();
        let _ = write!(md, "Terms not in the list (items naming them): {}\n\n", terms.join(", "));
    }
}
