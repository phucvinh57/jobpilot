//! A saved market-insight report as a PDF: the report's [`DigestData`] as JSON, rendered by
//! `assets/trends.typ` with the `typst` CLI (the same binary and errors as the CV, so a missing
//! typst is the usual no-typst Problem).
//!
//! ```text
//! let pdf = core.export_trend_report_pdf(report.id, Path::new("/home/me/Market 2026-Q4")).await?;
//! // -> /home/me/Market 2026-Q4.pdf
//! ```
//!
//! The JSON carries text only: the template inserts every value as text, never as markup, and the
//! only links are the cited items' stored http(s) addresses (anything else is shown without one).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use super::digest::cite::link_destination;
use super::digest::{DigestData, Section};
use super::period::Period;
use crate::pipeline::render;
use crate::store::Report;
use crate::Core;

/// The Typst template of the report (`assets/trends.typ`).
pub const TRENDS_TEMPLATE: &str = include_str!("../../../../assets/trends.typ");
/// Skills in the PDF's table.
const PDF_SKILLS: usize = 12;

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A stored address kept as a link only when it is http(s) (the digest's own rule,
/// [`link_destination`]); `""` otherwise.
fn http_url(url: &str) -> String {
    link_destination(url.trim()).unwrap_or_default()
}

/// What the period covers, in words, as the app names it: `Week of 9 November 2026`,
/// `November 2026`, `Q4 2026`, `2026`. The label itself when it cannot be read.
pub fn period_title(label: &str) -> String {
    let Some(period) = Period::parse(label) else { return one_line(label) };
    match period {
        Period::Week { .. } => format!("Week of {}", period.start().format("%-d %B %Y")),
        Period::Month { .. } => period.start().format("%B %Y").to_string(),
        Period::Quarter { year, quarter } => format!("Q{quarter} {year}"),
        Period::Year { year } => year.to_string(),
    }
}

/// The data `assets/trends.typ` reads: the report's sections in [`Section::ALL`] order (empty ones
/// left out), each point with its citations numbered by first use, the most mentioned skills, and
/// the numbered sources.
pub fn export_json(report: &Report, data: &DigestData) -> Value {
    let items: HashMap<i64, &super::CitedItem> = data.items.iter().map(|i| (i.id, i)).collect();
    let mut numbers: HashMap<i64, usize> = HashMap::new();
    let mut sources = Vec::new();
    let mut sections = Vec::new();
    for section in Section::ALL {
        let mut points = Vec::new();
        for point in data.points.iter().filter(|p| p.section == section) {
            let mut cites = Vec::new();
            for id in &point.citations {
                let Some(item) = items.get(id) else { continue };
                let n = *numbers.entry(*id).or_insert_with(|| {
                    sources.push(json!({
                        "n": sources.len() + 1,
                        "title": one_line(&item.title),
                        "url": http_url(&item.url),
                    }));
                    sources.len()
                });
                if !cites.contains(&n) {
                    cites.push(n);
                }
            }
            points.push(json!({
                "topic": one_line(&point.topic),
                "text": one_line(&point.text),
                "quote": one_line(&point.quote),
                "cites": cites,
            }));
        }
        if !points.is_empty() {
            sections.push(json!({ "title": section.title(), "points": points }));
        }
    }
    let counts = &data.counts;
    let skills: Vec<Value> = counts
        .skills
        .iter()
        .take(PDF_SKILLS)
        .map(|s| {
            let by_month: Vec<Value> =
                counts.months.iter().zip(&s.by_month).map(|(month, count)| json!({ "month": month, "count": count })).collect();
            json!({ "skill": one_line(&s.skill), "total": s.total, "direction": s.direction.as_str(), "counts": by_month })
        })
        .collect();
    let months = match (counts.months.first(), counts.months.last()) {
        (Some(first), Some(last)) if first != last => format!("{first} to {last}"),
        (Some(only), _) => only.clone(),
        _ => String::new(),
    };
    let created = report.created_at.date_naive();
    json!({
        "title": one_line(&report.title),
        "period_title": period_title(&report.period),
        "generated": created.format("%-d %B %Y").to_string(),
        "items_read": data.items_read,
        "dropped": data.dropped,
        "sections": sections,
        "months": months,
        "skills": skills,
        "sources": sources,
    })
}

/// `dest` with `.pdf` appended unless it already ends in `.pdf` (any case).
fn pdf_path(dest: &Path) -> PathBuf {
    if dest.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
        return dest.to_path_buf();
    }
    let mut name = dest.as_os_str().to_owned();
    name.push(".pdf");
    PathBuf::from(name)
}

