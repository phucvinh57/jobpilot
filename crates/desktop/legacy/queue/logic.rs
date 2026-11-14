//! Pure logic of the Job list screen (the queue) (no GPUI, easy to test): the one primary button, score and time
//! labels, source status lines.

use std::collections::{HashMap, VecDeque};

use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::discover::promote::Dismissed;
use jobpilot_core::store::Source;

/// Most postings that may wait behind the one being read (links pasted or files dropped meanwhile).
pub const MAX_WAITING: usize = 10;

/// What became of an input put behind the posting that is being read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Queued {
    /// At the end of the queue.
    Waiting,
    /// The same input is being read or already waits: nothing added.
    Duplicate,
    /// [`MAX_WAITING`] are waiting: nothing added.
    Full,
}

/// Put `item` at the end of `queue`, unless it is the `running` one or one already waiting (a
/// Retry or a second paste of the same link would read the same posting twice), or
/// [`MAX_WAITING`] wait already.
pub fn enqueue<T: PartialEq>(running: Option<&T>, queue: &mut VecDeque<T>, item: T) -> Queued {
    if running == Some(&item) || queue.contains(&item) {
        return Queued::Duplicate;
    }
    if queue.len() >= MAX_WAITING {
        return Queued::Full;
    }
    queue.push_back(item);
    Queued::Waiting
}

/// Most failures kept under the link box: the oldest fall off.
pub const MAX_FAILURES: usize = 5;

/// What the link box says when a posting is pasted that is being read or already waits.
pub const ALREADY_READING: &str = "Already reading this posting";

/// An input (link, file, text) that did not become a lead, with what Retry runs again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddFailure<T> {
    pub message: String,
    /// What Retry runs again; `None` for a refusal with nothing to retry (a drop that was refused,
    /// a full queue).
    pub input: Option<T>,
}

/// The failures under the link box, the newest first. An input has at most one (a new failure of
/// the same input replaces the old one), so a batch keeps what went wrong with each of its items.
#[derive(Debug)]
pub struct AddFailures<T> {
    items: Vec<AddFailure<T>>,
}

impl<T> Default for AddFailures<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T: PartialEq> AddFailures<T> {
    /// Show `failure` first. It replaces the one of the same input (or, without an input, the one
    /// with the same message); at most [`MAX_FAILURES`] are kept.
    pub fn push(&mut self, failure: AddFailure<T>) {
        self.items.retain(|f| match (&f.input, &failure.input) {
            (Some(a), Some(b)) => a != b,
            (None, None) => f.message != failure.message,
            _ => true,
        });
        self.items.insert(0, failure);
        self.items.truncate(MAX_FAILURES);
    }

    /// Forget the failure of `input` (it is being read again, or was read): the others stay.
    pub fn clear_input(&mut self, input: &T) {
        self.items.retain(|f| f.input.as_ref() != Some(input));
    }

    /// Forget them all; whether there was any.
    pub fn clear(&mut self) -> bool {
        let any = !self.items.is_empty();
        self.items.clear();
        any
    }

    /// The newest first.
    pub fn iter(&self) -> impl Iterator<Item = &AddFailure<T>> {
        self.items.iter()
    }
}

/// Put `input` behind the posting being read (`running`), and keep the failures honest about it:
/// the failure of `input` goes only when it is really queued again; a duplicate or a full queue
/// leaves every failure as it was (a full queue adds its own, with nothing to retry).
pub fn queue_behind<T: PartialEq + Clone>(
    running: Option<&T>,
    waiting: &mut VecDeque<T>,
    failures: &mut AddFailures<T>,
    input: T,
) -> Queued {
    let queued = enqueue(running, waiting, input.clone());
    match queued {
        Queued::Waiting => failures.clear_input(&input),
        Queued::Duplicate => {}
        Queued::Full => failures.push(AddFailure {
            message: format!("{MAX_WAITING} postings are already waiting: add this one when they have been read."),
            input: None,
        }),
    }
    queued
}

/// "3 waiting" next to the spinner; `None` when nothing waits.
pub fn waiting_label(waiting: usize) -> Option<String> {
    (waiting > 0).then(|| format!("{waiting} waiting"))
}

/// The Job list's primary action.
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

/// What the Job list's primary button looks like right now.
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
    /// New leads in the Job list, before the filters.
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

