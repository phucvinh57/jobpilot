//! Pure rules of this context, unit-tested here: how a period is named, how a skill's trend is
//! worded (counted in code, never by the AI), how a source's last run reads, how a finished run is
//! summarised, and where a topic stands in the saved preferences.

use chrono::{Local, NaiveDate};
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::prefs::Preferences;
use crate::engine::store::{Report, Source};
use crate::engine::trends::{
    Digest, Period, Section, SkillCount, SourceKind, SourceStatus, TrendRun,
};

pub use crate::shared::topics::Presence;
use crate::shared::topics::{self, presence_in, with_item};

// ------------------------------------------------------------------ periods

/// Weekly digest or quarterly summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Weekly,
    Quarterly,
}

impl Cadence {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "weekly" => Some(Cadence::Weekly),
            "quarterly" => Some(Cadence::Quarterly),
            _ => None,
        }
    }

    /// The week or quarter that contains `today`.
    pub fn period(self, today: NaiveDate) -> Period {
        match self {
            Cadence::Weekly => Period::week_of(today),
            Cadence::Quarterly => Period::quarter_of(today),
        }
    }
}

/// `Week of 9 November` or `Q4 2026`.
pub fn period_title(period: Period) -> String {
    match period {
        Period::Week { .. } => format!("Week of {}", period.start().format("%-d %B")),
        Period::Quarter { year, quarter } => format!("Q{quarter} {year}"),
    }
}

/// What the page header says under the title: the period of the digest shown and how much it read.
pub fn header_summary(shown: Option<&Digest>, current: Period) -> String {
    match shown {
        Some(digest) => {
            let covers = digest
                .period()
                .map_or_else(|| digest.report.title.clone(), period_title);
            match &digest.data {
                Some(data) => format!(
                    "{covers} \u{b7} from {} postings and articles in your sources",
                    data.items_read
                ),
                None => covers,
            }
        }
        None => format!(
            "No digest yet \u{b7} {} is the current period",
            period_title(current)
        ),
    }
}

/// When a report was made, in the user's time zone.
pub fn when(report: &Report) -> String {
    report
        .created_at
        .with_timezone(&Local)
        .format("%Y-%m-%d %H:%M")
        .to_string()
}

// ------------------------------------------------------------------ skill trends

/// Documents a month needs before its share of them says anything.
const MIN_DOCUMENTS: u32 = 10;
/// Two months are compared only when neither holds more than this many times the documents of the
/// other.
const MAX_SIZE_RATIO: u32 = 2;
/// Shares closer than this (percentage points) are the same.
const SAME_POINTS: f64 = 0.5;

/// Is the latest month of the counts not over at the time the digest was made? The documents of
/// that month are then the first ones only. (`created` and `month` are `YYYY-MM`.)
pub fn latest_month_is_partial(months: &[String], created: &str) -> bool {
    months.last().is_some_and(|last| created <= last.as_str())
}

