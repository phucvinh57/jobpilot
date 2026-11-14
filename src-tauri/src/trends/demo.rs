//! Sample data for the Trends screen (`JOBPILOT_DEMO=1`): a few sources, items and one digest with
//! citations. Every address is on `example.com`; nothing is fetched.

use anyhow::Result;
use jobpilot_core::chrono::{Duration, Utc};
use jobpilot_core::store::NewTrendItem;
use jobpilot_core::trends::digest::month_window;
use jobpilot_core::trends::{
    CitedItem, DigestData, Period, Point, Section, SkillCount, SkillCounts, SourceKind,
};
use jobpilot_core::Core;

/// Insert the sample sources, items and digest when there are none yet.
pub fn seed_demo(core: &Core) -> Result<()> {
    if !core.trend_sources()?.is_empty() || !core.trend_reports()?.is_empty() {
        return Ok(());
    }
    let hn = core.add_trend_source(SourceKind::HackerNews, "", "")?;
    let yc = core.add_trend_source(SourceKind::Yc, "", "")?;
    let blog = core.add_trend_source(
        SourceKind::Rss,
        "Example engineering blog",
        "https://blog.example.com/feed.xml",
    )?;
    core.add_trend_source(
        SourceKind::Page,
        "Example annual report",
        "https://reports.example.com/annual-2026.html",
    )?;

    let now = Utc::now();
    let samples = [
        (
            Some(hn.id),
            "https://news.example.com/item/1001",
            "Acme | Senior Rust Engineer | REMOTE | robotics platform",
            1,
        ),
        (
            Some(hn.id),
            "https://news.example.com/item/1002",
            "Globex | Platform Engineer | Berlin | Rust, Go, Kubernetes",
            2,
        ),
        (
            Some(hn.id),
            "https://news.example.com/postgres-19",
            "Postgres 19 is out",
            3,
        ),
        (
            Some(yc.id),
            "https://yc.example.com/companies/hirey",
            "Hirey (YC F26): hiring tools for startups",
            2,
        ),
        (
            Some(blog.id),
            "https://blog.example.com/rust-latency",
            "How we cut tail latency in half with Rust",
            4,
        ),
        (
            None,
            "https://reports.example.com/fintech-2026.pdf",
            "Fintech outlook 2026",
            5,
        ),
    ];
    let mut ids = Vec::new();
    for (source_id, url, title, age_days) in samples {
        let item = core.store.add_trend_item(&NewTrendItem {
            source_id,
            url: url.into(),
            title: title.into(),
            published_at: Some(now - Duration::days(age_days)),
            text_path: None,
        })?;
        ids.push(item.map(|i| (i.id, i.title, i.url)));
    }
    let cited: Vec<CitedItem> = ids
        .into_iter()
        .flatten()
        .map(|(id, title, url)| CitedItem { id, title, url })
        .collect();
    let id = |n: usize| cited.get(n).map_or(0, |i| i.id);

    let period = Period::current_week();
    let point = |section, topic: &str, text: &str, quote: &str, citations: &[usize]| Point {
        section,
        topic: topic.into(),
        text: text.into(),
        quote: quote.into(),
        citations: citations.iter().map(|n| id(*n)).collect(),
    };
    let points = vec![
        point(
            Section::RisingSkills,
            "Rust",
            "Rust shows up in more postings and in engineering write-ups about latency.",
            "cut tail latency in half with Rust",
            &[0, 1, 4],
        ),
        point(Section::RisingSkills, "Kubernetes", "Platform roles keep asking for Kubernetes next to Rust and Go.", "", &[1]),
        point(Section::FallingSkills, "PHP", "Fewer postings than a few months ago name PHP.", "", &[1]),
        point(Section::Domains, "fintech", "Fintech companies are hiring backend engineers while funding slows.", "", &[5]),
        point(Section::Domains, "developer tools", "A new YC batch company builds hiring tools for startups.", "hiring tools for startups", &[3]),
        point(Section::Funding, "", "The fintech outlook expects fewer but larger rounds this year.", "", &[5]),
        point(Section::Hiring, "", "Remote Rust roles are posted in the monthly hiring thread.", "Senior Rust Engineer", &[0]),
        point(
            Section::ForYou,
            "",
            "Your interest in backend and Rust matches the postings above; Kubernetes is the skill most often asked for next to it.",
            "",
            &[0, 1],
        ),
    ];
    let skills = [
        ("Rust", [4, 5, 7, 9, 12, 15]),
        ("Python", [20, 19, 21, 18, 20, 19]),
        ("Kubernetes", [8, 9, 9, 11, 12, 12]),
        ("Go", [6, 7, 6, 8, 8, 9]),
        ("PostgreSQL", [9, 8, 9, 8, 9, 10]),
        ("TypeScript", [14, 15, 13, 14, 12, 13]),
        ("AWS", [12, 12, 11, 12, 10, 11]),
        ("Kafka", [3, 4, 4, 5, 5, 6]),
        ("PHP", [7, 6, 5, 4, 3, 2]),
        ("LLM", [1, 2, 4, 6, 9, 13]),
    ];
    let counts = SkillCounts {
        months: month_window(&period, 6),
        documents: vec![60, 64, 62, 68, 70, 74],
        skills: {
            let mut list: Vec<SkillCount> = skills
                .into_iter()
                .map(|(skill, by_month)| SkillCount {
                    skill: skill.into(),
                    total: by_month.iter().sum(),
                    by_month: by_month.to_vec(),
                })
                .collect();
            list.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.skill.cmp(&b.skill)));
            list
        },
    };
    let data = DigestData {
        items: cited,
        points,
        counts,
        items_read: 6,
        dropped: 1,
    };
    core.save_trend_digest(&period, &data)?;
    Ok(())
}
