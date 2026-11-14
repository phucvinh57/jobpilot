//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.
//!
//! The screen reads everything it shows in one call ([`overview`]): the digest with its numbered
//! points and citations, the skill chart, the history, the collections waiting for the user, the
//! sources and which search keys are saved (never the keys themselves).

use anyhow::Result;
use jobpilot_core::chrono::Utc;
use jobpilot_core::graph::Outcome;
use jobpilot_core::prefs::Preferences;
use jobpilot_core::store::{Report, Source};
use jobpilot_core::trends::{
    Digest, DigestData, Period, Point, SearchProvider, Section, SourceKind, TrendChallenge,
    TrendRun,
};
use jobpilot_core::Core;
use serde::Serialize;

use super::rules::{
    challenge_name, challenge_phrase, describe, describe_source, header_summary, is_web_url,
    latest_month_is_partial, period_title, status_tag, status_text, summarize, target, when,
    Cadence, Field,
};
use crate::shared::{CommandError, CommandResult};

/// Skills drawn in the chart.
const CHART_ROWS: usize = 10;
/// Reports listed in the history.
const HISTORY_ROWS: usize = 30;

// ------------------------------------------------------------------ overview

/// What the Trends screen shows.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewDto {
    /// The current period of the chosen cadence: what "Collect & build digest" runs for.
    pub period: PeriodDto,
    /// The line under the page title.
    pub summary: String,
    /// The digest shown: the one asked for, else the newest.
    pub digest: Option<DigestDto>,
    /// The skill-frequency chart of that digest (`None` when it has no counts).
    pub chart: Option<ChartDto>,
    /// Earlier digests, newest first.
    pub history: Vec<ReportRowDto>,
    /// Collections paused at a login, captcha or block page.
    pub challenges: Vec<ChallengeDto>,
    pub sources: Vec<SourceDto>,
    /// The kinds of source that can be added.
    pub source_kinds: Vec<SourceKindDto>,
    pub search_keys: Vec<SearchKeyDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PeriodDto {
    /// `2026-W46` / `2026-Q4`: what the run commands take.
    pub label: String,
    /// `Week of 9 November` / `Q4 2026`.
    pub title: String,
}

impl PeriodDto {
    fn new(period: Period) -> Self {
        Self {
            label: period.label(),
            title: period_title(period),
        }
    }
}

/// Read everything the screen shows. `selected`: the report to show (the newest when `None` or gone).
pub fn overview(core: &Core, selected: Option<i64>, cadence: Cadence) -> Result<OverviewDto> {
    let current = cadence.period(Utc::now().date_naive());
    let mut shown = match selected {
        Some(id) => core.trend_digest(id)?,
        None => None,
    };
    if shown.is_none() {
        shown = core.latest_trend_digest()?;
    }
    let prefs = core.preferences()?;
    let sources = core.trend_sources()?;
    let saved_keys = core.saved_search_providers()?;
    Ok(OverviewDto {
        period: PeriodDto::new(current),
        summary: header_summary(shown.as_ref(), current),
        digest: shown.as_ref().map(|d| DigestDto::new(d, &prefs)),
        chart: shown.as_ref().and_then(|d| ChartDto::new(d, &prefs)),
        history: core
            .trend_reports()?
            .iter()
            .take(HISTORY_ROWS)
            .map(ReportRowDto::new)
            .collect(),
        challenges: core
            .trend_challenges()?
            .iter()
            .map(ChallengeDto::new)
            .collect(),
        source_kinds: SourceKind::ALL
            .into_iter()
            .map(|kind| SourceKindDto {
                kind: kind.as_str(),
                label: kind.label(),
                needs_url: kind.needs_url(),
                added: sources.iter().any(|s| s.kind == kind.as_str()),
            })
            .collect(),
        sources: sources.iter().map(SourceDto::new).collect(),
        search_keys: SearchProvider::ALL
            .into_iter()
            .map(|provider| SearchKeyDto {
                provider: provider_name(provider),
                label: provider.label(),
                saved: saved_keys.contains(&provider),
            })
            .collect(),
    })
}

/// How the web app spells a search provider.
pub fn provider_name(provider: SearchProvider) -> &'static str {
    match provider {
        SearchProvider::Brave => "brave",
        SearchProvider::Tavily => "tavily",
    }
}