/// One line per skill: the count and, when it is fair, how a month's share of documents compares
/// with the month before. A raw count would grow with the number of documents read, so the share
/// (count / documents of that month) is compared, and only when both months are of a similar size
/// and enough documents; otherwise the line says no more than the count. A month that is not over
/// is shown as "so far" and left out of the comparison: the last two whole months are compared.
pub fn describe(skill: &SkillCount, documents: &[u32], latest_partial: bool) -> String {
    let n = skill.by_month.len();
    let last = skill.by_month.last().copied().unwrap_or(0);
    let skipped = usize::from(latest_partial);
    let (this_month, prior) = if latest_partial {
        (format!("{last} so far this month"), "the month before")
    } else {
        (format!("{last} this month"), "last month")
    };
    let line = |tail: &str| format!("{} in total \u{b7} {this_month}{tail}", skill.total);
    if documents.len() != n || n < 2 + skipped {
        return line("");
    }
    let (before_at, now_at) = (n - 2 - skipped, n - 1 - skipped);
    let (docs_before, docs_now) = (documents[before_at], documents[now_at]);
    if docs_before < MIN_DOCUMENTS || docs_now < MIN_DOCUMENTS {
        return line(", too few documents to compare");
    }
    if docs_now.max(docs_before) > docs_now.min(docs_before).saturating_mul(MAX_SIZE_RATIO) {
        return line(", months too different in size to compare");
    }
    let share = |count: u32, docs: u32| f64::from(count) * 100.0 / f64::from(docs);
    let (now, before) = (
        share(skill.by_month[now_at], docs_now),
        share(skill.by_month[before_at], docs_before),
    );
    let change = if (now - before).abs() < SAME_POINTS {
        format!("about the same as {prior} ({before:.1}%)")
    } else if now > before {
        format!("up from {before:.1}% {prior}")
    } else {
        format!("down from {before:.1}% {prior}")
    };
    if latest_partial {
        line(&format!(
            " \u{b7} last month {now:.1}% of documents, {change}"
        ))
    } else {
        format!(
            "{} in total \u{b7} {now:.1}% of this month's documents, {change}",
            skill.total
        )
    }
}

// ------------------------------------------------------------------ links

/// Only web addresses are opened, whatever the saved data says.
pub fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

// ------------------------------------------------------------------ sources

/// The colour family of a tag; the names are the web kit's `Tone`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Ok,
    Warn,
    Danger,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Neutral => "neutral",
            Tone::Ok => "ok",
            Tone::Warn => "warn",
            Tone::Danger => "danger",
        }
    }
}

/// What the source's last run says, in words.
pub fn status_text(source: &Source) -> String {
    match source.last_status.as_deref() {
        None => "not collected yet".into(),
        Some("needs_human") => "needs you: see the banner above".into(),
        Some(status) => format!("last run: {status}"),
    }
}

/// The tag of a source: how its last run went, and in which tone.
pub fn status_tag(source: &Source) -> (&'static str, Tone) {
    match source.last_status.as_deref() {
        None => ("not run", Tone::Neutral),
        Some("ok") => ("ok", Tone::Ok),
        Some("needs_human") => ("needs you", Tone::Warn),
        Some(status) if status.starts_with("failed") => ("failed", Tone::Danger),
        Some(_) => ("left out", Tone::Neutral),
    }
}

/// `Hacker News` or `RSS / Atom feed · blog.example.com`: the kind, and the host of its address
/// (never the whole address: a feed address may carry a token).
pub fn describe_source(source: &Source) -> String {
    let label = SourceKind::parse(&source.kind)
        .map_or_else(|| source.kind.clone(), |k| k.label().to_string());
    match url::Url::parse(&source.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
    {
        Some(host) => format!("{label} \u{b7} {host}"),
        None => label,
    }
}

/// What a collection waiting for the user is stuck at.
pub fn challenge_phrase(kind: ChallengeKind) -> &'static str {
    match kind {
        ChallengeKind::Login => "needs you to sign in",
        ChallengeKind::Captcha => "shows a captcha",
        ChallengeKind::Blocked => "blocks automated access",
    }
}

pub fn challenge_name(kind: ChallengeKind) -> &'static str {
    match kind {
        ChallengeKind::Login => "login",
        ChallengeKind::Captcha => "captcha",
        ChallengeKind::Blocked => "blocked",
    }
}

// ------------------------------------------------------------------ runs

