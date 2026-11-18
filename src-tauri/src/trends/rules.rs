//! Pure rules of this context, unit-tested here: how a period is named, how a skill's trend is
//! worded (counted in code, never by the AI), how a source's last run reads, how a finished run is
//! summarised, where a topic stands in the saved preferences, and how the market numbers read
//! (shares, directions, the gap and the wishes: all decided in code, never by the AI).

use jobpilot_engine::graph::nodes::ChallengeKind;
use jobpilot_engine::prefs::Preferences;
use jobpilot_engine::store::{Report, Source};
use jobpilot_engine::trends::digest::{
    Direction, GapBucket, GapScope, ScopeCounts, WishKind, WishSignal, MARKET_SCAN_VERSION,
};
use jobpilot_engine::trends::{
    Digest, Period, Section, SkillCount, SkillCounts, SourceKind, SourceStatus, TrendRun,
};
use chrono::{Local, NaiveDate};

/// How a period is named (`Week of 9 November`): the sidebar's job list names it the same way.
pub use crate::shared::activity::period_title;
pub use crate::shared::topics::Presence;
use crate::shared::topics::{self, presence_in, with_item};

// ------------------------------------------------------------------ periods

/// How long a report covers: a week, a month, a quarter or a year.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cadence {
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

impl Cadence {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "weekly" => Some(Cadence::Weekly),
            "monthly" => Some(Cadence::Monthly),
            "quarterly" => Some(Cadence::Quarterly),
            "yearly" => Some(Cadence::Yearly),
            _ => None,
        }
    }

    /// The period of this cadence that contains `today`.
    pub fn period(self, today: NaiveDate) -> Period {
        match self {
            Cadence::Weekly => Period::week_of(today),
            Cadence::Monthly => Period::month_of(today),
            Cadence::Quarterly => Period::quarter_of(today),
            Cadence::Yearly => Period::year_of(today),
        }
    }

    /// The cadence of `period`.
    pub fn of(period: Period) -> Self {
        match period {
            Period::Week { .. } => Cadence::Weekly,
            Period::Month { .. } => Cadence::Monthly,
            Period::Quarter { .. } => Cadence::Quarterly,
            Period::Year { .. } => Cadence::Yearly,
        }
    }

    /// As the web app spells it.
    pub fn as_str(self) -> &'static str {
        match self {
            Cadence::Weekly => "weekly",
            Cadence::Monthly => "monthly",
            Cadence::Quarterly => "quarterly",
            Cadence::Yearly => "yearly",
        }
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

// ------------------------------------------------------------------ market numbers

/// `part` of `whole` in per mille, rounded (0 when there is no whole).
pub fn permille(part: u32, whole: u32) -> u32 {
    jobpilot_engine::trends::digest::permille(u64::from(part), u64::from(whole))
}

/// A per mille as a whole percent, rounded half up: what a bar is drawn and labelled with.
pub fn percent(permille: u32) -> u32 {
    permille.saturating_add(5) / 10
}

/// A per mille in words: `18%`, with one decimal below 10% (`2.5%`), `0%` for none.
pub fn percent_text(permille: u32) -> String {
    match permille {
        0 => "0%".into(),
        1..=99 => format!("{}.{}%", permille / 10, permille % 10),
        _ => format!("{}%", percent(permille)),
    }
}

/// Does the report know the user's background and place? A report saved before the market scan
/// has none of it: the screen then shows no direction, mark, scope, gap or wish. Told by the
/// counts' version, so a new report with thin data is not taken for an old one.
pub fn knows_background(counts: &SkillCounts) -> bool {
    counts.version >= MARKET_SCAN_VERSION
}

/// How a direction reads on its tag, and the tag's tone.
pub fn direction_tag(direction: Direction) -> (&'static str, Tone) {
    match direction {
        Direction::Rising => ("rising", Tone::Ok),
        Direction::Steady => ("steady", Tone::Accent),
        Direction::Falling => ("falling", Tone::Warn),
        Direction::Unclear => ("unclear", Tone::Neutral),
    }
}

/// How a scope is spelled for the web app.
pub fn scope_key(scope: GapScope) -> &'static str {
    match scope {
        GapScope::Global => "global",
        GapScope::Local => "local",
    }
}