pub fn parse_provider(name: &str) -> Option<SearchProvider> {
    SearchProvider::ALL
        .into_iter()
        .find(|p| provider_name(*p) == name)
}

// ------------------------------------------------------------------ digest

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DigestDto {
    pub report_id: i64,
    pub title: String,
    /// When it was made, in the user's time zone.
    pub when: String,
    /// Items it read (`None` for a report this version cannot read back).
    pub items_read: Option<u32>,
    pub body: DigestBodyDto,
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DigestBodyDto {
    /// Sections of numbered points, each tied to the stored items it rests on.
    Data {
        sections: Vec<SectionDto>,
        dropped: u32,
    },
    /// A report this version cannot read back: its text as saved (shown as plain lines).
    Plain { lines: Vec<String> },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDto {
    pub title: &'static str,
    pub points: Vec<PointDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PointDto {
    /// Position in the digest's points: the stable part of the element ids.
    pub index: usize,
    /// The badge: counts on through the sections in the order the points are shown.
    pub number: usize,
    pub topic: String,
    pub text: String,
    pub quote: String,
    pub citations: Vec<CitationDto>,
    /// A skill or domain the user can add to their preferences.
    pub suggest: Option<SuggestDto>,
}

/// A stored item a point cites.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CitationDto {
    /// The item's position among the digest's cited items (1-based): the number on the chip.
    pub number: usize,
    pub title: String,
    /// Only a web address (`None`: nothing to open).
    pub url: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SuggestDto {
    /// `skills` / `domains`.
    pub field: &'static str,
    /// `absent` / `added` / `avoided`: where the topic stands in the saved preferences.
    pub presence: &'static str,
}

impl DigestDto {
    fn new(digest: &Digest, prefs: &Preferences) -> Self {
        let body = match &digest.data {
            Some(data) => DigestBodyDto::Data {
                sections: sections(data, prefs),
                dropped: data.dropped,
            },
            None => DigestBodyDto::Plain {
                lines: digest.report.body_md.lines().map(str::to_string).collect(),
            },
        };
        Self {
            report_id: digest.report.id,
            title: digest.report.title.clone(),
            when: when(&digest.report),
            items_read: digest.data.as_ref().map(|d| d.items_read),
            body,
        }
    }
}

fn sections(data: &DigestData, prefs: &Preferences) -> Vec<SectionDto> {
    let mut number = 0;
    let mut out = Vec::new();
    for section in Section::ALL {
        let mut points = Vec::new();
        for (index, point) in data
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.section == section)
        {
            number += 1;
            points.push(point_dto(number, index, point, data, prefs));
        }
        if !points.is_empty() {
            out.push(SectionDto {
                title: section.title(),
                points,
            });
        }
    }
    out
}

fn point_dto(
    number: usize,
    index: usize,
    point: &Point,
    data: &DigestData,
    prefs: &Preferences,
) -> PointDto {
    let citations = point
        .citations
        .iter()
        .filter_map(|id| {
            let ordinal = data.items.iter().position(|i| i.id == *id)?;
            let item = data.items.get(ordinal)?;
            Some(CitationDto {
                number: ordinal + 1,
                title: item.title.clone(),
                url: is_web_url(&item.url).then(|| item.url.clone()),
            })
        })
        .collect();
    let suggest = (point.section.is_suggestible() && !point.topic.is_empty()).then(|| {
        let field = target(point.section);
        SuggestDto {
            field: field.as_str(),
            presence: field.presence(prefs, &point.topic).as_str(),
        }
    });
    PointDto {
        index,
        number,
        topic: point.topic.clone(),
        text: point.text.clone(),
        quote: point.quote.clone(),
        citations,
        suggest,
    }
}

// ------------------------------------------------------------------ chart

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDto {
    pub skills: Vec<ChartSkillDto>,
    /// The count the bars are scaled to (the most frequent skill's).
    pub max: u32,
    /// Documents (postings and leads) read over the window.
    pub documents: u32,
    pub from_month: String,
    pub to_month: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSkillDto {
    pub name: String,
    pub total: u32,
    /// The count, the share and its change, in words (the tooltip of the row).
    pub description: String,
    /// `absent` / `added` / `avoided`: where the skill stands in the saved preferences.
    pub presence: &'static str,
}

impl ChartDto {
    fn new(digest: &Digest, prefs: &Preferences) -> Option<Self> {
        let counts = &digest.data.as_ref()?.counts;
        let max = counts.skills.first()?.total.max(1);
        let documents: u32 = counts.documents.iter().sum();
        let partial = latest_month_is_partial(
            &counts.months,
            &digest.report.created_at.format("%Y-%m").to_string(),
        );
        let skills = counts
            .skills
            .iter()
            .take(CHART_ROWS)
            .map(|skill| ChartSkillDto {
                name: skill.skill.clone(),
                total: skill.total,
                description: describe(skill, &counts.documents, partial),
                // Any skill of the chart, rising or not: adding it makes no claim about the trend.
                presence: Field::Skills.presence(prefs, &skill.skill).as_str(),
            })
            .collect();
        Some(Self {
            skills,
            max,
            documents,
            from_month: counts.months.first().cloned().unwrap_or_default(),
            to_month: counts.months.last().cloned().unwrap_or_default(),
        })
    }
}

// ------------------------------------------------------------------ history, waiting runs

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReportRowDto {
    pub id: i64,
    pub title: String,
    pub when: String,
}

impl ReportRowDto {
    fn new(report: &Report) -> Self {
        Self {
            id: report.id,
            title: report.title.clone(),
            when: when(report),
        }
    }
}

/// A collection paused at a login, captcha or block page.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChallengeDto {
    /// The period it collects for: what "Continue" resumes.
    pub period: String,
    pub source_name: String,
    /// `login` / `captcha` / `blocked`.
    pub kind: &'static str,
    /// `needs you to sign in` / `shows a captcha` / `blocks automated access`.
    pub what: &'static str,
    /// The page to open (only a web address).
    pub url: Option<String>,
}

impl ChallengeDto {
    fn new(wait: &TrendChallenge) -> Self {
        Self {
            period: wait.period.label(),
            source_name: wait.source_name.clone(),
            kind: challenge_name(wait.kind),
            what: challenge_phrase(wait.kind),
            url: is_web_url(&wait.url).then(|| wait.url.clone()),
        }
    }
}

// ------------------------------------------------------------------ sources

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    /// The kind and the host of its address (never the whole address).
    pub description: String,
    /// What its last run says, in words.
    pub status_text: String,
    pub tag: TagDto,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub label: &'static str,
    /// The web kit's `Tone`.
    pub tone: &'static str,
}

impl SourceDto {
    fn new(source: &Source) -> Self {
        let (label, tone) = status_tag(source);
        Self {
            id: source.id,
            name: source.name.clone(),
            enabled: source.enabled,
            description: describe_source(source),
            status_text: status_text(source),
            tag: TagDto {
                label,
                tone: tone.as_str(),
            },
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceKindDto {
    /// `hackernews` / `yc` / `rss` / `page`.
    pub kind: &'static str,
    pub label: &'static str,
    /// A feed or page needs an address; Hacker News and YC do not.
    pub needs_url: bool,
    /// Already a source (Hacker News and YC can be added once).
    pub added: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchKeyDto {
    /// `brave` / `tavily`.
    pub provider: &'static str,
    pub label: &'static str,
    /// A key is in the OS keyring. The key itself never leaves it.
    pub saved: bool,
}

// ------------------------------------------------------------------ runs and edits

/// How a run that did not fail ended.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDto {
    /// `done` / `waiting` (a source needs the user) / `cancelled`.
    pub status: &'static str,
    /// The report the run saved.
    pub report_id: Option<i64>,
    pub headline: String,
    /// One line per source, and per thing that went less well.
    pub details: Vec<String>,
}

/// The result of a run. A failed run is an error carrying its `Problem`, so the screen offers the
/// same call to action (Settings shortcut, Retry) as everywhere else.
pub fn run_result(outcome: Outcome<TrendRun>) -> CommandResult<RunDto> {
    match outcome {
        Outcome::Done(run) => {
            let (headline, details) = summarize(&run);
            Ok(RunDto {
                status: "done",
                report_id: run.digest.report_id,
                headline,
                details,
            })
        }
        Outcome::Waiting(_) => Ok(RunDto {
            status: "waiting",
            report_id: None,
            headline: "A source needs you before the digest can be built: see the banner below."
                .into(),
            details: Vec::new(),
        }),
        Outcome::Failed(failure) => {
            Err(CommandError::with_problem(failure.problem, failure.message))
        }
        Outcome::Cancelled => Ok(RunDto {
            status: "cancelled",
            report_id: None,
            headline: "Cancelled.".into(),
            details: Vec::new(),
        }),
    }
}

/// What "Add to preferences" did.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddedDto {
    /// `added` / `avoided` / `absent`: where the topic stands now.
    pub presence: &'static str,
    /// Why it was not added.
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use jobpilot_core::graph::{Failure, Outcome, WaitReason};
    use jobpilot_core::health::Problem;
    use jobpilot_core::trends::{
        CitedItem, Digest, DigestData, Period, Point, Section, SkillCount, SkillCounts, TrendRun,
    };
    use jobpilot_core::{Core, Paths, Secrets};

    use super::{
        overview, parse_provider, provider_name, run_result, sections, ChartDto, DigestBodyDto,
    };
    use crate::trends::demo;
    use crate::trends::rules::Cadence;
    use jobpilot_core::prefs::Preferences;

    fn item(id: i64, url: &str) -> CitedItem {
        CitedItem {
            id,
            title: format!("item {id}"),
            url: url.into(),
        }
    }

    fn point(section: Section, topic: &str, citations: &[i64]) -> Point {
        Point {
            section,
            topic: topic.into(),
            text: "text".into(),
            quote: String::new(),
            citations: citations.to_vec(),
        }
    }

    #[test]
    fn points_are_numbered_through_the_sections_in_display_order() {
        let data = DigestData {
            items: vec![item(7, "https://a.test/1"), item(9, "file:///etc/passwd")],
            // Stored out of display order: the badges still count on from the first section.
            points: vec![
                point(Section::Hiring, "", &[7]),
                point(Section::RisingSkills, "Rust", &[9, 7, 404]),
                point(Section::Domains, "fintech", &[7]),
                point(Section::RisingSkills, "Go", &[7]),
            ],
            ..DigestData::default()
        };
        let prefs = Preferences {
            must_skills: vec!["rust".into()],
            avoid_domains: vec!["Fintech".into()],
            ..Preferences::default()
        };
        let out = sections(&data, &prefs);
        let titles: Vec<_> = out.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            ["Rising skills", "Active domains", "Hiring signals"]
        );
        let numbers: Vec<_> = out
            .iter()
            .flat_map(|s| s.points.iter().map(|p| (p.number, p.index)))
            .collect();
        assert_eq!(numbers, [(1, 1), (2, 3), (3, 2), (4, 0)]);

        let rust = &out[0].points[0];
        // The unknown id 404 is dropped; the numbers are the items' positions in the digest.
        assert_eq!(
            rust.citations.iter().map(|c| c.number).collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(
            rust.citations[0].url, None,
            "a file address is never offered"
        );
        assert_eq!(rust.citations[1].url.as_deref(), Some("https://a.test/1"));
        let suggest = rust.suggest.as_ref().map(|s| (s.field, s.presence));
        assert_eq!(suggest, Some(("skills", "added")));
        let fintech = out[1].points[0]
            .suggest
            .as_ref()
            .map(|s| (s.field, s.presence));
        assert_eq!(fintech, Some(("domains", "avoided")));
        assert!(
            out[2].points[0].suggest.is_none(),
            "a hiring signal names nothing to add"
        );
    }

    #[test]
    fn a_chart_scales_to_the_top_skill_and_says_where_each_stands() {
        let skill = |name: &str, by_month: &[u32]| SkillCount {
            skill: name.into(),
            by_month: by_month.to_vec(),
            total: by_month.iter().sum(),
        };
        let counts = SkillCounts {
            months: vec!["2026-10".into(), "2026-11".into()],
            documents: vec![100, 100],
            skills: vec![skill("Rust", &[10, 20]), skill("Go", &[5, 5])],
        };
        let data = DigestData {
            counts,
            ..DigestData::default()
        };
        let digest = Digest {
            report: jobpilot_core::store::Report {
                id: 1,
                kind: "weekly".into(),
                period: "2026-W46".into(),
                title: "t".into(),
                body_md: String::new(),
                citations_json: String::new(),
                created_at: jobpilot_core::chrono::Utc::now(),
            },
            data: Some(data),
        };
        let prefs = Preferences {
            nice_skills: vec!["Go".into()],
            ..Preferences::default()
        };
        let chart = ChartDto::new(&digest, &prefs).expect("a chart");
        assert_eq!((chart.max, chart.documents), (30, 200));
        assert_eq!(
            (chart.from_month.as_str(), chart.to_month.as_str()),
            ("2026-10", "2026-11")
        );
        assert_eq!(
            chart
                .skills
                .iter()
                .map(|s| (s.name.as_str(), s.presence))
                .collect::<Vec<_>>(),
            [("Rust", "absent"), ("Go", "added")]
        );

        let empty = Digest {
            data: Some(DigestData::default()),
            ..digest
        };
        assert!(
            ChartDto::new(&empty, &prefs).is_none(),
            "no skills, no chart"
        );
    }

    #[test]
    fn a_failed_run_keeps_its_problem_so_the_screen_can_offer_the_fix() {
        let failed = run_result(Outcome::Failed(Failure {
            problem: Problem::Network,
            node: "yc".into(),
            message: "timeout".into(),
        }));
        let error = failed.expect_err("a failed run is an error");
        assert_eq!(
            (
                error.problem.kind,
                error.problem.retryable,
                error.message.as_str()
            ),
            ("network", true, "timeout")
        );

        let no_provider = run_result(Outcome::Failed(Failure {
            problem: Problem::NoProvider,
            node: String::new(),
            message: "x".into(),
        }));
        assert_eq!(
            no_provider.expect_err("error").problem.settings_section,
            Some("AI providers")
        );

        let cancelled = run_result(Outcome::Cancelled).expect("cancelled is not an error");
        assert_eq!(
            (cancelled.status, cancelled.headline.as_str()),
            ("cancelled", "Cancelled.")
        );

        let waiting =
            run_result(Outcome::Waiting(WaitReason::Unknown)).expect("waiting is not an error");
        assert_eq!(waiting.status, "waiting");
        assert!(waiting.headline.contains("see the banner below"));

        let mut run = TrendRun::new(Period::parse("2026-W46").expect("a week"));
        run.digest.report_id = Some(5);
        let done = run_result(Outcome::Done(run)).expect("done");
        assert_eq!((done.status, done.report_id), ("done", Some(5)));
    }

    #[test]
    fn search_providers_round_trip() {
        for name in ["brave", "tavily"] {
            assert_eq!(parse_provider(name).map(provider_name), Some(name));
        }
        assert!(parse_provider("bing").is_none());
    }

    #[test]
    fn the_demo_data_reads_back_as_one_digest_with_citations() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let core =
            Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).expect("a core");
        let empty = overview(&core, None, Cadence::Weekly).expect("an empty overview");
        assert!(empty.digest.is_none() && empty.chart.is_none() && empty.history.is_empty());
        assert!(empty.summary.starts_with("No digest yet"));
        assert_eq!(empty.source_kinds.len(), 4);
        assert!(empty.source_kinds.iter().all(|k| !k.added));
        assert_eq!(
            empty
                .search_keys
                .iter()
                .map(|k| k.saved)
                .collect::<Vec<_>>(),
            [false, false]
        );

        demo::seed_demo(&core).expect("the demo data");
        let seen = overview(&core, None, Cadence::Weekly).expect("an overview");
        assert_eq!(seen.sources.len(), 4);
        assert!(seen
            .source_kinds
            .iter()
            .any(|k| k.kind == "hackernews" && k.added));
        assert!(
            seen.source_kinds.iter().any(|k| k.kind == "rss" && k.added),
            "a feed can be added again"
        );
        assert_eq!(seen.history.len(), 1);
        let digest = seen.digest.expect("a digest");
        let DigestBodyDto::Data { sections, dropped } = digest.body else {
            panic!("the demo digest has data")
        };
        assert_eq!(dropped, 1);
        assert_eq!(sections.len(), 6, "every section has a point");
        assert!(
            sections
                .iter()
                .flat_map(|s| &s.points)
                .all(|p| !p.citations.is_empty()),
            "every point cites an item"
        );
        assert_eq!(seen.chart.map(|c| c.skills.len()), Some(10));

        // A report that is gone falls back to the newest.
        let missing = overview(&core, Some(9999), Cadence::Quarterly).expect("an overview");
        assert_eq!(missing.digest.map(|d| d.report_id), Some(digest.report_id));
        assert!(missing.period.label.contains("-Q"));
    }
}