/// The one primary button of the Job list: "Prepare CV" while there are leads, "Search now" when the
/// list is empty. Disabled while the work it starts runs. Every other button on the screen is plain.
pub fn queue_primary_for(f: &QueueFacts) -> QueuePrimary {
    let ready = |action, hint: Option<String>| QueuePrimary { action, enabled: true, loading: false, hint };
    let blocked = |action, hint: &str| QueuePrimary { action, enabled: false, loading: false, hint: Some(hint.to_string()) };
    if f.total == 0 {
        return if f.searching {
            QueuePrimary { action: QueueAction::SearchNow, enabled: false, loading: true, hint: Some("Searching your sources".into()) }
        } else if f.sources == 0 {
            blocked(QueueAction::SearchNow, "No job source yet: add one in Settings, then search.")
        } else {
            ready(QueueAction::SearchNow, Some("No jobs yet.".into()))
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
/// reported leave the running ones and become failures the user can retry.
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

    #[test]
    fn postings_added_while_one_is_read_wait_in_order_up_to_a_limit() {
        let mut queue = std::collections::VecDeque::new();
        assert_eq!(waiting_label(queue.len()), None, "nothing waits, nothing is said");
        for n in 0..MAX_WAITING {
            assert_eq!(enqueue(None, &mut queue, n), Queued::Waiting);
        }
        assert_eq!(enqueue(None, &mut queue, 99), Queued::Full, "a full queue refuses the next one");
        assert_eq!(queue.len(), MAX_WAITING);
        assert_eq!(queue.pop_front(), Some(0), "first in, first read");
        assert_eq!(queue.pop_front(), Some(1));
        assert_eq!(enqueue(None, &mut queue, 99), Queued::Waiting, "room again after one is taken");
        assert_eq!(queue.back(), Some(&99));
        assert_eq!(waiting_label(1).as_deref(), Some("1 waiting"));
        assert_eq!(waiting_label(queue.len()).as_deref(), Some("9 waiting"));
    }

    #[test]
    fn an_input_already_running_or_waiting_is_not_queued_again() {
        let mut queue = std::collections::VecDeque::new();
        assert_eq!(enqueue(Some(&"a"), &mut queue, "a"), Queued::Duplicate, "the one being read");
        assert_eq!(enqueue(Some(&"a"), &mut queue, "b"), Queued::Waiting);
        assert_eq!(enqueue(Some(&"a"), &mut queue, "b"), Queued::Duplicate, "the one already waiting");
        assert_eq!(enqueue(None, &mut queue, "a"), Queued::Waiting, "nothing is running: it is a new one");
        assert_eq!(queue, ["b", "a"]);
        // A duplicate is a duplicate even when the queue is full.
        let mut full: std::collections::VecDeque<usize> = (0..MAX_WAITING).collect();
        assert_eq!(enqueue(None, &mut full, 3), Queued::Duplicate);
        assert_eq!(full.len(), MAX_WAITING);
    }

    fn failure(message: &str, input: Option<&'static str>) -> AddFailure<&'static str> {
        AddFailure { message: message.into(), input }
    }

    fn messages<'a>(failures: &'a AddFailures<&'static str>) -> Vec<&'a str> {
        failures.iter().map(|f| f.message.as_str()).collect()
    }

    #[test]
    fn failures_are_kept_per_input_newest_first_and_each_is_cleared_on_its_own() {
        let mut failures = AddFailures::default();
        assert_eq!(failures.iter().count(), 0);
        failures.push(failure("a failed", Some("a")));
        failures.push(failure("b failed", Some("b")));
        failures.push(failure("drop refused", None));
        assert_eq!(messages(&failures), ["drop refused", "b failed", "a failed"], "a batch keeps every failure, newest first");

        // Reading `a` again clears its failure only.
        failures.clear_input(&"a");
        assert_eq!(messages(&failures), ["drop refused", "b failed"]);
        failures.clear_input(&"c");
        assert_eq!(failures.iter().count(), 2, "an input with no failure changes nothing");

        // The same input failing again replaces its old message and moves to the front.
        failures.push(failure("b failed again", Some("b")));
        assert_eq!(messages(&failures), ["b failed again", "drop refused"]);
        // Without an input, the same message is shown once.
        failures.push(failure("drop refused", None));
        assert_eq!(messages(&failures), ["drop refused", "b failed again"]);

        assert!(failures.clear());
        assert_eq!(failures.iter().count(), 0);
        assert!(!failures.clear(), "nothing to clear");
    }

    #[test]
    fn a_failure_goes_only_when_its_input_is_really_queued_again() {
        let mut waiting = VecDeque::new();
        let mut failures = AddFailures::default();
        failures.push(failure("a failed", Some("a")));
        failures.push(failure("b failed", Some("b")));

        // `b` is being read: asking for `b` again changes nothing, and `a` keeps its failure.
        assert_eq!(queue_behind(Some(&"b"), &mut waiting, &mut failures, "b"), Queued::Duplicate);
        assert_eq!(messages(&failures), ["b failed", "a failed"], "a duplicate clears nothing");
        assert!(waiting.is_empty());

        // `a`'s Retry queues it behind `b`: its failure goes, `b`'s stays.
        assert_eq!(queue_behind(Some(&"b"), &mut waiting, &mut failures, "a"), Queued::Waiting);
        assert_eq!(messages(&failures), ["b failed"]);
        // `a` again while it waits: a duplicate.
        assert_eq!(queue_behind(Some(&"b"), &mut waiting, &mut failures, "a"), Queued::Duplicate);

        // A full queue refuses and says so, keeping the failure of the refused input.
        let mut waiting: VecDeque<&str> = ["w1", "w2", "w3", "w4", "w5", "w6", "w7", "w8", "w9", "w10"].into();
        assert_eq!(waiting.len(), MAX_WAITING);
        assert_eq!(queue_behind(Some(&"x"), &mut waiting, &mut failures, "b"), Queued::Full);
        assert_eq!(failures.iter().count(), 2);
        assert_eq!(failures.iter().next().map(|f| f.input), Some(None), "the refusal is the newest line");
        assert!(failures.iter().any(|f| f.input == Some("b")), "b still has its own failure and Retry");
    }

    #[test]
    fn only_the_newest_failures_are_kept() {
        let mut failures = AddFailures::default();
        let inputs = ["a", "b", "c", "d", "e", "f", "g"];
        for input in inputs {
            failures.push(failure(input, Some(input)));
        }
        assert_eq!(failures.iter().count(), MAX_FAILURES);
        assert_eq!(failures.iter().next().map(|f| f.message.as_str()), Some("g"));
        assert_eq!(failures.iter().last().map(|f| f.message.as_str()), Some("c"));
    }

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
    fn an_empty_job_list_offers_search_now() {
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