impl Core {
    /// Render report `report_id` as a PDF at `dest` (`.pdf` is appended when missing) and return
    /// the path written. Fails for a missing report, for a report whose data cannot be read
    /// ("rerun it"), and when typst is missing or fails.
    pub async fn export_trend_report_pdf(&self, report_id: i64, dest: &Path) -> Result<PathBuf> {
        let Some(digest) = self.trend_digest(report_id)? else { bail!("no such report") };
        let Some(data) = &digest.data else { bail!("this report's data cannot be read: rerun it to export it") };
        let json = serde_json::to_vec_pretty(&export_json(&digest.report, data))?;

        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        let work = self.paths.data_dir.join("tmp").join(format!("report-{report_id}-{}-{nonce}", std::process::id()));
        let out = pdf_path(dest);
        if out != dest && tokio::fs::try_exists(&out).await.unwrap_or(false) {
            let name = out.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            bail!("{name} already exists: choose it in the save dialog to replace it, or pick another name");
        }
        let result = render_into(&work, &json, &out).await;
        if let Err(e) = tokio::fs::remove_dir_all(&work).await {
            tracing::warn!(report = report_id, "removing the pdf work folder failed: {e}");
        }
        let out = result?;
        tracing::info!(report = report_id, "trend report exported as pdf");
        Ok(out)
    }
}

