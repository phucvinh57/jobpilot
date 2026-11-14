//! The YC company directory: the newest batches, one item per company (sector, tags, size, hiring
//! flag). YC has no public JSON API; this reads the static `yc-oss` mirror of the directory
//! (`<root>/meta.json` lists the batches, `<root>/batches/<slug>.json` holds one batch). The root is
//! [`Endpoints::yc`](crate::engine::trends::config::Endpoints), so another mirror or a fixture can stand in.

use std::collections::HashMap;

use anyhow::{Context, Result};
use chrono::{TimeZone, Utc};
use serde::Deserialize;

use super::{join, take_new, Ctx, Item};
use crate::engine::pipeline::ingest::clean;
use crate::engine::trends::text::cap;

/// Batches listed with fewer companies are announcements, not batches yet.
const MIN_BATCH_COMPANIES: u32 = 10;
/// Where a company's page lives; the item URL (and so the citation) is built from its slug.
const COMPANY_URL: &str = "https://www.ycombinator.com/companies/";

#[derive(Deserialize)]
struct Meta {
    #[serde(default)]
    batches: HashMap<String, BatchMeta>,
}

#[derive(Deserialize)]
struct BatchMeta {
    #[serde(default)]
    count: u32,
}

/// One company; every field may be missing or null in the mirror.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Company {
    name: Option<String>,
    slug: Option<String>,
    one_liner: Option<String>,
    long_description: Option<String>,
    website: Option<String>,
    all_locations: Option<String>,
    team_size: Option<u32>,
    industry: Option<String>,
    subindustry: Option<String>,
    tags: Option<Vec<String>>,
    batch: Option<String>,
    stage: Option<String>,
    status: Option<String>,
    launched_at: Option<i64>,
    #[serde(rename = "isHiring")]
    is_hiring: Option<bool>,
    top_company: Option<bool>,
}

pub async fn collect(ctx: &Ctx) -> Result<Vec<Item>> {
    let l = ctx.limits();
    let meta: Meta = ctx.http.get(&join(&ctx.endpoints().yc, "meta.json"), &[]).await?.json("the YC directory")?;
    let slugs = newest_batches(&meta, l.yc_batches);

    let mut items = Vec::new();
    let mut failed = None;
    for slug in &slugs {
        let url = join(&ctx.endpoints().yc, &format!("batches/{slug}.json"));
        let companies = async {
            // One broken company must not hide the rest of the batch.
            let all: Vec<serde_json::Value> = ctx.http.get(&url, &[]).await?.json("the YC directory")?;
            Ok::<_, anyhow::Error>(all.into_iter().filter_map(|v| serde_json::from_value::<Company>(v).ok()))
        }
        .await;
        match companies {
            Ok(companies) => items.extend(companies.filter_map(company_item)),
            Err(e) => failed = failed.or(Some(e)),
        }
    }
    // Hiring and top companies first, so the cap keeps the most informative ones.
    items.sort_by_key(|(rank, _)| *rank);
    let items: Vec<Item> = items.into_iter().map(|(_, item)| item).collect();
    match failed {
        Some(e) if items.is_empty() => Err(e).context("reading the newest YC batches"),
        _ => Ok(take_new(ctx, items, l.max_items)),
    }
}

/// `summer-2026`: its year and season (winter < spring < summer < fall).
fn batch_order(slug: &str) -> Option<(i32, u8)> {
    let (season, year) = slug.split_once('-')?;
    let season = ["winter", "spring", "summer", "fall"].iter().position(|s| *s == season)?;
    Some((year.parse().ok()?, u8::try_from(season).ok()?))
}

fn newest_batches(meta: &Meta, n: usize) -> Vec<String> {
    let mut batches: Vec<_> = meta
        .batches
        .iter()
        .filter(|(_, b)| b.count >= MIN_BATCH_COMPANIES)
        .filter_map(|(slug, _)| batch_order(slug).map(|order| (order, slug.clone())))
        .collect();
    batches.sort_by_key(|(order, _)| std::cmp::Reverse(*order));
    batches.into_iter().take(n).map(|(_, slug)| slug).collect()
}