/// The headline of a finished run and one line per source (and per thing that went less well).
pub fn summarize(run: &TrendRun) -> (String, Vec<String>) {
    let d = &run.digest;
    let headline = if d.report_id.is_some() {
        format!(
            "Digest ready: {} point(s) from {} item(s).",
            d.points.len(),
            d.items.len()
        )
    } else {
        format!("Collected {} new item(s).", run.stored.new)
    };
    let mut lines: Vec<String> = run
        .outcomes
        .iter()
        .map(|o| {
            let name = SourceKind::parse(&o.kind).map_or("Report search", SourceKind::label);
            let status = match &o.status {
                SourceStatus::Ok => format!("{} new item(s)", o.items),
                SourceStatus::Skipped { reason } => format!("skipped ({reason})"),
                SourceStatus::TimedOut => "timed out, left out of this run".to_string(),
                SourceStatus::Failed { error } => format!("failed ({error})"),
                SourceStatus::NeedsHuman { .. } => "needs you".to_string(),
            };
            format!("{name}: {status}")
        })
        .collect();
    if d.failed_batches > 0 {
        lines.push(format!(
            "{} batch(es) of items could not be read by the AI provider and were left out.",
            d.failed_batches
        ));
    }
    if d.dropped > 0 {
        lines.push(format!(
            "{} point(s) were left out: no valid source, or a quote that is not in the source.",
            d.dropped
        ));
    }
    (headline, lines)
}

// ------------------------------------------------------------------ preferences

/// A list of the preferences a topic of the digest can be added to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Domains,
    Skills,
}

impl Field {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "domains" => Some(Field::Domains),
            "skills" => Some(Field::Skills),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Field::Domains => "domains",
            Field::Skills => "skills",
        }
    }

    fn list(self, prefs: &Preferences) -> &Vec<String> {
        match self {
            Field::Domains => &prefs.domains,
            Field::Skills => &prefs.must_skills,
        }
    }

    fn list_mut(self, prefs: &mut Preferences) -> &mut Vec<String> {
        match self {
            Field::Domains => &mut prefs.domains,
            Field::Skills => &mut prefs.must_skills,
        }
    }

    /// Add `text` to this list of `prefs`; `false` when there was nothing to add (see [`with_item`])
    /// or it is in one of the related lists already (see [`presence`](Self::presence)).
    pub fn add(self, prefs: &mut Preferences, text: &str) -> bool {
        if self.presence(prefs, text) != Presence::Absent {
            return false;
        }
        match with_item(self.list(prefs), text) {
            Some(list) => {
                *self.list_mut(prefs) = list;
                true
            }
            None => false,
        }
    }

    /// Where `text` stands in `prefs` (ignoring case, with Unicode-aware lowercase like
    /// `Preferences::normalized`): skills count in the must, nice and avoid lists, domains in the
    /// interested and avoided ones.
    pub fn presence(self, prefs: &Preferences, text: &str) -> Presence {
        let (wanted, avoided): (&[&[String]], &[&[String]]) = match self {
            Field::Domains => (&[&prefs.domains], &[&prefs.avoid_domains]),
            Field::Skills => (
                &[&prefs.must_skills, &prefs.nice_skills],
                &[&prefs.avoid_skills],
            ),
        };
        presence_in(wanted, avoided, text)
    }

    /// Why `text` is not in `prefs` after an attempt to add it: it is in the avoid list, the list
    /// really is full, or something else went wrong.
    pub fn why_not_added(self, prefs: &Preferences, text: &str) -> String {
        topics::why_not_added(self.presence(prefs, text), self.list(prefs).len(), text)
    }
}

