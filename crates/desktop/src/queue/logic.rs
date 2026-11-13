//! Pure logic of the queue screen (no GPUI, easy to test): the one primary button, score and time
//! labels, source status lines.

use std::collections::HashMap;

use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::discover::promote::Dismissed;
use jobpilot_core::store::Source;

/// The queue's primary action.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum QueueAction {
    PrepareCv,
    SearchNow,
}

impl QueueAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::PrepareCv => "Prepare CV",
            Self::SearchNow => "Search now",
        }
    }
}

/// What the queue's primary button looks like right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueuePrimary {
    pub action: QueueAction,
    pub enabled: bool,
    /// Work is running: spinner on the button.
    pub loading: bool,
    /// Why the button is disabled, or what it will do.
    pub hint: Option<String>,
}

/// What `queue_primary_for` needs to know.
#[derive(Clone, Copy, Debug, Default)]
pub struct QueueFacts {
    /// New leads in the queue, before the filters.
    pub total: usize,
    /// Leads that pass the filters.
    pub shown: usize,
    /// Leads "Prepare CV" would act on: the ticked ones, else the open one.
    pub targets: usize,
    /// Of those, the ones not already waiting for or getting a CV.
    pub idle_targets: usize,
    /// A search is running.
    pub searching: bool,
    /// Enabled job sources.
    pub sources: usize,
}

/// The one primary button of the queue: "Prepare CV" while there are leads, "Search now" when the
/// queue is empty. Disabled while the work it starts runs. Every other button on the screen is plain.
pub fn queue_primary_for(f: &QueueFacts) -> QueuePrimary {
    let ready = |action, hint: Option<String>| QueuePrimary { action, enabled: true, loading: false, hint };
    let blocked = |action, hint: &str| QueuePrimary { action, enabled: false, loading: false, hint: Some(hint.to_string()) };
    if f.total == 0 {
        return if f.searching {
            QueuePrimary { action: QueueAction::SearchNow, enabled: false, loading: true, hint: Some("Searching your sources".into()) }
        } else if f.sources == 0 {
            blocked(QueueAction::SearchNow, "No job source yet: add one in Settings, then search.")
        } else {
            ready(QueueAction::SearchNow, Some("The queue is empty.".into()))
        };
    }
    if f.targets == 0 {
        let hint = if f.shown == 0 { "No lead matches the filters." } else { "Select a lead to prepare its CV." };
        return blocked(QueueAction::PrepareCv, hint);
    }
    if f.idle_targets == 0 {
        return QueuePrimary { action: QueueAction::PrepareCv, enabled: false, loading: true, hint: None };
    }
    let hint = (f.targets > 1).then(|| format!("{} leads selected, their CVs are written one after the other", f.targets));
    ready(QueueAction::PrepareCv, hint)
}

/// A score for a row: `88`, or a dash for a lead that was never scored.
pub fn score_label(score: Option<f64>) -> String {
    score.map_or_else(|| "\u{2013}".to_string(), |s| format!("{s:.0}"))
}

/// How good a score is, for its colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    High,
    Mid,
    Low,
    Unknown,
}

pub fn score_tone(score: Option<f64>) -> Tone {
    match score {
        Some(s) if s >= 75.0 => Tone::High,
        Some(s) if s >= 50.0 => Tone::Mid,
        Some(_) => Tone::Low,
        None => Tone::Unknown,
    }
}

/// `just now`, `5 min ago`, `3 h ago`, `2 d ago`.
pub fn ago(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let minutes = (now - then).num_minutes();
    match minutes {
        m if m < 1 => "just now".to_string(),
        m if m < 60 => format!("{m} min ago"),
        m if m < 60 * 24 => format!("{} h ago", m / 60),
        m => format!("{} d ago", m / (60 * 24)),
    }
}

/// The source's name, or its kind when it has none.
pub fn source_name(s: &Source) -> String {
    match s.name.trim() {
        "" => s.kind.clone(),
        name => name.to_string(),
    }
}

/// How the last run of a source ended, in words: `not searched yet`, `ok, 5 min ago`, `needs you`.
pub fn source_status(s: &Source, now: DateTime<Utc>) -> String {
    let status = match s.last_status.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        None => return "not searched yet".to_string(),
        Some("needs_human") => "needs you".to_string(),
        Some(other) => other.replace('_', " "),
    };
    match s.last_run_at {
        Some(at) => format!("{status}, {}", ago(now, at)),
        None => status,
    }
}

/// Does this status line need the user's attention (colour it)?
pub fn source_needs_attention(s: &Source) -> bool {
    s.last_status.as_deref().is_some_and(|t| t == "needs_human" || t.starts_with("failed"))
}

/// A link the system browser may open: http(s) only.
pub fn is_web_link(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("https://") || lower.starts_with("http://")
}