/// `2026-06` -> `Jun`; anything else as it is.
pub fn month_label(month: &str) -> String {
    const NAMES: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    month
        .split_once('-')
        .and_then(|(_, m)| m.parse::<usize>().ok())
        .and_then(|m| m.checked_sub(1))
        .and_then(|m| NAMES.get(m))
        .map_or_else(|| month.to_string(), |name| (*name).to_string())
}

/// What a scope of the chart says about its own data, when something needs saying: the local
/// market was too thin for the gap, or no skill has a direction yet.
pub fn scope_note(
    scope: GapScope,
    counts: &ScopeCounts,
    gap_scope: GapScope,
    knows: bool,
) -> Option<String> {
    if !knows {
        return None;
    }
    if scope == GapScope::Local && gap_scope == GapScope::Global {
        return Some(
            "Too little local data to tell directions: your gap is judged on the global market."
                .into(),
        );
    }
    let unclear = counts
        .skills
        .iter()
        .all(|s| s.direction == Direction::Unclear);
    (!counts.skills.is_empty() && unclear)
        .then(|| "Too few postings yet to tell which way a skill is going.".into())
}

/// The user's own leads next to the market, as one sentence: how many, and the skills they name
/// most. They are never counted in the market's numbers.
pub fn pipeline_note(pipeline: &ScopeCounts) -> String {
    let documents: u32 = pipeline.documents.iter().sum();
    let top: Vec<&str> = pipeline
        .skills
        .iter()
        .take(3)
        .map(|s| s.skill.as_str())
        .collect();
    let named = if top.is_empty() {
        String::new()
    } else {
        format!("; they name {} most", top.join(", "))
    };
    format!("Your own leads, for comparison: {documents} over the window{named}. They are not counted in the market above.")
}

/// The heading of a section. A report saved before the market scan wrote its `for_you` points about
/// the user's wishes, not the gap: it keeps the old title.
pub fn section_title(section: Section, knows: bool) -> &'static str {
    match section {
        Section::ForYou if !knows => "What it means for you",
        _ => section.title(),
    }
}

/// The buckets of the gap, in display order.
pub const BUCKETS: [GapBucket; 3] = [GapBucket::BuildOn, GapBucket::LearnNext, GapBucket::AtRisk];

/// `build_on` / `learn_next` / `at_risk`: the stable part of the element ids.
pub fn bucket_key(bucket: GapBucket) -> &'static str {
    match bucket {
        GapBucket::BuildOn => "build_on",
        GapBucket::LearnNext => "learn_next",
        GapBucket::AtRisk => "at_risk",
    }
}

/// The title of a bucket, what it means, and what an empty one says.
pub fn bucket_words(bucket: GapBucket) -> (&'static str, &'static str, &'static str) {
    match bucket {
        GapBucket::BuildOn => (
            "Build on",
            "In your profile, and rising or steadily asked for.",
            "No skill of your profile is in steady demand here yet.",
        ),
        GapBucket::LearnNext => (
            "Learn next",
            "Not in your profile, and rising or often asked for in your field.",
            "Nothing your field asks for is missing from your profile.",
        ),
        GapBucket::AtRisk => (
            "At risk",
            "In your profile, and asked for less and less.",
            "No skill of your profile is falling.",
        ),
    }
}

/// Which market the gap and the wishes were judged on, in words.
pub fn judged_on(scope: GapScope, place: &str, field: &[String]) -> String {
    let market = match (scope, place.is_empty()) {
        (GapScope::Local, false) => format!("postings in {place}"),
        (GapScope::Local, true) => "local postings".to_string(),
        (GapScope::Global, _) => "all postings".to_string(),
    };
    if field.is_empty() {
        format!("Judged by the app on {market}.")
    } else {
        format!("Judged by the app on {market} for {}.", field.join(" "))
    }
}

/// `skill` / `domain`.
pub fn wish_kind(kind: WishKind) -> &'static str {
    match kind {
        WishKind::Skill => "skill",
        WishKind::Domain => "domain",
    }
}