/// The list of the preferences a point of `section` is added to: domains, else skills.
pub fn target(section: Section) -> Field {
    match section {
        Section::Domains => Field::Domains,
        _ => Field::Skills,
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::store::{Source, SourcePurpose};
    use crate::engine::trends::{
        Period, Section, SkillCount, SourceOutcome, SourceStatus, TrendRun,
    };

    use super::{
        describe, describe_source, header_summary, is_web_url, latest_month_is_partial,
        period_title, status_tag, summarize, target, Cadence, Field, Preferences, Presence, Tone,
    };

    fn period() -> Period {
        Period::parse("2026-W46").unwrap()
    }

    fn skill(by_month: &[u32]) -> SkillCount {
        SkillCount {
            skill: "Rust".into(),
            by_month: by_month.to_vec(),
            total: by_month.iter().sum(),
        }
    }

    const DOT: &str = "\u{b7}";

    // ---- periods

    #[test]
    fn a_period_is_named_for_the_header() {
        assert_eq!(period_title(period()), "Week of 9 November");
        assert_eq!(period_title(Period::parse("2026-Q4").unwrap()), "Q4 2026");
        assert_eq!(
            header_summary(None, period()),
            "No digest yet \u{b7} Week of 9 November is the current period"
        );
    }

    #[test]
    fn a_cadence_is_the_week_or_quarter_of_today() {
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 14).unwrap();
        assert_eq!(Cadence::Weekly.period(today).label(), "2026-W46");
        assert_eq!(Cadence::Quarterly.period(today).label(), "2026-Q4");
        assert_eq!(Cadence::parse("weekly"), Some(Cadence::Weekly));
        assert_eq!(Cadence::parse("quarterly"), Some(Cadence::Quarterly));
        assert_eq!(Cadence::parse("daily"), None);
    }

    // ---- runs

    #[test]
    fn a_finished_run_is_summarised_per_source() {
        let mut run = TrendRun::new(period());
        run.outcomes = vec![
            SourceOutcome {
                source_id: Some(1),
                kind: "hackernews".into(),
                status: SourceStatus::Ok,
                items: 4,
            },
            SourceOutcome {
                source_id: Some(2),
                kind: "yc".into(),
                status: SourceStatus::TimedOut,
                items: 0,
            },
            SourceOutcome {
                source_id: None,
                kind: "report_search".into(),
                status: SourceStatus::Skipped {
                    reason: "no search API key saved".into(),
                },
                items: 0,
            },
        ];
        let (headline, lines) = summarize(&run);
        assert_eq!(headline, "Collected 0 new item(s).");
        assert_eq!(
            lines,
            [
                "Hacker News: 4 new item(s)",
                "YC companies: timed out, left out of this run",
                "Report search: skipped (no search API key saved)"
            ]
        );

        run.digest.report_id = Some(3);
        run.digest.items = vec![1, 2, 3];
        run.digest.failed_batches = 1;
        run.digest.dropped = 2;
        let (headline, lines) = summarize(&run);
        assert_eq!(headline, "Digest ready: 0 point(s) from 3 item(s).");
        assert!(
            lines.iter().any(|l| l.starts_with("1 batch(es)"))
                && lines
                    .iter()
                    .any(|l| l.starts_with("2 point(s) were left out"))
        );
    }

    // ---- skill trends

    #[test]
    fn a_skill_is_described_by_its_share_of_the_documents() {
        // 20 of 100 documents last month, 30 of 100 this month: up.
        assert_eq!(
            describe(&skill(&[10, 20, 30]), &[100, 100, 100], false),
            format!("60 in total {DOT} 30.0% of this month's documents, up from 20.0% last month")
        );
        assert_eq!(
            describe(&skill(&[30, 10]), &[100, 100], false),
            format!(
                "40 in total {DOT} 10.0% of this month's documents, down from 30.0% last month"
            )
        );
        assert_eq!(
            describe(&skill(&[20, 20]), &[100, 100], false),
            format!("40 in total {DOT} 20.0% of this month's documents, about the same as last month (20.0%)")
        );
    }

    #[test]
    fn more_documents_alone_are_not_a_rise() {
        // The count doubles, but so do the documents read: the share is the same.
        let line = describe(&skill(&[10, 20]), &[50, 100], false);
        assert!(
            line.contains("about the same as last month (20.0%)"),
            "{line}"
        );
        // The count falls, but the share rises.
        let line = describe(&skill(&[20, 15]), &[100, 60], false);
        assert!(
            line.contains("up from 20.0% last month") && line.contains("25.0% of this month's"),
            "{line}"
        );
    }

    #[test]
    fn a_month_that_is_not_over_is_left_out_of_the_comparison() {
        // The last two whole months (10 -> 20 of 100 documents) are compared; this month is "so far".
        let line = describe(&skill(&[10, 20, 30]), &[100, 100, 100], true);
        assert_eq!(
            line,
            format!("60 in total {DOT} 30 so far this month {DOT} last month 20.0% of documents, up from 10.0% the month before")
        );
        assert_eq!(
            describe(&skill(&[30, 10, 5]), &[100, 100, 100], true),
            format!(
                "45 in total {DOT} 5 so far this month {DOT} last month 10.0% of documents, down from 30.0% the month before"
            )
        );
        assert!(describe(&skill(&[20, 20, 1]), &[100, 100, 100], true)
            .contains("about the same as the month before (20.0%)"));
        // The size of the month that is not over does not matter: only the whole months are compared.
        assert!(describe(&skill(&[10, 20, 3]), &[100, 100, 4], true)
            .contains("up from 10.0% the month before"));
        // The same counts with the month over: the last two months are compared instead.
        assert!(describe(&skill(&[10, 20, 3]), &[100, 100, 4], false).contains("too few documents"));
    }

    #[test]
    fn nothing_is_said_about_the_trend_when_it_would_not_be_fair() {
        let rising = skill(&[1, 2, 5]);
        // Months of very different size (more than twice as many documents).
        let line = describe(&rising, &[100, 40, 90], false);
        assert!(
            line.ends_with("months too different in size to compare"),
            "{line}"
        );
        assert!(!line.contains("up from") && !line.contains('%'), "{line}");
        let line = describe(&rising, &[40, 100, 90], true);
        assert_eq!(
            line,
            format!(
                "8 in total {DOT} 5 so far this month, months too different in size to compare"
            )
        );
        // Too few documents for a share to mean anything.
        let line = describe(&rising, &[100, 100, 9], false);
        assert!(line.ends_with("too few documents to compare"), "{line}");
        let line = describe(&rising, &[100, 9, 100], true);
        assert!(
            line.ends_with("so far this month, too few documents to compare"),
            "{line}"
        );
        // No document counts, too few months (a month that is not over needs two whole ones before it).
        assert_eq!(
            describe(&rising, &[], false),
            format!("8 in total {DOT} 5 this month")
        );
        assert_eq!(
            describe(&skill(&[3]), &[50], false),
            format!("3 in total {DOT} 3 this month")
        );
        assert_eq!(
            describe(&skill(&[]), &[], false),
            format!("0 in total {DOT} 0 this month")
        );
        assert_eq!(
            describe(&skill(&[3, 4]), &[50, 50], true),
            format!("7 in total {DOT} 4 so far this month")
        );
        assert_eq!(
            describe(&skill(&[4]), &[50], true),
            format!("4 in total {DOT} 4 so far this month")
        );
    }

    #[test]
    fn the_latest_month_is_partial_while_it_is_not_over() {
        let months = vec!["2026-10".to_string(), "2026-11".to_string()];
        assert!(
            latest_month_is_partial(&months, "2026-11"),
            "made in the month"
        );
        assert!(
            latest_month_is_partial(&months, "2026-10"),
            "made before the month"
        );
        assert!(
            !latest_month_is_partial(&months, "2026-12"),
            "made after it"
        );
        assert!(!latest_month_is_partial(&[], "2026-12"));
    }

    // ---- links

    #[test]
    fn only_web_addresses_are_opened() {
        assert!(is_web_url("https://a.test/x") && is_web_url("http://a.test"));
        for bad in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "mailto:a@b.c",
            "ftp://x",
            "",
        ] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }

    // ---- sources

    fn source(last_status: Option<&str>) -> Source {
        Source {
            id: 1,
            kind: "hackernews".into(),
            name: "Hacker News".into(),
            url: String::new(),
            purpose: SourcePurpose::Trends,
            enabled: true,
            session_id: None,
            last_run_at: None,
            last_status: last_status.map(str::to_string),
        }
    }

    #[test]
    fn a_source_is_tagged_by_how_its_last_run_ended() {
        assert_eq!(status_tag(&source(None)), ("not run", Tone::Neutral));
        assert_eq!(status_tag(&source(Some("ok"))), ("ok", Tone::Ok));
        assert_eq!(
            status_tag(&source(Some("needs_human"))),
            ("needs you", Tone::Warn)
        );
        assert_eq!(
            status_tag(&source(Some("failed: HTTP 404"))),
            ("failed", Tone::Danger)
        );
        assert_eq!(
            status_tag(&source(Some("timed out"))),
            ("left out", Tone::Neutral)
        );
    }

    #[test]
    fn a_source_shows_its_kind_and_host_but_never_the_whole_address() {
        assert_eq!(describe_source(&source(None)), "Hacker News");
        let mut feed = source(None);
        feed.kind = "rss".into();
        feed.url = "https://blog.example.com/feed.xml?token=secret".into();
        let line = describe_source(&feed);
        assert_eq!(line, format!("RSS / Atom feed {DOT} blog.example.com"));
        assert!(!line.contains("secret"));
    }

    // ---- preferences

    #[test]
    fn a_point_is_added_to_the_list_of_its_kind() {
        assert_eq!(target(Section::RisingSkills), Field::Skills);
        assert_eq!(target(Section::Domains), Field::Domains);
    }

    #[test]
    fn a_topic_is_added_once_and_never_against_the_avoid_list() {
        let mut prefs = Preferences::default();
        assert_eq!(Field::Skills.presence(&prefs, " Rust "), Presence::Absent);
        assert!(Field::Skills.add(&mut prefs, " Rust "));
        assert_eq!(prefs.must_skills, ["Rust"]);
        assert_eq!(Field::Skills.presence(&prefs, "rust"), Presence::Added);
        assert!(
            !Field::Skills.add(&mut prefs, "RUST"),
            "already there, ignoring case"
        );

        prefs.nice_skills.push("Go".into());
        assert_eq!(
            Field::Skills.presence(&prefs, "go"),
            Presence::Added,
            "the nice list counts"
        );
        prefs.avoid_skills.push("PHP".into());
        assert_eq!(Field::Skills.presence(&prefs, "php"), Presence::Avoided);
        assert!(!Field::Skills.add(&mut prefs, "php"));
        assert_eq!(
            Field::Skills.why_not_added(&prefs, "php"),
            "\"php\" is in your avoid list, so it was not added."
        );

        assert!(Field::Domains.add(&mut prefs, "fintech"));
        assert_eq!(prefs.domains, ["fintech"]);
        assert_eq!(
            Field::Domains.presence(&prefs, "Rust"),
            Presence::Absent,
            "domains and skills are separate"
        );
        assert!(!Field::Domains.add(&mut prefs, "   "), "blank");
    }

    #[test]
    fn a_full_list_says_why_nothing_was_added() {
        let mut prefs = Preferences {
            must_skills: (0..crate::engine::prefs::chat::MAX_LIST_ITEMS)
                .map(|i| format!("skill {i}"))
                .collect(),
            ..Preferences::default()
        };
        assert!(!Field::Skills.add(&mut prefs, "one more"));
        assert!(Field::Skills
            .why_not_added(&prefs, "one more")
            .contains("holds up to"));
        assert_eq!(
            Field::Skills.why_not_added(&Preferences::default(), "x"),
            "Could not add \"x\"."
        );
    }

    #[test]
    fn fields_and_presences_have_stable_names() {
        for field in [Field::Domains, Field::Skills] {
            assert_eq!(Field::parse(field.as_str()), Some(field));
        }
        assert_eq!(Field::parse("locations"), None);
        assert_eq!(
            [Presence::Absent, Presence::Added, Presence::Avoided].map(Presence::as_str),
            ["absent", "added", "avoided"]
        );
    }
}
