//! Sample data for the Trends screen (`JOBPILOT_DEMO=1`): a few sources, items and one digest with
//! citations, market numbers for a global and a local scope, the gap and the wishes. Every address
//! is on `example.com`; nothing is fetched.

use jobpilot_engine::store::NewTrendItem;
use jobpilot_engine::trends::digest::{
    centi, log_odds_z, month_window, permille, trend_z, wilson, Coverage, Direction, GapBucket,
    GapScope, GapSkill, ScopeCounts, Stratum, TermCount, WishKind, WishSignal,
    MARKET_SCAN_VERSION,
};
use jobpilot_engine::trends::{
    CitedItem, DigestData, Period, Point, Section, SkillCount, SkillCounts, SourceKind,
};
use jobpilot_engine::Core;
use anyhow::Result;
use chrono::{Datelike, Duration, Utc};

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
        version: MARKET_SCAN_VERSION,
        months: month_window(&period, 6),
        target_months: TARGET_MONTHS as u32,
        documents: GLOBAL_DOCUMENTS.to_vec(),
        skills: skill_list(&GLOBAL, &GLOBAL_DOCUMENTS),
        local: Some(ScopeCounts {
            documents: LOCAL_DOCUMENTS.to_vec(),
            skills: skill_list(&LOCAL, &LOCAL_DOCUMENTS),
        }),
        place: "Ho Chi Minh City, Singapore or remote".into(),
        field: vec!["backend".into(), "engineer".into()],
        pipeline: Some(ScopeCounts {
            documents: PIPELINE_DOCUMENTS.to_vec(),
            skills: skill_list(
                &[
                    ("Rust", [0, 0, 2, 4, 6, 6], Direction::Unclear),
                    ("Go", [0, 0, 1, 3, 4, 3], Direction::Unclear),
                    ("Kubernetes", [0, 0, 1, 2, 3, 3], Direction::Unclear),
                ],
                &PIPELINE_DOCUMENTS,
            ),
        }),
        gap_scope: GapScope::Local,
        gap: vec![
            gap(GapBucket::BuildOn, "Python", Direction::Steady, 209, 205),
            gap(
                GapBucket::BuildOn,
                "PostgreSQL",
                Direction::Steady,
                121,
                118,
            ),
            gap(GapBucket::BuildOn, "Rust", Direction::Rising, 94, 130),
            gap(GapBucket::LearnNext, "AWS", Direction::Steady, 150, 148),
            gap(
                GapBucket::LearnNext,
                "Kubernetes",
                Direction::Rising,
                133,
                190,
            ),
            gap(GapBucket::LearnNext, "LLM", Direction::Rising, 86, 130),
            gap(GapBucket::AtRisk, "PHP", Direction::Falling, 49, 25),
        ],
        uncatalogued: vec![
            term("Temporal", 11, Stratum::HiringPosting, 312),
            term("dbt", 9, Stratum::HiringPosting, 0),
            term("vector databases", 14, Stratum::HnStory, 0),
            term("agent harness", 6, Stratum::Feed, 0),
        ],
        coverage: Coverage {
            postings_counted: GLOBAL_DOCUMENTS.iter().sum(),
            postings_unreadable: 0,
            items_counted: u32::try_from(cited.len()).unwrap_or(0),
            left_unread_at_source: Some(0),
            months_without_postings: Vec::new(),
            not_comparable: vec![Stratum::Yc],
        },
        wishes: vec![
            wish("Rust", WishKind::Skill, false, Direction::Rising, 53),
            wish("Kubernetes", WishKind::Skill, false, Direction::Rising, 75),
            wish("PHP", WishKind::Skill, true, Direction::Falling, 33),
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
    // One report per cadence, so the Reports list has a row in each; the week is saved last and
    // is the one shown first. The month's report is the one whose counts did not see everything:
    // it shows the coverage as a notice.
    let today = now.date_naive();
    let last_year = today.with_year(today.year() - 1).unwrap_or(today);
    let partial = partial(&data);
    for other in [
        Period::year_of(last_year),
        Period::quarter_of(today),
        Period::week_of(today - Duration::days(7)),
    ] {
        core.save_trend_digest(&other, &data)?;
    }
    core.save_trend_digest(&Period::month_of(today), &partial)?;
    core.save_trend_digest(&period, &data)?;
    Ok(())
}

/// The same report from counts that did not see everything: postings whose text could not be
/// read, and items a source left unread in the run.
fn partial(data: &DigestData) -> DigestData {
    let mut data = data.clone();
    data.counts.coverage.postings_unreadable = 14;
    data.counts.coverage.left_unread_at_source = Some(120);
    data
}