async fn render_into(work: &Path, json: &[u8], out: &Path) -> Result<PathBuf> {
    tokio::fs::create_dir_all(work).await.with_context(|| format!("creating {}", work.display()))?;
    let work = work.canonicalize().with_context(|| format!("resolving {}", work.display()))?;
    let template = work.join("trends.typ");
    tokio::fs::write(&template, TRENDS_TEMPLATE).await.context("writing trends.typ")?;
    tokio::fs::write(work.join("data.json"), json).await.context("writing the report data")?;
    let pdf = work.join("report.pdf");
    render::compile(&work, &template, "data.json", &pdf).await?;
    tokio::fs::copy(&pdf, out).await.with_context(|| format!("writing {}", out.display()))?;
    Ok(out.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends::digest::{CitedItem, Direction, Point, SkillCount, SkillCounts};

    fn point(section: Section, text: &str, citations: &[i64]) -> Point {
        Point { section, topic: String::new(), text: text.into(), quote: String::new(), citations: citations.to_vec() }
    }

    fn sample() -> DigestData {
        DigestData {
            items: vec![
                CitedItem { id: 7, title: "Rust at Acme".into(), url: "https://acme.test/rust".into() },
                CitedItem { id: 3, title: "Evil\n  #set page(width: 1pt)".into(), url: "javascript:alert(1)".into() },
                CitedItem { id: 9, title: "Funding round".into(), url: "http://news.test/a?b=(c)".into() },
            ],
            points: vec![
                // Out of section order on purpose.
                point(Section::Funding, "Acme raised a round.", &[9, 7]),
                Point { topic: "Rust".into(), quote: "we use Rust\n everywhere".into(), ..point(Section::RisingSkills, "Rust is rising.", &[7, 3, 7]) },
                point(Section::RisingSkills, "Zig too.", &[404]),
            ],
            counts: SkillCounts {
                months: vec!["2026-10".into(), "2026-11".into()],
                skills: vec![SkillCount { skill: "Rust".into(), by_month: vec![2, 5], total: 7, direction: Direction::Rising, ..Default::default() }],
                ..Default::default()
            },
            items_read: 12,
            dropped: 1,
        }
    }

    fn report(period: &str) -> Report {
        Report {
            id: 1,
            kind: "quarterly".into(),
            period: period.into(),
            title: "Quarterly summary 2026-Q4".into(),
            body_md: String::new(),
            citations_json: String::new(),
            created_at: chrono::DateTime::parse_from_rfc3339("2026-11-18T10:00:00Z").unwrap().into(),
        }
    }

    #[test]
    fn json_keeps_section_order_numbers_sources_by_first_use_and_drops_non_http_links() {
        let v = export_json(&report("2026-Q4"), &sample());
        let titles: Vec<&str> = v["sections"].as_array().unwrap().iter().map(|s| s["title"].as_str().unwrap()).collect();
        assert_eq!(titles, [Section::RisingSkills.title(), Section::Funding.title()], "section order, empty ones skipped");
        let rising = &v["sections"][0]["points"];
        assert_eq!(rising[0]["cites"], json!([1, 2]), "item 7 is [1], item 3 is [2], a repeat is not cited twice");
        assert_eq!((rising[0]["topic"].as_str(), rising[0]["quote"].as_str()), (Some("Rust"), Some("we use Rust everywhere")));
        assert_eq!(rising[1]["cites"], json!([]), "an unknown item is not numbered");
        assert_eq!(v["sections"][1]["points"][0]["cites"], json!([3, 1]));
        assert_eq!(
            v["sources"],
            json!([
                { "n": 1, "title": "Rust at Acme", "url": "https://acme.test/rust" },
                { "n": 2, "title": "Evil #set page(width: 1pt)", "url": "" },
                { "n": 3, "title": "Funding round", "url": "http://news.test/a?b=%28c%29" },
            ])
        );
        assert_eq!(v["skills"][0]["counts"], json!([{ "month": "2026-10", "count": 2 }, { "month": "2026-11", "count": 5 }]));
        assert_eq!(v["skills"][0]["direction"], "rising");
        assert_eq!(v["months"], "2026-10 to 2026-11");
        assert_eq!((v["period_title"].as_str(), v["generated"].as_str()), (Some("Q4 2026"), Some("18 November 2026")));
        assert_eq!((v["items_read"].as_u64(), v["dropped"].as_u64()), (Some(12), Some(1)));
    }

    #[test]
    fn periods_read_as_words_and_dest_gets_a_pdf_extension() {
        assert_eq!(period_title("2026-W46"), "Week of 9 November 2026");
        assert_eq!(period_title("2026-M11"), "November 2026");
        assert_eq!(period_title("2026"), "2026");
        assert_eq!(period_title("odd\nlabel"), "odd label");
        assert_eq!(pdf_path(Path::new("/x/report")), Path::new("/x/report.pdf"));
        assert_eq!(pdf_path(Path::new("/x/report.PDF")), Path::new("/x/report.PDF"));
        assert_eq!(pdf_path(Path::new("/x/2026-Q4")), Path::new("/x/2026-Q4.pdf"));
    }

    #[tokio::test]
    async fn a_saved_report_exports_as_a_pdf() {
        let dir = tempfile::tempdir().unwrap();
        let paths = crate::Paths::new(dir.path().join("Dữ liệu xu hướng"));
        let core = Core::open_at(paths, crate::Secrets::memory()).unwrap();
        let saved = core.save_trend_digest(&Period::parse("2026-Q4").unwrap(), &sample()).unwrap();
        let dest = dir.path().join("Báo cáo thị trường");
        let out = core.export_trend_report_pdf(saved.id, &dest).await.unwrap();
        assert_eq!(out, dest.with_extension("pdf"));
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.starts_with(b"%PDF"), "a PDF");
        let links = crate::pipeline::ingest::pdf_links(&bytes);
        assert!(links.iter().any(|l| l == "https://acme.test/rust"), "{links:?}");
        assert!(!links.iter().any(|l| l.starts_with("javascript")), "{links:?}");
        assert!(!core.paths.data_dir.join("tmp").read_dir().unwrap().any(|_| true), "the work folder is removed");
        // An extensionless name from the dialog must not silently replace an existing PDF.
        let err = core.export_trend_report_pdf(saved.id, &dest).await.unwrap_err();
        assert!(err.to_string().contains("Báo cáo thị trường.pdf already exists"), "{err}");
        // ...while the name the dialog confirmed is replaced.
        core.export_trend_report_pdf(saved.id, &out).await.unwrap();

        let err = core.export_trend_report_pdf(9999, &dir.path().join("x")).await.unwrap_err();
        assert!(err.to_string().contains("no such report"));
        let old = core
            .store
            .add_report(&crate::store::NewReport { kind: "weekly".into(), period: "2026-W45".into(), citations_json: "[1, 2]".into(), ..Default::default() })
            .unwrap();
        let err = core.export_trend_report_pdf(old.id, &dir.path().join("x")).await.unwrap_err();
        assert!(err.to_string().contains("rerun it"), "{err}");
    }
}
