//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.
//!
//! The screen reads everything it shows in one call ([`overview`]): the digest with its numbered
//! points and citations (and, next to them, the gap and the wishes as the app measured them), the
//! skill chart per market scope (global, and local to the user's place), the history, the
//! collections waiting for the user, the sources and which search keys are saved (never the keys
//! themselves). Every number and label of the market is worked out here, in code.

use crate::engine::graph::Outcome;
use crate::engine::prefs::Preferences;
use crate::engine::store::{Report, Source};
use crate::engine::trends::digest::{GapScope, ScopeCounts, SkillCounts};
use crate::engine::trends::{
    Digest, DigestData, Period, Point, SearchProvider, Section, SourceKind, TrendChallenge,
    TrendRun,
};
use crate::engine::Core;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;

use super::rules::{
    bucket_key, bucket_words, challenge_name, challenge_phrase, describe, describe_source,
    direction_tag, header_summary, is_web_url, judged_on, knows_background,
    latest_month_is_partial, month_label, percent, percent_text, period_title, permille,
    pipeline_note, scope_key, scope_note, section_title, status_tag, status_text, summarize,
    target, when, wish_kind, wish_signal, Cadence, Field, Tone, BUCKETS,
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
        /// Frequent extracted topics the skill list does not know (third-party words: text only).
        uncatalogued: Vec<TermDto>,
    },
    /// A report this version cannot read back: its text as saved (shown as plain lines).
    Plain { lines: Vec<String> },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SectionDto {
    /// `rising_skills` ... `wishes_check`: the stable part of the element ids.
    pub key: &'static str,
    pub title: &'static str,
    pub points: Vec<PointDto>,
    /// The gap as the app sorted it (only in `for_you`, when the report has one).
    pub gap: Option<GapDto>,
    /// The wishes as the app measured them (only in `wishes_check`).
    pub wishes: Option<WishesDto>,
}

/// The gap: the user's background against the market, sorted in code.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapDto {
    /// `local` / `global`: the market it was judged on.
    pub scope: &'static str,
    /// `Judged by the app on postings in Ho Chi Minh City, Singapore or remote for backend engineer.`
    pub hint: String,
    /// Build on, learn next, at risk: always the three, in that order.
    pub groups: Vec<GapGroupDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapGroupDto {
    /// `build_on` / `learn_next` / `at_risk`.
    pub bucket: &'static str,
    pub title: &'static str,
    /// What the bucket means.
    pub hint: &'static str,
    /// What the group says when it holds no skill.
    pub empty: &'static str,
    pub skills: Vec<GapSkillDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GapSkillDto {
    pub skill: String,
    /// `18%`: its share of the field's postings over the window.
    pub share: String,
    pub direction: TagDto,
}

/// The user's wishes against the market, applied last.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WishesDto {
    /// Which market they were measured on.
    pub hint: String,
    pub wishes: Vec<WishDto>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WishDto {
    pub wish: String,
    /// `skill` / `domain`.
    pub kind: &'static str,
    /// The user wants to avoid it rather than have it.
    pub avoided: bool,
    /// `rising` / `steady` / `falling` / `unclear` / `no data`.
    pub signal: TagDto,
    /// `42 postings` / `no posting names it`.
    pub detail: String,
}

/// A term the skill list does not know.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TermDto {
    pub term: String,
    /// Items whose topics name it.
    pub items: u32,
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
                uncatalogued: data
                    .counts
                    .uncatalogued
                    .iter()
                    .map(|t| TermDto {
                        term: t.term.clone(),
                        items: t.items,
                    })
                    .collect(),
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
    let counts = &data.counts;
    let knows = knows_background(counts);
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
        let gap = (section == Section::ForYou && !counts.gap.is_empty()).then(|| gap_dto(counts));
        let wishes = (section == Section::WishesCheck && !counts.wishes.is_empty())
            .then(|| wishes_dto(counts));
        if !points.is_empty() || gap.is_some() || wishes.is_some() {
            out.push(SectionDto {
                key: section.as_str(),
                title: section_title(section, knows),
                points,
                gap,
                wishes,
            });
        }
    }
    out
}