/// The item and its sort rank (0 = hiring, 1 = top company, 2 = the rest).
fn company_item(c: Company) -> Option<(u8, Item)> {
    let name = c.name.as_deref().map(str::trim).filter(|n| !n.is_empty())?;
    let slug = c.slug.as_deref().filter(|s| !s.is_empty() && s.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-'))?;
    let batch = c.batch.as_deref().unwrap_or("unknown batch");
    let mut text = format!("# {name}\n\nY Combinator company, {batch}.");
    if let Some(one) = c.one_liner.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str(&format!(" {one}"));
    }
    if let Some(long) = c.long_description.as_deref().map(clean).filter(|s| !s.is_empty()) {
        text.push_str(&format!("\n\n{}", cap(&long, 4000)));
    }
    let mut facts = Vec::new();
    match (c.industry.as_deref(), c.subindustry.as_deref()) {
        (Some(i), Some(s)) if !s.is_empty() => facts.push(format!("Sector: {i} ({s})")),
        (Some(i), _) => facts.push(format!("Sector: {i}")),
        _ => {}
    }
    if let Some(tags) = c.tags.as_ref().filter(|t| !t.is_empty()) {
        facts.push(format!("Tags: {}", tags.join(", ")));
    }
    if let Some(n) = c.team_size {
        facts.push(format!("Team size: {n}"));
    }
    if let Some(l) = c.all_locations.as_deref().filter(|l| !l.is_empty()) {
        facts.push(format!("Location: {l}"));
    }
    if let Some(s) = c.stage.as_deref().filter(|s| !s.is_empty()) {
        facts.push(format!("Stage: {s}"));
    }
    if let Some(s) = c.status.as_deref().filter(|s| !s.is_empty()) {
        facts.push(format!("Status: {s}"));
    }
    facts.push(format!("Hiring: {}", if c.is_hiring == Some(true) { "yes" } else { "no" }));
    if let Some(w) = c.website.as_deref().filter(|w| !w.is_empty()) {
        facts.push(format!("Website: {w}"));
    }
    text.push_str(&format!("\n\n{}", facts.join("\n")));

    let rank = if c.is_hiring == Some(true) {
        0
    } else if c.top_company == Some(true) {
        1
    } else {
        2
    };
    let item = Item {
        url: format!("{COMPANY_URL}{slug}"),
        title: name.to_string(),
        published_at: c.launched_at.and_then(|t| Utc.timestamp_opt(t, 0).single()),
        text,
    };
    Some((rank, item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::trends::testutil::{ctx, Fixtures};

    const META: &str = r#"{"batches": {
        "summer-2027": {"name": "Summer 2027", "count": 1},
        "fall-2026": {"name": "Fall 2026", "count": 231},
        "summer-2026": {"name": "Summer 2026", "count": 140},
        "winter-2026": {"name": "Winter 2026", "count": 120},
        "unspecified": {"name": "Unspecified", "count": 400},
        "../evil": {"name": "x", "count": 99}
    }}"#;

    const FALL: &str = r#"[
        {"name": "Tire Swing", "slug": "tire-swing", "one_liner": "The AI Affordable Housing Platform", "long_description": "Tire Swing uses AI.\r\n\r\nIt serves compliance staff.", "website": "https://gettireswing.com", "all_locations": "New York City, NY, USA", "team_size": 2, "industry": "B2B", "subindustry": "B2B -> Legal", "launched_at": 1740473580, "tags": ["GovTech", "Legal"], "batch": "Fall 2026", "isHiring": false, "top_company": false, "stage": "Early", "status": "Active"},
        {"name": "Hirey", "slug": "hirey", "one_liner": "Hiring tools", "batch": "Fall 2026", "isHiring": true, "tags": null, "team_size": null},
        {"name": "Top Co", "slug": "top-co", "batch": "Fall 2026", "top_company": true},
        {"name": "No Slug", "batch": "Fall 2026"},
        {"name": "Bad Slug", "slug": "../x"},
        {"slug": "nameless"},
        {"name": "Wrong Types", "slug": "wrong", "team_size": "many"}
    ]"#;

    #[test]
    fn the_newest_real_batches_are_picked_by_year_and_season() {
        let meta: Meta = serde_json::from_str(META).unwrap();
        assert_eq!(newest_batches(&meta, 2), ["fall-2026", "summer-2026"], "the 1-company announcement and odd slugs are skipped");
        assert_eq!(newest_batches(&meta, 9), ["fall-2026", "summer-2026", "winter-2026"]);
        assert_eq!(batch_order("spring-2025"), Some((2025, 1)));
        assert_eq!(batch_order("../evil"), None);
    }

    #[tokio::test]
    async fn companies_become_items_hiring_first() {
        let fx = Fixtures::start().await;
        fx.route("/yc/meta.json", 200, "application/json", META);
        fx.route("/yc/batches/fall-2026.json", 200, "application/json", FALL);
        fx.route("/yc/batches/summer-2026.json", 404, "text/plain", "missing");
        let ctx = ctx(&fx, &[]);

        let items = collect(&ctx).await.unwrap();
        let urls: Vec<_> = items.iter().map(|i| i.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.ycombinator.com/companies/hirey",
                "https://www.ycombinator.com/companies/top-co",
                "https://www.ycombinator.com/companies/tire-swing",
            ],
            "hiring, then top, then the rest; entries without a name or a safe slug are skipped; a failing batch is left out"
        );
        let tire = &items[2];
        assert_eq!(tire.title, "Tire Swing");
        assert!(tire.text.starts_with("# Tire Swing\n\nY Combinator company, Fall 2026. The AI Affordable Housing Platform"), "{}", tire.text);
        assert!(tire.text.contains("Tire Swing uses AI.\n\nIt serves compliance staff."), "{}", tire.text);
        assert!(tire.text.contains("Sector: B2B (B2B -> Legal)\nTags: GovTech, Legal\nTeam size: 2\nLocation: New York City, NY, USA\nStage: Early\nStatus: Active\nHiring: no"), "{}", tire.text);
        assert!(items[0].text.contains("Hiring: yes"));
        assert_eq!(tire.published_at.unwrap().timestamp(), 1740473580);
    }

    #[tokio::test]
    async fn known_companies_and_the_cap_apply_to_new_items_only() {
        let fx = Fixtures::start().await;
        fx.route("/yc/meta.json", 200, "application/json", META);
        fx.route("/yc/batches/fall-2026.json", 200, "application/json", FALL);
        fx.route("/yc/batches/summer-2026.json", 200, "application/json", "[]");
        let mut c = ctx(&fx, &["https://www.ycombinator.com/companies/hirey"]);
        let mut config = (*c.config).clone();
        config.limits.max_items = 1;
        c.config = std::sync::Arc::new(config);
        let items = collect(&c).await.unwrap();
        assert_eq!(items.iter().map(|i| i.title.as_str()).collect::<Vec<_>>(), ["Top Co"]);
    }

    #[tokio::test]
    async fn an_unreachable_directory_is_an_error() {
        let fx = Fixtures::start().await;
        fx.route("/yc/meta.json", 503, "text/plain", "down");
        assert!(collect(&ctx(&fx, &[])).await.unwrap_err().to_string().contains("HTTP 503"));
        fx.route("/yc/meta.json", 200, "application/json", META);
        // Both batches fail: nothing to show.
        let err = format!("{:#}", collect(&ctx(&fx, &[])).await.unwrap_err());
        assert!(err.contains("HTTP 404"), "{err}");
    }
}