/// What the market says about one wish: its direction's tag ("no data" when no posting names
/// it), and the count behind it.
pub fn wish_signal(wish: &WishSignal) -> ((&'static str, Tone), String) {
    if wish.mentions == 0 {
        return (("no data", Tone::Neutral), "no posting names it".into());
    }
    let postings = if wish.mentions == 1 {
        "posting"
    } else {
        "postings"
    };
    (
        direction_tag(wish.direction),
        format!("{} {postings}", wish.mentions),
    )
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
    Accent,
    Ok,
    Warn,
    Danger,
}

impl Tone {
    pub fn as_str(self) -> &'static str {
        match self {
            Tone::Neutral => "neutral",
            Tone::Accent => "accent",
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
            // The only kind without an adapter is `report_search`, in runs of older versions.
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
            "{} point(s) were left out: no valid source, a quote that is not in the source, or a claim against the app's numbers.",
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
    use jobpilot_engine::store::{Source, SourcePurpose};
    use jobpilot_engine::trends::{
        Period, Section, SkillCount, SourceOutcome, SourceStatus, TrendRun,
    };

    use jobpilot_engine::trends::digest::{
        Direction, GapBucket, GapScope, ScopeCounts, SkillCounts, WishKind, WishSignal,
        MARKET_SCAN_VERSION,
    };

    use super::{
        bucket_key, bucket_words, describe, describe_source, direction_tag, header_summary,
        is_web_url, judged_on, knows_background, latest_month_is_partial, month_label, percent,
        percent_text, period_title, permille, pipeline_note, scope_note, section_title, status_tag,
        summarize, target, wish_signal, Cadence, Field, Preferences, Presence, Tone, BUCKETS,
    };

    fn period() -> Period {
        Period::parse("2026-W46").unwrap()
    }

    fn skill(by_month: &[u32]) -> SkillCount {
        SkillCount {
            skill: "Rust".into(),
            by_month: by_month.to_vec(),
            total: by_month.iter().sum(),
            ..SkillCount::default()
        }
    }

    const DOT: &str = "\u{b7}";

    // ---- periods

    #[test]
    fn a_period_is_named_for_the_header() {
        assert_eq!(period_title(period()), "Week of 9 November");
        assert_eq!(period_title(Period::parse("2026-Q4").unwrap()), "Q4 2026");
        assert_eq!(period_title(Period::parse("2026-M11").unwrap()), "November 2026");
        assert_eq!(period_title(Period::parse("2026").unwrap()), "2026");
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
        assert_eq!(Cadence::Monthly.period(today).label(), "2026-M11");
        assert_eq!(Cadence::Yearly.period(today).label(), "2026");
        for cadence in [Cadence::Weekly, Cadence::Monthly, Cadence::Quarterly, Cadence::Yearly] {
            assert_eq!(Cadence::parse(cadence.as_str()), Some(cadence));
            assert_eq!(Cadence::of(cadence.period(today)), cadence);
        }
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

    // ---- market numbers

    #[test]
    fn shares_are_per_mille_of_the_documents_and_read_as_percents() {
        assert_eq!(permille(30, 200), 150);
        assert_eq!(permille(1, 3), 333);
        assert_eq!(permille(2, 3), 667, "rounded");
        assert_eq!(permille(5, 0), 0, "no documents, no share");
        assert_eq!(permille(u32::MAX, 1), u32::MAX, "never overflows");
        assert_eq!(
            (percent(150), percent(25), percent(4), percent(0)),
            (15, 3, 0, 0)
        );
        assert_eq!(percent_text(182), "18%");
        assert_eq!(percent_text(95), "9.5%");
        assert_eq!(percent_text(1), "0.1%");
        assert_eq!(percent_text(0), "0%");
        assert_eq!(percent_text(1000), "100%");
    }

    #[test]
    fn a_report_knows_the_background_only_when_it_carries_any_of_it() {
        let mut counts = SkillCounts {
            skills: vec![skill(&[1, 2])],
            ..SkillCounts::default()
        };
        assert!(!knows_background(&counts), "a report saved before the market scan");
        let thin = SkillCounts {
            version: MARKET_SCAN_VERSION,
            ..SkillCounts::default()
        };
        assert!(
            knows_background(&thin),
            "a new report with thin data is still new"
        );
        assert_eq!(
            section_title(jobpilot_engine::trends::Section::ForYou, false),
            "What it means for you"
        );
        counts.version = MARKET_SCAN_VERSION;
        assert!(knows_background(&counts));
        assert_eq!(
            section_title(jobpilot_engine::trends::Section::ForYou, true),
            "Your gap: build on, learn next, at risk"
        );
        let placed = SkillCounts {
            local: Some(ScopeCounts::default()),
            ..SkillCounts::default()
        };
        assert!(
            !knows_background(&placed),
            "the version decides, not the fields"
        );
    }

    #[test]
    fn directions_and_wishes_read_as_tags() {
        assert_eq!(direction_tag(Direction::Rising), ("rising", Tone::Ok));
        assert_eq!(direction_tag(Direction::Steady), ("steady", Tone::Accent));
        assert_eq!(direction_tag(Direction::Falling), ("falling", Tone::Warn));
        assert_eq!(
            direction_tag(Direction::Unclear),
            ("unclear", Tone::Neutral)
        );
        let wish = |direction, mentions| WishSignal {
            wish: "Rust".into(),
            kind: WishKind::Skill,
            avoided: false,
            direction,
            mentions,
        };
        assert_eq!(
            wish_signal(&wish(Direction::Rising, 42)),
            (("rising", Tone::Ok), "42 postings".to_string())
        );
        assert_eq!(wish_signal(&wish(Direction::Steady, 1)).1, "1 posting");
        assert_eq!(
            wish_signal(&wish(Direction::Rising, 0)),
            (
                ("no data", Tone::Neutral),
                "no posting names it".to_string()
            ),
            "no mention is no data, whatever the direction says"
        );
    }

    #[test]
    fn a_scope_says_when_its_data_is_too_thin() {
        let unclear = ScopeCounts {
            documents: vec![5, 5],
            skills: vec![skill(&[1, 2])],
        };
        assert!(
            scope_note(GapScope::Local, &unclear, GapScope::Global, true)
                .is_some_and(|n| n.starts_with("Too little local data"))
        );
        assert!(
            scope_note(GapScope::Global, &unclear, GapScope::Global, true)
                .is_some_and(|n| n.starts_with("Too few postings"))
        );
        assert_eq!(
            scope_note(GapScope::Global, &unclear, GapScope::Global, false),
            None,
            "an old report says nothing"
        );
        let mut clear = unclear.clone();
        clear.skills[0].direction = Direction::Rising;
        assert_eq!(
            scope_note(GapScope::Local, &clear, GapScope::Local, true),
            None
        );
        assert_eq!(
            scope_note(
                GapScope::Global,
                &ScopeCounts::default(),
                GapScope::Global,
                true
            ),
            None
        );
    }

    #[test]
    fn months_gap_buckets_and_the_pipeline_read_in_words() {
        assert_eq!(month_label("2026-06"), "Jun");
        assert_eq!(month_label("2026-12"), "Dec");
        assert_eq!(month_label("2026-13"), "2026-13");
        assert_eq!(month_label("June"), "June");
        assert_eq!(
            BUCKETS.map(bucket_key),
            ["build_on", "learn_next", "at_risk"]
        );
        assert_eq!(
            BUCKETS.map(|b| bucket_words(b).0),
            ["Build on", "Learn next", "At risk"]
        );
        assert_eq!(
            bucket_words(GapBucket::AtRisk).2,
            "No skill of your profile is falling."
        );
        assert_eq!(
            judged_on(
                GapScope::Local,
                "Singapore or remote",
                &["backend".into(), "engineer".into()]
            ),
            "Judged by the app on postings in Singapore or remote for backend engineer."
        );
        assert_eq!(
            judged_on(GapScope::Global, "Singapore", &[]),
            "Judged by the app on all postings."
        );
        assert_eq!(
            judged_on(GapScope::Local, "", &[]),
            "Judged by the app on local postings."
        );
        let pipeline = ScopeCounts {
            documents: vec![2, 3],
            skills: Vec::new(),
        };
        assert_eq!(
            pipeline_note(&pipeline),
            "Your own leads, for comparison: 5 over the window. They are not counted in the market above."
        );
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
            must_skills: (0..jobpilot_engine::prefs::chat::MAX_LIST_ITEMS)
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