fn tag((label, tone): (&'static str, Tone)) -> TagDto {
    TagDto {
        label,
        tone: tone.as_str(),
    }
}

fn gap_dto(counts: &SkillCounts) -> GapDto {
    GapDto {
        scope: scope_key(counts.gap_scope),
        hint: judged_on(counts.gap_scope, &counts.place, &counts.field),
        groups: BUCKETS
            .into_iter()
            .map(|bucket| {
                let (title, hint, empty) = bucket_words(bucket);
                GapGroupDto {
                    bucket: bucket_key(bucket),
                    title,
                    hint,
                    empty,
                    skills: counts
                        .gap
                        .iter()
                        .filter(|g| g.bucket == bucket)
                        .map(|g| GapSkillDto {
                            skill: g.skill.clone(),
                            share: percent_text(g.share_permille),
                            direction: tag(direction_tag(g.direction)),
                        })
                        .collect(),
                }
            })
            .collect(),
    }
}

fn wishes_dto(counts: &SkillCounts) -> WishesDto {
    WishesDto {
        hint: judged_on(counts.gap_scope, &counts.place, &[]),
        wishes: counts
            .wishes
            .iter()
            .map(|w| {
                let (signal, detail) = wish_signal(w);
                WishDto {
                    wish: w.wish.clone(),
                    kind: wish_kind(w.kind),
                    avoided: w.avoided,
                    signal: tag(signal),
                    detail,
                }
            })
            .collect(),
    }
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

/// The skill chart: one view per market scope, the switch between them, and the user's own leads
/// as a comparison line.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartDto {
    /// Local first (when the report has a place), then global: the order of the switch.
    pub scopes: Vec<ScopeDto>,
    /// The scope shown first: the one the gap was judged on.
    pub scope: &'static str,
    pub from_month: String,
    pub to_month: String,
    /// The report knows the user's background: directions and the profile / wish marks are shown,
    /// and the bars highlight the profile (a report saved before highlights the preferences).
    pub marks: bool,
    /// The user's own leads, for comparison (never counted in the market).
    pub pipeline: Option<String>,
}

/// One slice of the market's postings.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScopeDto {
    /// `local` / `global`.
    pub key: &'static str,
    /// `Local` / `Global`: the switch's choice.
    pub label: &'static str,
    /// What the scope covers: `Ho Chi Minh City, Singapore or remote` / `every posting`.
    pub covers: String,
    /// Postings read over the window.
    pub documents: u32,
    pub months: Vec<MonthDto>,
    pub skills: Vec<ChartSkillDto>,
    /// The percent the bars are scaled to (the most named skill's, at least 1).
    pub max_percent: u32,
    /// What the scope says about its own data (too little local data, no direction yet).
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonthDto {
    /// `2026-06`.
    pub month: String,
    /// `Jun`.
    pub label: String,
    /// Postings read that month.
    pub documents: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSkillDto {
    pub name: String,
    pub total: u32,
    /// Its share of the scope's postings over the window, as a whole percent (the bar).
    pub percent: u32,
    /// The same share in words: `18%`, `2.5%`.
    pub share: String,
    /// The count, the share and its change, in words (the tooltip of the row).
    pub description: String,
    /// `absent` / `added` / `avoided`: where the skill stands in the saved preferences.
    pub presence: &'static str,
    /// Where its share is going, decided in code (`None` for a report saved before).
    pub direction: Option<TagDto>,
    /// In the user's profile.
    pub in_profile: bool,
    /// One the user wishes for (wanted, nice to have or avoided).
    pub wished: bool,
}

impl ChartDto {
    fn new(digest: &Digest, prefs: &Preferences) -> Option<Self> {
        let counts = &digest.data.as_ref()?.counts;
        counts.skills.first()?;
        let partial = latest_month_is_partial(
            &counts.months,
            &digest.report.created_at.format("%Y-%m").to_string(),
        );
        let knows = knows_background(counts);
        let global = ScopeCounts {
            documents: counts.documents.clone(),
            skills: counts.skills.clone(),
        };
        let mut scopes = Vec::new();
        if let Some(local) = &counts.local {
            scopes.push(scope_dto(
                GapScope::Local,
                local,
                counts,
                partial,
                knows,
                prefs,
            ));
        }
        scopes.push(scope_dto(
            GapScope::Global,
            &global,
            counts,
            partial,
            knows,
            prefs,
        ));
        let scope = if counts.local.is_some() {
            scope_key(counts.gap_scope)
        } else {
            scope_key(GapScope::Global)
        };
        Some(Self {
            scopes,
            scope,
            from_month: counts.months.first().cloned().unwrap_or_default(),
            to_month: counts.months.last().cloned().unwrap_or_default(),
            marks: knows,
            pipeline: counts.pipeline.as_ref().map(pipeline_note),
        })
    }
}

fn scope_dto(
    scope: GapScope,
    data: &ScopeCounts,
    counts: &SkillCounts,
    partial: bool,
    knows: bool,
    prefs: &Preferences,
) -> ScopeDto {
    let documents: u32 = data.documents.iter().sum();
    let skills: Vec<ChartSkillDto> = data
        .skills
        .iter()
        .take(CHART_ROWS)
        .map(|skill| {
            let share = permille(skill.total, documents);
            ChartSkillDto {
                name: skill.skill.clone(),
                total: skill.total,
                percent: percent(share),
                share: percent_text(share),
                description: describe(skill, &data.documents, partial),
                // Any skill of the chart, rising or not: adding it makes no claim about the trend.
                presence: Field::Skills.presence(prefs, &skill.skill).as_str(),
                direction: knows.then(|| tag(direction_tag(skill.direction))),
                in_profile: skill.in_profile,
                wished: skill.wished,
            }
        })
        .collect();
    let (label, covers) = match scope {
        GapScope::Local if !counts.place.is_empty() => ("Local", counts.place.clone()),
        GapScope::Local => ("Local", "your place".to_string()),
        GapScope::Global => ("Global", "every posting".to_string()),
    };
    ScopeDto {
        key: scope_key(scope),
        label,
        covers,
        documents,
        months: counts
            .months
            .iter()
            .enumerate()
            .map(|(i, month)| MonthDto {
                month: month.clone(),
                label: month_label(month),
                documents: data.documents.get(i).copied().unwrap_or(0),
            })
            .collect(),
        max_percent: skills.iter().map(|s| s.percent).max().unwrap_or(0).max(1),
        note: scope_note(scope, data, counts.gap_scope, knows),
        skills,
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
    use crate::engine::graph::{Failure, Outcome, WaitReason};
    use crate::engine::health::Problem;
    use crate::engine::trends::digest::{
        Direction, GapBucket, GapScope, GapSkill, ScopeCounts, TermCount, WishKind, WishSignal,
        MARKET_SCAN_VERSION,
    };
    use crate::engine::trends::{
        CitedItem, Digest, DigestData, Period, Point, Section, SkillCount, SkillCounts, TrendRun,
    };
    use crate::engine::{Core, Paths, Secrets};

    use super::{
        overview, parse_provider, provider_name, run_result, sections, ChartDto, DigestBodyDto,
    };
    use crate::engine::prefs::Preferences;
    use crate::trends::demo;
    use crate::trends::rules::Cadence;

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

    fn digest_of(counts: SkillCounts) -> Digest {
        Digest {
            report: crate::engine::store::Report {
                id: 1,
                kind: "weekly".into(),
                period: "2026-W46".into(),
                title: "t".into(),
                body_md: String::new(),
                citations_json: String::new(),
                created_at: chrono::Utc::now(),
            },
            data: Some(DigestData {
                counts,
                ..DigestData::default()
            }),
        }
    }

    fn skill(name: &str, by_month: &[u32]) -> SkillCount {
        SkillCount {
            skill: name.into(),
            by_month: by_month.to_vec(),
            total: by_month.iter().sum(),
            ..SkillCount::default()
        }
    }

    #[test]
    fn a_chart_shows_shares_and_says_where_each_skill_stands() {
        let counts = SkillCounts {
            months: vec!["2026-10".into(), "2026-11".into()],
            documents: vec![100, 100],
            skills: vec![skill("Rust", &[10, 20]), skill("Go", &[5, 0])],
            ..SkillCounts::default()
        };
        let digest = digest_of(counts);
        let prefs = Preferences {
            nice_skills: vec!["Go".into()],
            ..Preferences::default()
        };
        let chart = ChartDto::new(&digest, &prefs).expect("a chart");
        // A report saved before P15: one global scope, no marks, no direction, no note.
        assert!(!chart.marks && chart.pipeline.is_none());
        assert_eq!(chart.scope, "global");
        assert_eq!(chart.scopes.len(), 1);
        let global = &chart.scopes[0];
        assert_eq!(
            (global.key, global.label, global.documents),
            ("global", "Global", 200)
        );
        assert_eq!(global.note, None);
        assert_eq!(
            global
                .months
                .iter()
                .map(|m| (m.label.as_str(), m.documents))
                .collect::<Vec<_>>(),
            [("Oct", 100), ("Nov", 100)]
        );
        assert_eq!(global.max_percent, 15);
        assert_eq!(
            global
                .skills
                .iter()
                .map(|s| (s.name.as_str(), s.percent, s.share.as_str(), s.presence))
                .collect::<Vec<_>>(),
            [("Rust", 15, "15%", "absent"), ("Go", 3, "2.5%", "added")]
        );
        assert!(global.skills.iter().all(|s| s.direction.is_none()));
        assert_eq!(
            (chart.from_month.as_str(), chart.to_month.as_str()),
            ("2026-10", "2026-11")
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
    fn a_chart_with_a_place_switches_between_local_and_global_and_starts_on_the_gap_scope() {
        let mut rust = skill("Rust", &[30, 40]);
        rust.direction = Direction::Rising;
        rust.in_profile = true;
        let mut php = skill("PHP", &[10, 5]);
        php.direction = Direction::Falling;
        php.wished = true;
        let mut local_rust = skill("Rust", &[3, 4]);
        local_rust.in_profile = true;
        let counts = SkillCounts {
            version: MARKET_SCAN_VERSION,
            months: vec!["2026-10".into(), "2026-11".into()],
            documents: vec![200, 200],
            skills: vec![rust, php],
            local: Some(ScopeCounts {
                documents: vec![10, 10],
                skills: vec![local_rust],
            }),
            place: "Singapore or remote".into(),
            pipeline: Some(ScopeCounts {
                documents: vec![4, 8],
                skills: vec![skill("Rust", &[3, 6]), skill("Go", &[1, 1])],
            }),
            gap_scope: GapScope::Global,
            ..SkillCounts::default()
        };
        let chart =
            ChartDto::new(&digest_of(counts.clone()), &Preferences::default()).expect("a chart");
        assert!(chart.marks);
        assert_eq!(
            chart.scopes.iter().map(|s| s.key).collect::<Vec<_>>(),
            ["local", "global"]
        );
        assert_eq!(
            chart.scope, "global",
            "the local data was too thin for the gap"
        );
        let local = &chart.scopes[0];
        assert_eq!(
            (local.label, local.covers.as_str(), local.documents),
            ("Local", "Singapore or remote", 20)
        );
        assert!(local
            .note
            .as_deref()
            .is_some_and(|n| n.starts_with("Too little local data")));
        assert_eq!(local.skills[0].share, "35%");
        let global = &chart.scopes[1];
        assert_eq!(global.note, None);
        let rust = &global.skills[0];
        assert!(rust.in_profile && !rust.wished);
        assert_eq!(
            rust.direction.as_ref().map(|t| (t.label, t.tone)),
            Some(("rising", "ok"))
        );
        let php = &global.skills[1];
        assert!(php.wished && !php.in_profile);
        assert_eq!(
            php.direction.as_ref().map(|t| (t.label, t.tone)),
            Some(("falling", "warn"))
        );
        assert_eq!(
            chart.pipeline.as_deref(),
            Some("Your own leads, for comparison: 12 over the window; they name Rust, Go most. They are not counted in the market above.")
        );

        let local_gap = SkillCounts {
            gap_scope: GapScope::Local,
            ..counts
        };
        let chart = ChartDto::new(&digest_of(local_gap), &Preferences::default()).expect("a chart");
        assert_eq!(chart.scope, "local");
        assert!(
            chart.scopes[0]
                .note
                .as_deref()
                .is_some_and(|n| n.starts_with("Too few postings")),
            "no local skill has a direction yet"
        );
    }

    #[test]
    fn the_gap_and_the_wishes_sit_in_their_sections_even_without_a_point() {
        let gap = |name: &str, bucket, direction, share| GapSkill {
            skill: name.into(),
            bucket,
            direction,
            share_permille: share,
        };
        let data = DigestData {
            items: vec![item(7, "https://a.test/1")],
            points: vec![point(Section::WishesCheck, "PHP", &[7])],
            counts: SkillCounts {
                version: MARKET_SCAN_VERSION,
                place: "Singapore".into(),
                field: vec!["backend".into(), "engineer".into()],
                gap_scope: GapScope::Local,
                gap: vec![
                    gap("Rust", GapBucket::BuildOn, Direction::Rising, 182),
                    gap("Kubernetes", GapBucket::LearnNext, Direction::Steady, 95),
                ],
                uncatalogued: vec![TermCount {
                    term: "Temporal".into(),
                    items: 4,
                }],
                wishes: vec![
                    WishSignal {
                        wish: "PHP".into(),
                        kind: WishKind::Skill,
                        avoided: true,
                        direction: Direction::Falling,
                        mentions: 12,
                    },
                    WishSignal {
                        wish: "climate tech".into(),
                        kind: WishKind::Domain,
                        avoided: false,
                        direction: Direction::Unclear,
                        mentions: 0,
                    },
                ],
                ..SkillCounts::default()
            },
            ..DigestData::default()
        };
        let out = sections(&data, &Preferences::default());
        assert_eq!(
            out.iter().map(|s| (s.key, s.title)).collect::<Vec<_>>(),
            [
                ("for_you", "Your gap: build on, learn next, at risk"),
                ("wishes_check", "Your wishes vs the market")
            ]
        );
        let gap = out[0].gap.as_ref().expect("the gap");
        assert!(out[0].points.is_empty());
        assert_eq!(gap.scope, "local");
        assert_eq!(
            gap.hint,
            "Judged by the app on postings in Singapore for backend engineer."
        );
        assert_eq!(
            gap.groups
                .iter()
                .map(|g| (g.bucket, g.title, g.skills.len()))
                .collect::<Vec<_>>(),
            [
                ("build_on", "Build on", 1),
                ("learn_next", "Learn next", 1),
                ("at_risk", "At risk", 0)
            ]
        );
        let rust = &gap.groups[0].skills[0];
        assert_eq!(
            (rust.share.as_str(), rust.direction.label),
            ("18%", "rising")
        );
        assert_eq!(gap.groups[1].skills[0].share, "9.5%");

        let wishes = out[1].wishes.as_ref().expect("the wishes");
        assert_eq!(out[1].points.len(), 1);
        let read: Vec<_> = wishes
            .wishes
            .iter()
            .map(|w| {
                (
                    w.wish.as_str(),
                    w.kind,
                    w.avoided,
                    w.signal.label,
                    w.detail.as_str(),
                )
            })
            .collect();
        assert_eq!(
            read,
            [
                ("PHP", "skill", true, "falling", "12 postings"),
                (
                    "climate tech",
                    "domain",
                    false,
                    "no data",
                    "no posting names it"
                )
            ]
        );

        // A report saved before P15: its "for you" points keep their old title, nothing is added.
        let old = DigestData {
            items: vec![item(7, "https://a.test/1")],
            points: vec![point(Section::ForYou, "", &[7])],
            ..DigestData::default()
        };
        let out = sections(&old, &Preferences::default());
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].title, "What it means for you");
        assert!(out[0].gap.is_none() && out[0].wishes.is_none());
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
        let DigestBodyDto::Data {
            sections,
            dropped,
            uncatalogued,
        } = digest.body
        else {
            panic!("the demo digest has data")
        };
        assert_eq!(dropped, 1);
        assert_eq!(sections.len(), 7, "every section has a point");
        assert!(!uncatalogued.is_empty());
        let gap = sections
            .iter()
            .find_map(|s| s.gap.as_ref())
            .expect("the demo has a gap");
        assert!(
            gap.groups.iter().all(|g| !g.skills.is_empty()),
            "every bucket has a skill"
        );
        let wishes = sections
            .iter()
            .find_map(|s| s.wishes.as_ref())
            .expect("the demo has wishes");
        assert!(wishes.wishes.iter().any(|w| w.avoided));
        assert!(wishes.wishes.iter().any(|w| w.kind == "domain"));
        assert!(wishes.wishes.iter().any(|w| w.signal.label == "no data"));
        assert!(
            sections
                .iter()
                .flat_map(|s| &s.points)
                .all(|p| !p.citations.is_empty()),
            "every point cites an item"
        );
        let chart = seen.chart.expect("a chart");
        assert!(chart.marks && chart.pipeline.is_some());
        assert_eq!(chart.scope, "local");
        assert_eq!(
            chart
                .scopes
                .iter()
                .map(|s| (s.key, s.skills.len()))
                .collect::<Vec<_>>(),
            [("local", 10), ("global", 10)]
        );
        assert!(chart.scopes[0].documents < chart.scopes[1].documents);

        // A report that is gone falls back to the newest.
        let missing = overview(&core, Some(9999), Cadence::Quarterly).expect("an overview");
        assert_eq!(missing.digest.map(|d| d.report_id), Some(digest.report_id));
        assert!(missing.period.label.contains("-Q"));
    }
}