/// A skill of the demo: postings naming it per month, and its direction (set by hand; the test
/// below checks that the scores computed from the counts agree with it).
type DemoSkill = (&'static str, [u32; 6], Direction);

/// How many of the demo's six months are the period's own (the last three); the three before are
/// what they are compared with.
const TARGET_MONTHS: usize = 3;
/// Postings of the field in the target months of the demo's local scope, for the gap's intervals.
const FIELD_TARGET_DOCUMENTS: u32 = 210;
/// The prior of the step test, as the digest uses it.
const STEP_PRIOR: f64 = 1.0;

/// Postings read per month in each scope.
const GLOBAL_DOCUMENTS: [u32; 6] = [340, 355, 330, 362, 371, 348];
const LOCAL_DOCUMENTS: [u32; 6] = [92, 98, 88, 104, 110, 101];
const PIPELINE_DOCUMENTS: [u32; 6] = [0, 0, 3, 6, 9, 8];

/// Skills the demo profile names (`in_profile`) and the demo preferences wish for (`wished`).
const PROFILE: [&str; 5] = ["Rust", "Python", "PostgreSQL", "Go", "PHP"];
const WISHED: [&str; 3] = ["Rust", "Kubernetes", "PHP"];

/// Every hiring posting of the window.
const GLOBAL: [DemoSkill; 10] = [
    ("Python", [70, 72, 68, 74, 73, 70], Direction::Steady),
    ("TypeScript", [62, 64, 58, 63, 62, 58], Direction::Steady),
    ("AWS", [55, 56, 52, 55, 53, 50], Direction::Steady),
    ("PostgreSQL", [40, 42, 38, 43, 44, 41], Direction::Steady),
    ("Kubernetes", [32, 34, 36, 44, 52, 57], Direction::Rising),
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
    ("Kubernetes", [6, 8, 9, 14, 18, 20], Direction::Rising),
    ("PostgreSQL", [12, 12, 11, 13, 12, 12], Direction::Steady),
    ("Go", [9, 10, 9, 10, 10, 9], Direction::Steady),
    ("Rust", [4, 5, 5, 10, 13, 16], Direction::Rising),
    ("LLM", [3, 5, 7, 10, 12, 14], Direction::Rising),
    ("PHP", [10, 8, 6, 4, 3, 2], Direction::Falling),
    ("Kafka", [4, 5, 4, 5, 5, 4], Direction::Steady),
];

/// Most named first, ties by name, with the profile and wish marks and the numbers the digest's
/// tests make of `documents` (read per month): the target months' share, its interval, the scores.
fn skill_list(skills: &[DemoSkill], documents: &[u32]) -> Vec<SkillCount> {
    let at = documents.len().saturating_sub(TARGET_MONTHS);
    let (documents_before, documents_now) = documents.split_at(at);
    let (documents_before, documents_now) = (
        documents_before.iter().sum::<u32>(),
        documents_now.iter().sum::<u32>(),
    );
    let mut list: Vec<SkillCount> = skills
        .iter()
        .map(|(skill, by_month, direction)| {
            let (before, now) = by_month.split_at(at.min(by_month.len()));
            let (before, now) = (before.iter().sum::<u32>(), now.iter().sum::<u32>());
            let (low_permille, high_permille) = wilson(now, documents_now);
            SkillCount {
                skill: (*skill).into(),
                total: by_month.iter().sum(),
                by_month: by_month.to_vec(),
                direction: *direction,
                target: now,
                target_permille: permille(u64::from(now), u64::from(documents_now)),
                low_permille,
                high_permille,
                z_centi: centi(log_odds_z(
                    now,
                    documents_now,
                    before,
                    documents_before,
                    STEP_PRIOR,
                )),
                trend_z_centi: centi(trend_z(by_month, documents)),
                in_profile: PROFILE.contains(skill),
                wished: WISHED.contains(skill),
            }
        })
        .collect();
    list.sort_by(|a, b| b.total.cmp(&a.total).then_with(|| a.skill.cmp(&b.skill)));
    list
}

/// A gap skill: its share of the field's postings over the window, and in the target months (with
/// the interval of that share on [`FIELD_TARGET_DOCUMENTS`] postings).
fn gap(
    bucket: GapBucket,
    skill: &str,
    direction: Direction,
    share_permille: u32,
    target_permille: u32,
) -> GapSkill {
    let named = (target_permille * FIELD_TARGET_DOCUMENTS + 500) / 1000;
    let (low_permille, high_permille) = wilson(named, FIELD_TARGET_DOCUMENTS);
    GapSkill {
        skill: skill.into(),
        bucket,
        direction,
        share_permille,
        target_permille,
        low_permille,
        high_permille,
    }
}

/// A term outside the skill list; a `z_centi` above zero marks one that rose against its baseline.
fn term(term: &str, items: u32, stratum: Stratum, z_centi: i32) -> TermCount {
    TermCount {
        term: term.into(),
        items,
        stratum,
        z_centi,
        rising: z_centi > 0,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sample_directions_agree_with_the_numbers_beside_them() {
        let scopes = [
            skill_list(&GLOBAL, &GLOBAL_DOCUMENTS),
            skill_list(&LOCAL, &LOCAL_DOCUMENTS),
        ];
        for skill in scopes.iter().flatten() {
            // 196 is the score of a 5% test; the digest asks for more where it tests many skills.
            let (step, trend) = (skill.z_centi, skill.trend_z_centi);
            match skill.direction {
                Direction::Rising => assert!(step > 196 && trend > 196, "{skill:?}"),
                Direction::Falling => assert!(step < -196 && trend < -196, "{skill:?}"),
                Direction::Steady => assert!(step.abs() < 196 && trend.abs() < 196, "{skill:?}"),
                Direction::Unclear => {}
            }
            assert!(
                skill.low_permille <= skill.target_permille
                    && skill.target_permille <= skill.high_permille,
                "the share is inside its interval: {skill:?}"
            );
        }
    }
}
