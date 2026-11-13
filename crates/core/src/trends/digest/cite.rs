//! `enforce_citations` and `save_report`: what the model wrote is checked in code before it is
//! shown, then saved as a `reports` row.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use anyhow::Result;

use super::{read_text, CitedItem, DigestData, DigestEnv, DigestState, Point, Section};
use crate::store::{NewReport, Report};
use crate::trends::collect::TrendRun;
use crate::trends::period::Period;
use crate::Store;

/// A quote shorter than this (after [`normalize`]) proves nothing: one word is in every text.
const MIN_QUOTE_CHARS: usize = 10;
/// Citations kept per point.
const MAX_CITATIONS: usize = 5;
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

/// Drop every point without a valid citation (an id of an item this digest was built from) and
/// every point whose quote is in none of its cited items. Invalid ids are removed from the points
/// that stay. Counts go in `dropped`.
pub(super) async fn enforce_citations(env: Arc<DigestEnv>, mut s: TrendRun) -> Result<TrendRun> {
    let valid: HashSet<i64> = s.digest.items.iter().copied().collect();
    let drafted = std::mem::take(&mut s.digest.points);
    let total = drafted.len();
    let mut haystacks: HashMap<i64, String> = HashMap::new();
    let (mut uncited, mut fake_quotes, mut empty) = (0usize, 0usize, 0usize);
    let mut kept: Vec<Point> = Vec::new();

    for mut point in drafted {
        point.text = one_line(&point.text);
        point.topic = one_line(&point.topic);
        point.quote = one_line(&point.quote);
        if point.text.is_empty() {
            empty += 1;
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
                if !haystacks.contains_key(id) {
                    let text = match env.store.get_trend_item(*id)? {
                        Some(item) => format!("{}\n{}", item.title, read_text(&item).await),
                        None => String::new(),
                    };
                    haystacks.insert(*id, haystack(&text));
                }
                if haystacks.get(id).is_some_and(|h| quote_found(&point.quote, h)) {
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
        point.citations = ids;
        kept.push(point);
    }
    kept.sort_by_key(|p| Section::ALL.iter().position(|s| *s == p.section));

    tracing::info!(drafted = total, kept = kept.len(), uncited, fake_quotes, empty, "digest citations enforced");
    s.digest.dropped = u32::try_from(uncited + fake_quotes + empty).unwrap_or(u32::MAX);
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

/// The readable digest: sections of points, each point followed by its sources as links, the
/// quote under it, and a table of the most mentioned skills.
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
            if point.topic.is_empty() {
                let _ = write!(md, "- {}", point.text);
            } else {
                let _ = write!(md, "- **{}**: {}", point.topic, point.text);
            }
            let links: Vec<String> = point
                .citations
                .iter()
                .map(|id| match url_of.get(id) {
                    Some(url) => format!("[{id}]({url})"),
                    None => format!("[{id}]"),
                })
                .collect();
            let _ = writeln!(md, " (sources: {})", links.join(", "));
            if !point.quote.is_empty() {
                let _ = writeln!(md, "  > {}", point.quote);
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
            let _ = write!(md, "| {} |", skill.skill);
            for n in &skill.by_month {
                let _ = write!(md, " {n} |");
            }
            let _ = writeln!(md, " {} |", skill.total);
        }
        md.push('\n');
    }
    if data.dropped > 0 {
        let _ = writeln!(
            md,
            "_{} point(s) were left out: no valid source, or a quote that is not in the source._",
            data.dropped
        );
    }
    md
}