/// The line under a lead's name: location and source.
pub fn meta_line(location: &str, source: Option<&str>) -> String {
    let location = location.trim();
    let parts: Vec<&str> = [(!location.is_empty()).then_some(location), source].into_iter().flatten().collect();
    if parts.is_empty() {
        "location not stated".to_string()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// What to tell the user after "Dismiss".
pub fn dismissed_note(out: &Dismissed) -> String {
    if let Some(why) = &out.not_excluded {
        return format!("Lead dismissed, but the company could not be excluded: {why}");
    }
    let more = |n: usize| if n == 1 { "1 more job".to_string() } else { format!("{n} more jobs") };
    let mut text = match out.also {
        0 => "Lead dismissed.".to_string(),
        n => format!("Lead dismissed, and {} from the same company.", more(n)),
    };
    if out.kept > 0 {
        let verb = if out.kept == 1 { "was" } else { "were" };
        text.push_str(&format!(" {} from that company {verb} kept: the CV is being prepared.", more(out.kept)));
    }
    text
}

/// A "Prepare CV" batch whose results stopped coming (the task ended early): its jobs that never
/// reported leave the queue of running jobs and become failures the user can retry.
pub fn drop_unreported<J>(jobs: &mut HashMap<i64, J>, failed: &mut HashMap<i64, String>, unreported: impl IntoIterator<Item = i64>) {
    for id in unreported {
        if jobs.remove(&id).is_some() {
            failed.insert(id, "Preparing the CV stopped before it finished. Try again.".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::chrono::{Duration, TimeZone, Utc};
    use jobpilot_core::store::{Source, SourcePurpose};

    use super::*;

    fn facts(f: impl FnOnce(&mut QueueFacts)) -> QueueFacts {
        let mut facts = QueueFacts { total: 5, shown: 5, targets: 1, idle_targets: 1, searching: false, sources: 2 };
        f(&mut facts);
        facts
    }

    #[test]
    fn the_dismiss_note_says_what_else_went_and_what_was_kept() {
        let note = |also, kept| dismissed_note(&Dismissed { also, kept, not_excluded: None });
        assert_eq!(note(0, 0), "Lead dismissed.");
        assert_eq!(note(1, 0), "Lead dismissed, and 1 more job from the same company.");
        assert_eq!(note(3, 0), "Lead dismissed, and 3 more jobs from the same company.");
        assert_eq!(note(0, 1), "Lead dismissed. 1 more job from that company was kept: the CV is being prepared.");
        let failed = Dismissed { not_excluded: Some("disk full".into()), ..Dismissed::default() };
        assert_eq!(dismissed_note(&failed), "Lead dismissed, but the company could not be excluded: disk full");
        assert_eq!(
            note(2, 2),
            "Lead dismissed, and 2 more jobs from the same company. 2 more jobs from that company were kept: the CV is being prepared."
        );
    }

    #[test]
    fn jobs_that_never_reported_are_cleared_and_can_be_retried() {
        let mut jobs: HashMap<i64, &str> = HashMap::from([(1, "running"), (2, "queued"), (3, "queued"), (9, "other batch")]);
        let mut failed = HashMap::new();
        // Lead 1 reported (and left `jobs` then); 2 and 3 never did; 1 is not in the list of unreported ones.
        jobs.remove(&1);
        drop_unreported(&mut jobs, &mut failed, [2, 3]);
        assert_eq!(jobs.keys().copied().collect::<Vec<_>>(), [9], "only this batch's jobs are cleared");
        assert_eq!(failed.keys().copied().collect::<std::collections::BTreeSet<_>>(), [2, 3].into());
        assert!(failed[&2].contains("Try again"));
        drop_unreported(&mut jobs, &mut failed, [2]);
        assert_eq!(failed.len(), 2, "nothing to clear twice");
    }

    #[test]
    fn the_primary_is_prepare_cv_while_there_are_leads() {
        let p = queue_primary_for(&facts(|_| {}));
        assert_eq!((p.action, p.enabled, p.loading, p.hint), (QueueAction::PrepareCv, true, false, None));
        assert_eq!(p.action.label(), "Prepare CV");

        let many = queue_primary_for(&facts(|f| (f.targets, f.idle_targets) = (3, 3)));
        assert!(many.enabled && many.hint.unwrap().contains("3 leads"));
        // Some targets are already queued: the button still works for the rest.
        assert!(queue_primary_for(&facts(|f| (f.targets, f.idle_targets) = (3, 1))).enabled);
        // A search running elsewhere never blocks it.
        assert!(queue_primary_for(&facts(|f| f.searching = true)).enabled);
    }

    #[test]
    fn it_waits_for_a_selection_and_is_disabled_while_the_work_runs() {
        let none = queue_primary_for(&facts(|f| (f.targets, f.idle_targets) = (0, 0)));
        assert_eq!((none.action, none.enabled), (QueueAction::PrepareCv, false));
        assert!(none.hint.unwrap().contains("Select a lead"));

        let hidden = queue_primary_for(&facts(|f| (f.shown, f.targets, f.idle_targets) = (0, 0, 0)));
        assert!(!hidden.enabled && hidden.hint.unwrap().contains("filters"));

        let running = queue_primary_for(&facts(|f| f.idle_targets = 0));
        assert_eq!((running.action, running.enabled, running.loading), (QueueAction::PrepareCv, false, true));
    }

    #[test]
    fn an_empty_queue_offers_search_now() {
        let empty = |f: &mut QueueFacts| (f.total, f.shown, f.targets, f.idle_targets) = (0, 0, 0, 0);
        let p = queue_primary_for(&facts(empty));
        assert_eq!((p.action, p.enabled, p.loading), (QueueAction::SearchNow, true, false));
        assert_eq!(p.action.label(), "Search now");

        let searching = queue_primary_for(&facts(|f| {
            empty(f);
            f.searching = true;
        }));
        assert_eq!((searching.action, searching.enabled, searching.loading), (QueueAction::SearchNow, false, true));

        let no_sources = queue_primary_for(&facts(|f| {
            empty(f);
            f.sources = 0;
        }));
        assert_eq!((no_sources.action, no_sources.enabled), (QueueAction::SearchNow, false));
        assert!(no_sources.hint.unwrap().contains("Settings"));
    }

    #[test]
    fn scores_and_tones() {
        assert_eq!(score_label(Some(87.6)), "88");
        assert_eq!(score_label(Some(41.4)), "41");
        assert_eq!(score_label(None), "\u{2013}");
        assert_eq!(score_tone(Some(75.0)), Tone::High);
        assert_eq!(score_tone(Some(74.9)), Tone::Mid);
        assert_eq!(score_tone(Some(50.0)), Tone::Mid);
        assert_eq!(score_tone(Some(10.0)), Tone::Low);
        assert_eq!(score_tone(None), Tone::Unknown);
    }

    #[test]
    fn times_read_naturally() {
        let now = Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap();
        assert_eq!(ago(now, now - Duration::seconds(20)), "just now");
        assert_eq!(ago(now, now - Duration::minutes(5)), "5 min ago");
        assert_eq!(ago(now, now - Duration::minutes(130)), "2 h ago");
        assert_eq!(ago(now, now - Duration::days(3)), "3 d ago");
        assert_eq!(ago(now, now + Duration::minutes(5)), "just now", "a clock a little ahead is not the past");
    }

    fn source(name: &str, status: Option<&str>, ran: bool) -> Source {
        let now = Utc.with_ymd_and_hms(2026, 11, 14, 11, 0, 0).unwrap();
        Source {
            id: 1,
            kind: "greenhouse".into(),
            name: name.into(),
            url: String::new(),
            purpose: SourcePurpose::Jobs,
            enabled: true,
            session_id: None,
            last_run_at: ran.then_some(now),
            last_status: status.map(str::to_string),
        }
    }

    #[test]
    fn a_source_line_says_how_the_last_search_went() {
        let now = Utc.with_ymd_and_hms(2026, 11, 14, 12, 0, 0).unwrap();
        assert_eq!(source_name(&source("Acme careers", None, false)), "Acme careers");
        assert_eq!(source_name(&source("  ", None, false)), "greenhouse");
        assert_eq!(source_status(&source("a", None, false), now), "not searched yet");
        assert_eq!(source_status(&source("a", Some("ok"), true), now), "ok, 1 h ago");
        assert_eq!(source_status(&source("a", Some("needs_human"), false), now), "needs you");
        assert_eq!(source_status(&source("a", Some("paused for today: repeated challenges"), true), now), "paused for today: repeated challenges, 1 h ago");
        assert!(source_needs_attention(&source("a", Some("needs_human"), false)));
        assert!(source_needs_attention(&source("a", Some("failed: boom"), true)));
        assert!(!source_needs_attention(&source("a", Some("ok"), true)));
        assert!(!source_needs_attention(&source("a", None, false)));
    }

    #[test]
    fn links_and_meta_lines() {
        assert!(is_web_link("https://a.test/jobs/1") && is_web_link(" HTTP://a.test"));
        assert!(!is_web_link("file:///etc/passwd") && !is_web_link("javascript:alert(1)") && !is_web_link(""));
        assert_eq!(meta_line("Singapore", Some("Acme careers")), "Singapore \u{b7} Acme careers");
        assert_eq!(meta_line("", Some("HN")), "HN");
        assert_eq!(meta_line(" Remote ", None), "Remote");
        assert_eq!(meta_line("", None), "location not stated");
    }
}
