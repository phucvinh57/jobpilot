//! Sample data for the Trends screen (`JOBPILOT_DEMO=1`): a few sources, items and one digest with
//! citations, market numbers for a global and a local scope, the gap and the wishes. Every address
//! is on `example.com`; nothing is fetched.

use crate::engine::store::NewTrendItem;
use crate::engine::trends::digest::{
    month_window, Direction, GapBucket, GapScope, GapSkill, ScopeCounts, TermCount, WishKind,
    WishSignal,
};
use crate::engine::trends::{
    CitedItem, DigestData, Period, Point, Section, SkillCount, SkillCounts, SourceKind,
};
use crate::engine::Core;
use anyhow::Result;
use chrono::{Duration, Utc};

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
            "Kubernetes",
            "Kubernetes is the skill platform roles ask for most next to Rust and Go, and your profile does not name it yet.",
            "",
            &[1],
        ),
        point(
            Section::WishesCheck,
            "fintech",
            "Fintech, a domain you want, is hiring backend engineers even while its funding slows.",
            "",
            &[5],
        ),
    ];
    let counts = SkillCounts {
        months: month_window(&period, 6),
        documents: vec![340, 355, 330, 362, 371, 348],
        skills: skill_list(&GLOBAL),
        local: Some(ScopeCounts {
            documents: vec![92, 98, 88, 104, 110, 101],
            skills: skill_list(&LOCAL),
        }),
        place: "Ho Chi Minh City, Singapore or remote".into(),
        field: vec!["backend".into(), "engineer".into()],
        pipeline: Some(ScopeCounts {
            documents: vec![0, 0, 3, 6, 9, 8],
            skills: skill_list(&[
                ("Rust", [0, 0, 2, 4, 6, 6], Direction::Unclear),
                ("Go", [0, 0, 1, 3, 4, 3], Direction::Unclear),
                ("Kubernetes", [0, 0, 1, 2, 3, 3], Direction::Unclear),
            ]),
        }),
        gap_scope: GapScope::Local,
        gap: vec![
            gap(GapBucket::BuildOn, "Python", Direction::Steady, 209),
            gap(GapBucket::BuildOn, "PostgreSQL", Direction::Steady, 121),
            gap(GapBucket::BuildOn, "Rust", Direction::Rising, 94),
            gap(GapBucket::LearnNext, "AWS", Direction::Steady, 150),
            gap(GapBucket::LearnNext, "Kubernetes", Direction::Rising, 133),
            gap(GapBucket::LearnNext, "LLM", Direction::Rising, 86),
            gap(GapBucket::AtRisk, "PHP", Direction::Falling, 49),
        ],
        uncatalogued: vec![
            term("Temporal", 5),
            term("dbt", 4),
            term("vector databases", 3),
        ],
        wishes: vec![
            wish("Rust", WishKind::Skill, false, Direction::Rising, 56),
            wish("Kubernetes", WishKind::Skill, false, Direction::Rising, 79),
            wish("PHP", WishKind::Skill, true, Direction::Falling, 29),
            wish("fintech", WishKind::Domain, false, Direction::Steady, 18),
            wish(
                "climate tech",
                WishKind::Domain,
                false,
                Direction::Unclear,
                0,
            ),
        ],
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

/// A skill of the demo: postings naming it per month, and its direction.
type DemoSkill = (&'static str, [u32; 6], Direction);

/// Skills the demo profile names (`in_profile`) and the demo preferences wish for (`wished`).
const PROFILE: [&str; 5] = ["Rust", "Python", "PostgreSQL", "Go", "PHP"];
const WISHED: [&str; 3] = ["Rust", "Kubernetes", "PHP"];

/// Every hiring posting of the window.
const GLOBAL: [DemoSkill; 10] = [
    ("Python", [70, 72, 68, 74, 73, 70], Direction::Steady),
    ("TypeScript", [62, 64, 58, 63, 62, 58], Direction::Steady),
    ("AWS", [55, 56, 52, 55, 53, 50], Direction::Steady),
    ("PostgreSQL", [40, 42, 38, 43, 44, 41], Direction::Steady),
    ("Kubernetes", [38, 41, 40, 45, 48, 47], Direction::Rising),
    ("Rust", [18, 21, 22, 28, 33, 36], Direction::Rising),
    ("Go", [30, 31, 29, 32, 31, 30], Direction::Steady),
    ("LLM", [12, 18, 24, 33, 41, 49], Direction::Rising),
    ("Kafka", [14, 15, 13, 15, 16, 14], Direction::Steady),
    ("PHP", [22, 20, 17, 15, 12, 10], Direction::Falling),
];

/// The postings of the demo place: fewer, so the shares move more.
const LOCAL: [DemoSkill; 10] = [
    ("Python", [20, 21, 19, 22, 22, 20], Direction::Steady),
    ("TypeScript", [16, 17, 15, 18, 17, 16], Direction::Steady),
    ("AWS", [15, 15, 14, 16, 15, 14], Direction::Steady),
    ("Kubernetes", [10, 12, 12, 14, 16, 15], Direction::Rising),
    ("PostgreSQL", [12, 12, 11, 13, 12, 12], Direction::Steady),
    ("Go", [9, 10, 9, 10, 10, 9], Direction::Steady),
    ("Rust", [6, 7, 8, 10, 12, 13], Direction::Rising),
    ("LLM", [3, 5, 7, 10, 12, 14], Direction::Rising),
    ("PHP", [7, 6, 5, 4, 4, 3], Direction::Falling),
    ("Kafka", [4, 5, 4, 5, 5, 4], Direction::Unclear),
];

/// Most named first, ties by name, with the profile and wish marks.
fn skill_list(skills: &[DemoSkill]) -> Vec<SkillCount> {
    let mut list: Vec<SkillCount> = skills
        .iter()
        .map(|(skill, by_month, direction)| SkillCount {
            skill: (*skill).into(),
            total: by_month.iter().sum(),
            by_month: by_month.to_vec(),
            direction: *direction,
            in_profile: PROFILE.contains(skill),
            wished: WISHED.contains(skill),
        })
        .collect();
    list.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.skill.cmp(&b.skill)));
    list
}

fn gap(bucket: GapBucket, skill: &str, direction: Direction, share_permille: u32) -> GapSkill {
    GapSkill {
        skill: skill.into(),
        bucket,
        direction,
        share_permille,
    }
}

fn term(term: &str, items: u32) -> TermCount {
    TermCount {
        term: term.into(),
        items,
    }
}

fn wish(
    wish: &str,
    kind: WishKind,
    avoided: bool,
    direction: Direction,
    mentions: u32,
) -> WishSignal {
    WishSignal {
        wish: wish.into(),
        kind,
        avoided,
        direction,
        mentions,
    }
}
