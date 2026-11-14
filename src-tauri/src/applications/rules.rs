//! Pure rules of this context, unit-tested here: the stages of the Jobs page, the Application screen's
//! one primary action and stepper, the timeline's wording, the CV checklist and the Apply tab's
//! review rows. No I/O, no Tauri.

use std::collections::HashMap;

use crate::engine::apply::{Edit, FieldKind, Review, Row, SessionState, Source};
use crate::engine::browser::SessionStatus;
use chrono::{NaiveDate, NaiveDateTime};
use crate::engine::pipeline::{Analysis, Progress};
use crate::engine::store::{clean_round, Application, Email, Event, Status};
use serde::Serialize;

/// What a status or a tag means; the web app maps each to its colours.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Tone {
    Neutral,
    Accent,
    Warn,
    Ok,
    Danger,
}

// ---------------------------------------------------------------- Jobs page

/// The stages of the Jobs page that hold applications, left to right. The page's first stage,
/// Leads, comes from discovery: a lead becomes an application (stage `Cv`) on Prepare CV.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Cv,
    Applied,
    InProgress,
    Done,
}

impl Stage {
    pub const ALL: [Stage; 4] = [Stage::Cv, Stage::Applied, Stage::InProgress, Stage::Done];

    pub fn label(self) -> &'static str {
        match self {
            Self::Cv => "CVs",
            Self::Applied => "Applied",
            Self::InProgress => "In progress",
            Self::Done => "Done",
        }
    }

    /// Stable identifier for the web app (`cv`, `applied`, `inProgress`, `done`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Cv => "cv",
            Self::Applied => "applied",
            Self::InProgress => "inProgress",
            Self::Done => "done",
        }
    }
}

/// The stage a status belongs in. Every status has one: until the application is sent it is about
/// its CV (a failed submit too: the primary button there is "Review & Submit" again); an interview
/// is in progress; an answer (offer or rejection) is done.
pub fn stage_for(status: Status) -> Stage {
    match status {
        Status::Draft | Status::CvReady | Status::Submitting | Status::Failed => Stage::Cv,
        Status::Applied => Stage::Applied,
        Status::Interview => Stage::InProgress,
        Status::Offer | Status::Rejected => Stage::Done,
    }
}

/// All stages in order (empty ones too), each with its applications in the order of `apps`
/// (the store lists the most recently updated first).
pub fn board(apps: &[Application]) -> Vec<(Stage, Vec<&Application>)> {
    Stage::ALL
        .into_iter()
        .map(|s| (s, apps.iter().filter(|a| stage_for(a.status) == s).collect()))
        .collect()
}

/// How a finished application ended: an offer is a success, a rejection a failure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    Success,
    Failed,
}

impl Outcome {
    /// Stable identifier for the web app (`success`, `failed`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failed => "failed",
        }
    }
}

/// The outcome of a status: an offer is a success, a rejection a failure; anything else is open.
pub fn outcome_for(status: Status) -> Option<Outcome> {
    match status {
        Status::Offer => Some(Outcome::Success),
        Status::Rejected => Some(Outcome::Failed),
        _ => None,
    }
}

/// Rounds offered when the company brief does not list the interview process.
pub const DEFAULT_ROUNDS: [&str; 4] = ["HR screen", "Technical interview", "System design", "Final interview"];

/// Most rounds offered.
pub const MAX_ROUNDS: usize = 8;

/// The rounds the user can mark: the brief's interview steps in order, each cleaned as the store
/// keeps a round (`clean_round`: trimmed, cut to `ROUND_MAX_CHARS`, so a saved chip shows as selected),
/// without the offer step, without repeats (ignoring case) and at most [`MAX_ROUNDS`];
/// [`DEFAULT_ROUNDS`] when that leaves none.
pub fn round_options(insight_steps: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for step in insight_steps {
        let Some(step) = clean_round(Some(step)) else {
            continue;
        };
        let lower = step.to_lowercase();
        if lower.contains("offer") || out.iter().any(|o| o.to_lowercase() == lower) {
            continue;
        }
        out.push(step);
        if out.len() == MAX_ROUNDS {
            break;
        }
    }
    if out.is_empty() {
        out = DEFAULT_ROUNDS.iter().map(|s| s.to_string()).collect();
    }
    out
}

/// What the user can mark by hand once the application was sent: the company's answers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mark {
    Interview,
    Offer,
    Rejected,
}

impl Mark {
    pub const ALL: [Mark; 3] = [Mark::Interview, Mark::Offer, Mark::Rejected];

    /// Stable identifier for the web app (`interview`, `offer`, `rejected`).
    pub fn key(self) -> &'static str {
        match self {
            Self::Interview => "interview",
            Self::Offer => "offer",
            Self::Rejected => "rejected",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Interview => "Got an interview",
            Self::Offer => "Offer",
            Self::Rejected => "Rejected",
        }
    }

    pub fn parse(key: &str) -> Option<Mark> {
        Mark::ALL.into_iter().find(|m| m.key() == key)
    }

    /// The status the mark moves the application to.
    pub fn status(self) -> Status {
        match self {
            Self::Interview => Status::Interview,
            Self::Offer => Status::Offer,
            Self::Rejected => Status::Rejected,
        }
    }
}

/// The marks offered for a status: only a sent application (Applied) or one in interviews can be
/// marked, and only forward (`Status::can_advance_to`).
pub fn marks_for(status: Status) -> Vec<Mark> {
    if !matches!(status, Status::Applied | Status::Interview) {
        return Vec::new();
    }
    Mark::ALL
        .into_iter()
        .filter(|m| status.can_advance_to(m.status()))
        .collect()
}

/// What the user does next with an application, as the card's tag says it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NextAction {
    pub label: String,
    pub tone: Tone,
}

fn next(label: &str, tone: Tone) -> NextAction {
    NextAction { label: label.to_string(), tone }
}

/// The tag of a card. It follows the Application screen's one primary button (Generate -> Approve
/// CV -> Review & Submit) while the application is open, and says where it stands once it is not
/// (in interviews: the round the user marked): `busy` = a generate / revise job is running for it,
/// `has_cv` = its `cv.yaml` exists.
pub fn next_action(status: Status, has_cv: bool, busy: bool, round: Option<&str>) -> NextAction {
    if busy {
        return next(if has_cv { "Working" } else { "Generating" }, Tone::Neutral);
    }
    match status {
        Status::Draft if !has_cv => next("Generate CV", Tone::Accent),
        Status::Draft => next("Approve CV", Tone::Accent),
        Status::CvReady | Status::Submitting => next("Review & submit", Tone::Accent),
        Status::Failed => next("Submit failed", Tone::Danger),
        Status::Applied => next("No reply yet", Tone::Neutral),
        Status::Interview => match round.map(str::trim).filter(|r| !r.is_empty()) {
            Some(round) => next(round, Tone::Warn),
            None => next("Check your email", Tone::Warn),
        },
        Status::Offer => next("Offer", Tone::Ok),
        Status::Rejected => next("Rejected", Tone::Danger),
    }
}

/// The title and subtitle of a card: the role and the company, whichever the analysis found.
pub fn card_title(role: &str, company: &str) -> (String, String) {
    match (role.trim(), company.trim()) {
        ("", "") => ("Untitled".to_string(), String::new()),
        ("", company) => (company.to_string(), String::new()),
        (role, company) => (role.to_string(), company.to_string()),
    }
}

/// How long ago a card last changed: `today`, `1 d`, `12 d`.
pub fn age_label(today: NaiveDate, then: NaiveDate) -> String {
    match (today - then).num_days() {
        d if d <= 0 => "today".to_string(),
        d => format!("{d} d"),
    }
}

/// When an event happened: the time today, `Yesterday`, the weekday within the last week, else the date.
pub fn activity_when(now: NaiveDateTime, then: NaiveDateTime) -> String {
    match (now.date() - then.date()).num_days() {
        d if d <= 0 => then.format("%H:%M").to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => then.format("%a").to_string(),
        _ => then.format("%d %b").to_string(),
    }
}

/// The colour of an event's dot: errors red, mail and warnings amber, status moves blue.
pub fn activity_tone(kind: &str) -> Tone {
    match kind {
        "error" => Tone::Danger,
        "email" | "email_review" | "warning" => Tone::Warn,
        "status" | "round" => Tone::Accent,
        _ => Tone::Neutral,
    }
}

/// An event's text for people: a stored status move (`cv_ready -> applied: note`) reads
/// "moved to Applied: note"; anything else is shown as stored. Links are cut to host and path
/// ([`short_urls`]).
pub fn activity_text(kind: &str, message: &str) -> String {
    short_urls(&status_words(kind, message))
}

fn status_words(kind: &str, message: &str) -> String {
    if kind != "status" {
        return message.to_string();
    }
    let Some((_, rest)) = message.split_once(" -> ") else {
        return message.to_string();
    };
    let (to, note) = rest
        .split_once(": ")
        .map_or((rest, None), |(t, n)| (t, Some(n)));
    match (Status::parse(to.trim()), note) {
        (Some(st), Some(n)) => format!("moved to {}: {n}", st.label()),
        (Some(st), None) => format!("moved to {}", st.label()),
        (None, _) => message.to_string(),
    }
}

/// Every `http(s)://` link in `text` as host and path only: no scheme, query or fragment (a link in
/// an error can carry a session or anti-bot token), no trailing `/`. Punctuation that ends a sentence
/// right after a link stays outside it.
pub fn short_urls(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len());
    let mut rest = 0;
    while let Some(start) = next_link(&lower, rest) {
        out.push_str(&text[rest..start]);
        let end = text[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`' | ')' | ']' | '}'))
            .map_or(text.len(), |n| start + n);
        let link = &text[start..end];
        let trimmed = link.trim_end_matches(['.', ',', ';', ':', '!']);
        let tail = &link[trimmed.len()..];
        let without_scheme = trimmed.split_once("://").map_or(trimmed, |(_, r)| r);
        let bare = without_scheme
            .split(['?', '#'])
            .next()
            .unwrap_or_default()
            .trim_end_matches('/');
        out.push_str(bare);
        out.push_str(tail);
        rest = end;
    }
    out.push_str(&text[rest..]);
    out
}

/// Where the next `http://` or `https://` link of `lower` (an ASCII-lowercased text) starts, from `from` on.
fn next_link(lower: &str, from: usize) -> Option<usize> {
    let tail = &lower[from..];
    [tail.find("https://"), tail.find("http://")]
        .into_iter()
        .flatten()
        .min()
        .map(|n| from + n)
}

// ---------------------------------------------------------------- Application screen

/// The single primary button: its label follows the status and the live apply state
/// (Generate -> Approve CV -> Review & Submit -> Submit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimaryAction {
    Generate,
    ApproveCv,
    ReviewAndSubmit,
    /// The form of the next step is open (multi-step forms): fill it, then continue. Nothing is sent yet.
    Continue,
    Submit,
}

impl PrimaryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generate => "Generate",
            Self::ApproveCv => "Approve CV",
            Self::ReviewAndSubmit => "Review & Submit",
            Self::Continue => "Continue",
            Self::Submit => "Submit",
        }
    }

    /// Stable identifier for the web app.
    pub fn key(self) -> &'static str {
        match self {
            Self::Generate => "generate",
            Self::ApproveCv => "approveCv",
            Self::ReviewAndSubmit => "reviewAndSubmit",
            Self::Continue => "continue",
            Self::Submit => "submit",
        }
    }
}

/// What the primary button looks like right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Primary {
    pub action: PrimaryAction,
    pub enabled: bool,
    /// Work is running: spinner on the button.
    pub loading: bool,
    /// Why the button is disabled, or what is left to do.
    pub hint: Option<String>,
    /// The Settings section that fixes what blocks the button, if there is one.
    pub settings: Option<&'static str>,
}

impl Primary {
    fn ready(action: PrimaryAction) -> Self {
        Self {
            action,
            enabled: true,
            loading: false,
            hint: None,
            settings: None,
        }
    }

    fn blocked(action: PrimaryAction, hint: impl Into<String>) -> Self {
        Self {
            action,
            enabled: false,
            loading: false,
            hint: Some(hint.into()),
            settings: None,
        }
    }

    fn working(action: PrimaryAction) -> Self {
        Self {
            action,
            enabled: false,
            loading: true,
            hint: None,
            settings: None,
        }
    }
}

/// What the Apply tab is doing for one application, as the primary button needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApplyUi {
    /// Nothing open: the primary button opens and fills the form.
    Idle,
    /// Opening / filling the form, submitting, or closing it: the primary button waits.
    Working {
        submitting: bool,
    },
    /// The form is filled and open for review.
    Review {
        complete: bool,
        dry_run: bool,
        missing: usize,
    },
    /// Stopped for a human (failure, CAPTCHA, unsupported step).
    Handoff,
    Done,
}

/// Why the form cannot be opened, and which Settings section helps.
pub struct Blocker {
    pub message: String,
    pub settings: Option<&'static str>,
}

/// The one primary action for an application, from its status and the live state of the Apply tab.
/// `None` = read-only (already applied, or answered by the company).
///
/// An open or running apply flow wins over the status, so the button always matches what the Apply tab
/// shows. `Submit` stays disabled until every required field has a value; every button is disabled while
/// work runs. `blocker` is only asked when the form could be opened (no apply URL, no session, no Chrome).
pub fn primary_for(
    status: Status,
    has_cv: bool,
    cv_broken: bool,
    job_running: bool,
    apply: &ApplyUi,
    blocker: &dyn Fn() -> Option<Blocker>,
) -> Option<Primary> {
    match apply {
        ApplyUi::Working { submitting } => {
            let action = if *submitting {
                PrimaryAction::Submit
            } else {
                PrimaryAction::ReviewAndSubmit
            };
            return Some(Primary::working(action));
        }
        ApplyUi::Review {
            complete,
            dry_run,
            missing,
        } => {
            let action = if *complete {
                PrimaryAction::Submit
            } else {
                PrimaryAction::Continue
            };
            return Some(if *complete && *dry_run {
                Primary::blocked(action, "Dry run: the form is filled but nothing is sent. Close the dry run to start over.")
            } else if *missing > 0 {
                Primary::blocked(action, format!("{missing} required field(s) still empty"))
            } else {
                Primary::ready(action)
            });
        }
        ApplyUi::Idle | ApplyUi::Handoff | ApplyUi::Done => {}
    }
    match status {
        Status::Applied | Status::Interview | Status::Rejected | Status::Offer => None,
        Status::Draft if !has_cv => Some(if job_running {
            Primary::working(PrimaryAction::Generate)
        } else {
            Primary::ready(PrimaryAction::Generate)
        }),
        Status::Draft => Some(if job_running {
            Primary::working(PrimaryAction::ApproveCv)
        } else if cv_broken {
            Primary::blocked(
                PrimaryAction::ApproveCv,
                "The CV has an error: fix it before approving",
            )
        } else {
            Primary::ready(PrimaryAction::ApproveCv)
        }),
        Status::CvReady | Status::Failed | Status::Submitting => Some(if job_running {
            Primary::working(PrimaryAction::ReviewAndSubmit)
        } else {
            match blocker() {
                Some(b) => Primary {
                    settings: b.settings,
                    ..Primary::blocked(PrimaryAction::ReviewAndSubmit, b.message)
                },
                None => Primary::ready(PrimaryAction::ReviewAndSubmit),
            }
        }),
    }
}

/// After the application was sent (or while its confirmation shows), a plain "Next job" button
/// keeps the user moving: it opens the best lead left on Today.
pub fn next_job_offered(status: Status, apply: &ApplyUi) -> bool {
    matches!(apply, ApplyUi::Done) || is_sent(status)
}

/// The application was sent: it is read-only now.
pub fn is_sent(status: Status) -> bool {
    matches!(
        status,
        Status::Applied | Status::Interview | Status::Rejected | Status::Offer
    )
}

/// The steps of the progress bar under the header.
pub const STEPS: [&str; 5] = ["Lead", "CV review", "Apply", "Submitted", "Replies"];

/// The step in progress for a status: the index into [`STEPS`], or `STEPS.len()` when every step is done.
/// A draft without a CV is still a lead; with a CV it waits for review; an approved CV (and a failed or
/// running submit) is at Apply; a sent application waits for replies; any answer closes the bar.
pub fn step_index(status: Status, has_cv: bool) -> usize {
    match status {
        Status::Draft if !has_cv => 0,
        Status::Draft => 1,
        Status::CvReady | Status::Submitting | Status::Failed => 2,
        Status::Applied => 4,
        Status::Interview | Status::Offer | Status::Rejected => STEPS.len(),
    }
}

/// The tag colour of a status.
pub fn status_tone(status: Status) -> Tone {
    match status {
        Status::Draft => Tone::Neutral,
        Status::CvReady => Tone::Accent,
        Status::Submitting => Tone::Warn,
        Status::Applied | Status::Interview | Status::Offer => Tone::Ok,
        Status::Rejected | Status::Failed => Tone::Danger,
    }
}

/// The line under the title: company, then the location when the analysis found one.
pub fn subtitle(company: &str, analysis: Option<&Analysis>) -> String {
    let location = analysis
        .map(|a| a.location.trim())
        .filter(|l| !l.is_empty());
    match location {
        Some(location) => format!("{company} \u{b7} {location}"),
        None => company.to_string(),
    }
}

/// One line for the header: `Tailoring the CV` or `Tailoring the CV: repairing 2 problem(s)`.
pub fn progress_label(p: &Progress) -> String {
    if p.detail.is_empty() {
        p.stage.label().to_string()
    } else {
        format!("{}: {}", p.stage.label(), p.detail)
    }
}

// ---------------------------------------------------------------- Timeline

/// Longest event text shown on its row; the full text is the row's title.
pub const ROW_CHARS: usize = 110;

/// Look of one timeline line, derived from the event's free-form `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventStyle {
    Created,
    Status,
    /// The user marked an interview round.
    Round,
    Cv,
    Mail,
    Review,
    Warning,
    Error,
    Note,
}

impl EventStyle {
    pub fn of(kind: &str) -> Self {
        match kind {
            "created" => Self::Created,
            "status" => Self::Status,
            "round" => Self::Round,
            "cv" => Self::Cv,
            "email" => Self::Mail,
            "email_review" => Self::Review,
            "warning" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Note,
        }
    }

    /// The colour of the row's dot.
    pub fn tone(self) -> Tone {
        match self {
            Self::Created | Self::Note => Tone::Neutral,
            Self::Status | Self::Round | Self::Cv | Self::Mail => Tone::Accent,
            Self::Review | Self::Warning => Tone::Warn,
            Self::Error => Tone::Danger,
        }
    }
}

/// What a row shows of an event text: its first non-empty line, cut at [`ROW_CHARS`]; `true` when something was left out.
pub fn row_text(message: &str) -> (String, bool) {
    let first = message
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.chars().count() > ROW_CHARS {
        let cut: String = first.chars().take(ROW_CHARS).collect();
        return (format!("{}...", cut.trim_end()), true);
    }
    (first.to_string(), first.len() != message.trim().len())
}

/// Newest first (the store returns oldest first).
pub fn newest_first(mut events: Vec<Event>) -> Vec<Event> {
    events.reverse();
    events
}

/// One line for a pending email review: `Rejection (60% sure): Subject`.
pub fn review_title(email: &Email) -> String {
    let kind = email.kind.as_deref().unwrap_or("other");
    let kind = kind
        .chars()
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + &kind[c.len_utf8()..])
        .unwrap_or_default();
    match email.confidence {
        Some(c) => format!("{kind} ({:.0}% sure): {}", c * 100.0, email.subject),
        None => format!("{kind}: {}", email.subject),
    }
}

/// The body of a pending email review: who wrote it and what the classifier made of it.
pub fn review_body(email: &Email) -> String {
    format!(
        "From {}{}",
        email.from_addr,
        email
            .summary
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!(" - {s}"))
            .unwrap_or_default()
    )
}

// ---------------------------------------------------------------- CV checklist

/// One line of the "What they ask for" checklist.
#[derive(Debug, PartialEq, Eq)]
pub struct Requirement {
    pub text: String,
    /// From the nice-to-have list (not required).
    pub nice: bool,
    /// The gap line that says the profile has no evidence for it; `None` = covered.
    pub gap: Option<String>,
}

impl Requirement {
    /// The note under an uncovered requirement.
    pub fn note(&self) -> Option<String> {
        self.gap.as_ref().map(|g| {
            if g.eq_ignore_ascii_case(&self.text) {
                "Nothing in profile.yaml shows it".to_string()
            } else {
                g.clone()
            }
        })
    }
}

/// The words of a text, lower case; `+` and `#` stay (C++, C#).
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '+' || c == '#'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// What a gap line is about: the part before the first colon (`Kafka: nothing shows it` -> `Kafka`).
fn gap_subject(gap: &str) -> &str {
    gap.split(':').next().unwrap_or(gap)
}

/// Do a requirement and a gap line talk about the same thing? (all the words of one are in the other,
/// comparing the requirement with the line's subject)
pub fn same_topic(requirement: &str, gap: &str) -> bool {
    let (a, b) = (words(requirement), words(gap_subject(gap)));
    !a.is_empty()
        && !b.is_empty()
        && (a.iter().all(|w| b.contains(w)) || b.iter().all(|w| a.contains(w)))
}

/// The JD's requirements (must have, then nice to have) with the gaps the tailoring step reported:
/// a requirement is covered unless a gap line is about it. Gap lines that match no requirement are
/// listed too, so none is lost. Empty when there is nothing to list.
pub fn requirements(analysis: Option<&Analysis>, gaps: &[String]) -> Vec<Requirement> {
    let mut rows: Vec<Requirement> = Vec::new();
    if let Some(a) = analysis {
        let lists = [(&a.must_have, false), (&a.nice_to_have, true)];
        for (list, nice) in lists {
            for text in list.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                if rows.iter().any(|r| r.text.eq_ignore_ascii_case(text)) {
                    continue;
                }
                let gap = gaps.iter().find(|g| same_topic(text, g)).cloned();
                rows.push(Requirement {
                    text: text.to_string(),
                    nice,
                    gap,
                });
            }
        }
    }
    for gap in gaps {
        let listed = rows
            .iter()
            .any(|r| r.gap.as_deref() == Some(gap.as_str()) || same_topic(&r.text, gap));
        if !listed {
            rows.push(Requirement {
                text: gap.clone(),
                nice: false,
                gap: Some(gap.clone()),
            });
        }
    }
    rows
}

/// `gaps_json` as a list of "JD wants X, no evidence in the profile" lines (empty when absent or invalid).
pub fn parse_gaps(json: Option<&str>) -> Vec<String> {
    json.and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect()
}

// ---------------------------------------------------------------- Apply tab

/// Choice fields with at most this many options are shown as buttons; longer lists are typed.
pub const MAX_CHIPS: usize = 8;

/// How one review row is edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldInput {
    /// Not editable on this step, or a file: shown as text.
    ReadOnly,
    Checkbox,
    /// One of a few options, as buttons.
    Chips,
    /// Several of a few options, as buttons.
    MultiChips,
    Area,
    Line,
}

impl FieldInput {
    pub fn key(self) -> &'static str {
        match self {
            Self::ReadOnly => "readonly",
            Self::Checkbox => "checkbox",
            Self::Chips => "chips",
            Self::MultiChips => "multiChips",
            Self::Area => "area",
            Self::Line => "line",
        }
    }
}

/// Does a choice field show its options as buttons?
pub fn uses_chips(row: &Row) -> bool {
    matches!(
        row.kind,
        FieldKind::Select | FieldKind::Radio | FieldKind::CheckboxGroup | FieldKind::Combobox
    ) && !row.options.is_empty()
        && row.options.len() <= MAX_CHIPS
}

/// The editor a row gets.
pub fn field_input(row: &Row) -> FieldInput {
    if !row.editable || row.kind == FieldKind::File {
        FieldInput::ReadOnly
    } else if row.kind == FieldKind::Checkbox {
        FieldInput::Checkbox
    } else if uses_chips(row) {
        if row.kind == FieldKind::CheckboxGroup {
            FieldInput::MultiChips
        } else {
            FieldInput::Chips
        }
    } else if row.kind == FieldKind::Textarea || row.value.chars().count() > 90 {
        FieldInput::Area
    } else {
        FieldInput::Line
    }
}

/// The hint inside an empty single-line field.
pub fn placeholder(row: &Row) -> String {
    if row.options.is_empty() {
        "Your answer".to_string()
    } else {
        format!("One of: {}", row.options.join(" | "))
    }
}

/// The value a row has now: what the user typed, else what the form was filled with.
pub fn current_value<'a>(row: &'a Row, typed: &'a HashMap<String, String>) -> &'a str {
    typed
        .get(&row.id)
        .map_or(row.value.as_str(), String::as_str)
}

/// Required rows that are still empty, from what is typed right now.
pub fn missing_count(review: &Review, typed: &HashMap<String, String>) -> usize {
    review
        .rows
        .iter()
        .filter(|r| r.editable && r.required && r.kind != FieldKind::File)
        .filter(|r| current_value(r, typed).trim().is_empty())
        .count()
}

/// Current value of every editable row, as edits for the core.
pub fn edits_for(
    review: &Review,
    typed: &HashMap<String, String>,
    remember: &std::collections::HashSet<String>,
) -> Vec<Edit> {
    review
        .rows
        .iter()
        .filter(|r| r.editable)
        .map(|r| Edit {
            id: r.id.clone(),
            value: current_value(r, typed).to_string(),
            remember: remember.contains(&r.id),
        })
        .collect()
}

/// The tag of a row: where its answer came from.
pub fn source_tag(row: &Row) -> (String, Tone) {
    match (row.needs_input, row.source) {
        (true, _) | (_, Source::Unknown) => ("needs input".to_string(), Tone::Danger),
        (_, Source::Profile) => ("profile".to_string(), Tone::Ok),
        (_, Source::Generated) => ("generated".to_string(), Tone::Warn),
        (_, s) => (s.as_str().to_string(), Tone::Neutral),
    }
}

/// The last part of a path, as shown.
pub fn file_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

/// The host of an address, as shown.
pub fn host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

/// The session line of the idle Apply tab and its colour.
pub fn session_line(session: &SessionState) -> (String, Tone) {
    let line = match session {
        SessionState::NotNeeded => {
            "No login needed: runs in a throwaway browser profile.".to_string()
        }
        SessionState::Matched { name, status, .. } => match status {
            SessionStatus::Ok => format!("Session: {name} (logged in)"),
            SessionStatus::New => {
                format!("Session: {name} (not verified yet: check it in Settings)")
            }
            SessionStatus::Expired => {
                format!("Session: {name} (expired: log in again in Settings)")
            }
        },
        SessionState::Missing { domain } => {
            format!("No browser session for {domain}: create one in Settings > Browser sessions.")
        }
    };
    let tone = match session {
        SessionState::Missing { .. } => Tone::Danger,
        SessionState::Matched {
            status: SessionStatus::Expired,
            ..
        } => Tone::Warn,
        _ => Tone::Neutral,
    };
    (line, tone)
}

/// The banner above a review: a dry run, more steps to go, or required fields still empty.
pub fn review_notice(review: &Review, missing: usize) -> Option<(Tone, &'static str, String)> {
    if review.dry_run {
        Some((
            Tone::Accent,
            "Dry run",
            "The form is filled but will never be submitted.".to_string(),
        ))
    } else if !review.complete {
        Some((
            Tone::Accent,
            "More steps",
            "This form has more steps. Fill the highlighted fields, then press Continue (top right). Nothing is submitted yet.".to_string(),
        ))
    } else if missing > 0 {
        Some((
            Tone::Warn,
            "Required fields empty",
            format!(
                "{missing} required field(s) still empty: fill them to enable the Submit button."
            ),
        ))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::apply::Ats;
    use chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    use crate::engine::pipeline::{Analysis, Stage as JobStage};

    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("a valid date")
    }

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        day(d).and_hms_opt(h, m, 0).expect("a valid time")
    }

    // ------------------------------------------------------------ Jobs page

    #[test]
    fn every_status_has_a_stage() {
        for (status, stage) in [
            (Status::Draft, Stage::Cv),
            (Status::CvReady, Stage::Cv),
            (Status::Submitting, Stage::Cv),
            (Status::Failed, Stage::Cv),
            (Status::Applied, Stage::Applied),
            (Status::Interview, Stage::InProgress),
            (Status::Offer, Stage::Done),
            (Status::Rejected, Stage::Done),
        ] {
            assert_eq!(stage_for(status), stage, "{status:?}");
        }
        // The table above must list every status, so a new one cannot be forgotten.
        assert_eq!(Status::ALL.len(), 8);
        for status in Status::ALL {
            assert!(Stage::ALL.contains(&stage_for(status)), "{status:?}");
        }
    }

    #[test]
    fn stages_run_from_cvs_to_done() {
        assert_eq!(Stage::ALL.map(Stage::label), ["CVs", "Applied", "In progress", "Done"]);
        assert_eq!(Stage::ALL.map(Stage::key), ["cv", "applied", "inProgress", "done"]);
    }

    #[test]
    fn only_an_answer_has_an_outcome() {
        assert_eq!(outcome_for(Status::Offer), Some(Outcome::Success));
        assert_eq!(outcome_for(Status::Rejected), Some(Outcome::Failed));
        for status in Status::ALL {
            if !matches!(status, Status::Offer | Status::Rejected) {
                assert_eq!(outcome_for(status), None, "{status:?}");
            }
        }
        assert_eq!((Outcome::Success.key(), Outcome::Failed.key()), ("success", "failed"));
    }

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn rounds_come_from_the_brief_without_the_offer() {
        assert_eq!(
            round_options(&strings(&["Recruiter call", " Technical screen ", "recruiter CALL", "", "Offer", "Job offer & references"])),
            strings(&["Recruiter call", "Technical screen"])
        );
        let many: Vec<String> = (1..=12).map(|n| format!("Round {n}")).collect();
        assert_eq!(round_options(&many), many[..MAX_ROUNDS].to_vec());
    }

    #[test]
    fn a_long_round_is_offered_as_the_store_keeps_it() {
        use crate::engine::store::ROUND_MAX_CHARS;
        let long = format!("  {}  ", "Pair programming with the platform team ".repeat(3));
        let options = round_options(&strings(&[&long, "HR screen"]));
        let kept = clean_round(Some(&long));
        assert_eq!(options.first(), kept.as_ref());
        assert!(options[0].chars().count() <= ROUND_MAX_CHARS);
        assert_eq!(options[1], "HR screen");
    }

    #[test]
    fn without_a_process_the_usual_rounds_are_offered() {
        assert_eq!(round_options(&[]), strings(&DEFAULT_ROUNDS));
        assert_eq!(round_options(&strings(&["Offer", "  "])), strings(&DEFAULT_ROUNDS));
    }

    #[test]
    fn only_a_sent_application_can_be_marked_and_only_forward() {
        assert_eq!(marks_for(Status::Applied), [Mark::Interview, Mark::Offer, Mark::Rejected]);
        assert_eq!(marks_for(Status::Interview), [Mark::Offer, Mark::Rejected]);
        for status in [Status::Draft, Status::CvReady, Status::Submitting, Status::Failed, Status::Offer, Status::Rejected] {
            assert!(marks_for(status).is_empty(), "{status:?}");
        }
        // Every mark offered is a move the store accepts.
        for status in Status::ALL {
            for mark in marks_for(status) {
                assert!(status.can_advance_to(mark.status()), "{status:?} -> {mark:?}");
            }
        }
    }

    #[test]
    fn marks_have_keys_and_labels() {
        assert_eq!(Mark::ALL.map(Mark::key), ["interview", "offer", "rejected"]);
        assert_eq!(Mark::ALL.map(Mark::label), ["Got an interview", "Offer", "Rejected"]);
        assert_eq!(Mark::parse("offer"), Some(Mark::Offer));
        assert_eq!(Mark::parse("Offer"), None);
        assert_eq!(Mark::parse("applied"), None);
        assert_eq!(Mark::Interview.status(), Status::Interview);
    }

    fn app(id: i64, status: Status) -> Application {
        let moment = Utc
            .with_ymd_and_hms(2026, 11, 1, 9, 0, 0)
            .single()
            .expect("a valid moment");
        Application {
            id,
            slug: format!("a{id}"),
            company: format!("Company {id}"),
            role: "Engineer".into(),
            source: None,
            apply_url: None,
            status,
            dir: Default::default(),
            analysis_json: None,
            gaps_json: None,
            created_at: moment,
            updated_at: moment,
            insight_json: None,
            interview_round: None,
        }
    }

    #[test]
    fn the_board_lists_all_stages_and_keeps_the_order() {
        let apps = vec![
            app(1, Status::Applied),
            app(2, Status::Draft),
            app(3, Status::Applied),
            app(4, Status::Offer),
            app(5, Status::Rejected),
            app(6, Status::Failed),
        ];
        let ids = |rows: &[&Application]| rows.iter().map(|a| a.id).collect::<Vec<_>>();
        let stages = board(&apps);
        assert_eq!(stages.iter().map(|(s, _)| *s).collect::<Vec<_>>(), Stage::ALL);
        assert_eq!(ids(&stages[0].1), [2, 6]);
        assert_eq!(ids(&stages[1].1), [1, 3], "the list's order is kept inside a stage");
        assert!(stages[2].1.is_empty(), "an empty stage is still there");
        assert_eq!(ids(&stages[3].1), [4, 5]);
        assert!(board(&[]).iter().all(|(_, rows)| rows.is_empty()));
    }

    #[test]
    fn the_tag_is_the_next_action() {
        let label = |status, has_cv| next_action(status, has_cv, false, None).label;
        assert_eq!(label(Status::Draft, false), "Generate CV");
        assert_eq!(label(Status::Draft, true), "Approve CV");
        assert_eq!(label(Status::CvReady, true), "Review & submit");
        assert_eq!(label(Status::Submitting, true), "Review & submit");
        assert_eq!(label(Status::Failed, true), "Submit failed");
        assert_eq!(label(Status::Applied, true), "No reply yet");
        assert_eq!(label(Status::Interview, true), "Check your email");
        assert_eq!(label(Status::Offer, true), "Offer");
        assert_eq!(label(Status::Rejected, true), "Rejected");
        // Every status says something, with a tone that fits.
        for status in Status::ALL {
            assert!(!next_action(status, true, false, None).label.is_empty(), "{status:?}");
        }
        assert_eq!(next_action(Status::Failed, true, false, None).tone, Tone::Danger);
        assert_eq!(next_action(Status::Interview, true, false, None).tone, Tone::Warn);
        assert_eq!(next_action(Status::Offer, true, false, None).tone, Tone::Ok);
        // A failed outcome reads red.
        assert_eq!(next_action(Status::Rejected, true, false, None).tone, Tone::Danger);
    }

    #[test]
    fn an_interview_card_shows_the_marked_round() {
        let tag = next_action(Status::Interview, true, false, Some("Technical interview"));
        assert_eq!((tag.label.as_str(), tag.tone), ("Technical interview", Tone::Warn));
        assert_eq!(next_action(Status::Interview, true, false, Some("  ")).label, "Check your email");
        // A round kept from the interviews does not hide the answer.
        assert_eq!(next_action(Status::Offer, true, false, Some("Final interview")).label, "Offer");
        assert_eq!(next_action(Status::Rejected, true, false, Some("HR screen")).label, "Rejected");
    }

    #[test]
    fn running_work_wins_over_the_status() {
        assert_eq!(next_action(Status::Draft, false, true, None).label, "Generating");
        assert_eq!(next_action(Status::Draft, true, true, None).label, "Working");
        assert_eq!(next_action(Status::CvReady, true, true, None).label, "Working");
        assert_eq!(next_action(Status::Draft, false, true, None).tone, Tone::Neutral);
    }

    #[test]
    fn a_card_is_titled_by_its_role_then_its_company() {
        assert_eq!(
            card_title("Engineer", "Acme"),
            ("Engineer".to_string(), "Acme".to_string())
        );
        assert_eq!(card_title(" ", "Acme"), ("Acme".to_string(), String::new()));
        assert_eq!(card_title("", ""), ("Untitled".to_string(), String::new()));
    }

    #[test]
    fn ages_count_whole_days() {
        assert_eq!(age_label(day(14), day(14)), "today");
        assert_eq!(age_label(day(14), day(13)), "1 d");
        assert_eq!(age_label(day(14), day(2)), "12 d");
        assert_eq!(
            age_label(day(14), day(15)),
            "today",
            "a clock a little ahead is not negative"
        );
    }

    #[test]
    fn events_are_dated_for_a_glance() {
        let now = at(14, 18, 0);
        assert_eq!(activity_when(now, at(14, 9, 5)), "09:05");
        assert_eq!(activity_when(now, at(13, 23, 59)), "Yesterday");
        // 2026-11-12 is a Thursday, 2026-11-09 a Monday.
        assert_eq!(activity_when(now, at(12, 8, 0)), "Thu");
        assert_eq!(activity_when(now, at(9, 8, 0)), "Mon");
        assert_eq!(
            activity_when(now, at(8, 8, 0)),
            "Sun",
            "six days ago still has a weekday"
        );
        assert_eq!(activity_when(now, at(7, 8, 0)), "07 Nov");
    }

    #[test]
    fn events_have_a_tone() {
        assert_eq!(activity_tone("error"), Tone::Danger);
        assert_eq!(activity_tone("email"), Tone::Warn);
        assert_eq!(activity_tone("email_review"), Tone::Warn);
        assert_eq!(activity_tone("status"), Tone::Accent);
        assert_eq!(activity_tone("round"), Tone::Accent);
        assert_eq!(activity_tone("created"), Tone::Neutral);
        assert_eq!(activity_tone("anything else"), Tone::Neutral);
    }

    #[test]
    fn a_status_move_reads_as_words() {
        assert_eq!(
            activity_text("status", "draft -> cv_ready: demo data"),
            "moved to CV ready: demo data"
        );
        assert_eq!(
            activity_text("status", "applied -> interview"),
            "moved to Interview"
        );
        assert_eq!(activity_text("status", "odd text"), "odd text");
        assert_eq!(activity_text("email", "a -> b"), "a -> b");
        assert_eq!(activity_text("round", "Round: HR screen"), "Round: HR screen");
    }

    #[test]
    fn activity_links_lose_their_query() {
        assert_eq!(
            activity_text(
                "error",
                "fetching https://grab.careers/jobs/123/?afd_azwaf_tok=eyJhbGciOi.secret#top failed: 403"
            ),
            "fetching grab.careers/jobs/123 failed: 403"
        );
        assert_eq!(
            activity_text("status", "applied -> interview: see https://acme.test/x?t=1."),
            "moved to Interview: see acme.test/x."
        );
    }

    #[test]
    fn links_are_shown_as_host_and_path() {
        assert_eq!(short_urls("no link here"), "no link here");
        assert_eq!(short_urls("https://a.test"), "a.test");
        assert_eq!(short_urls("HTTPS://A.test/Path/?q=1"), "A.test/Path");
        assert_eq!(short_urls("http://a.test/p#frag, then"), "a.test/p, then");
        assert_eq!(
            short_urls("(https://a.test/x?y=1) and \"http://b.test/?z\""),
            "(a.test/x) and \"b.test\""
        );
        assert_eq!(short_urls("two: https://a.test/1?x https://b.test/2?y"), "two: a.test/1 b.test/2");
        assert_eq!(short_urls("caf\u{e9} https://a.test/\u{e9}?t=1 \u{2192} ok"), "caf\u{e9} a.test/\u{e9} \u{2192} ok");
        assert_eq!(short_urls("ftp://a.test/x?y"), "ftp://a.test/x?y", "only web links");
    }

    // ------------------------------------------------------------ Application screen

    fn none() -> Option<Blocker> {
        None
    }

    fn idle(status: Status, has_cv: bool) -> Option<Primary> {
        primary_for(status, has_cv, false, false, &ApplyUi::Idle, &none)
    }

    #[test]
    fn the_primary_action_follows_the_status() {
        assert_eq!(
            idle(Status::Draft, false).map(|p| p.action),
            Some(PrimaryAction::Generate)
        );
        assert_eq!(
            idle(Status::Draft, true).map(|p| p.action),
            Some(PrimaryAction::ApproveCv)
        );
        assert_eq!(
            idle(Status::CvReady, true).map(|p| p.action),
            Some(PrimaryAction::ReviewAndSubmit)
        );
        assert_eq!(
            idle(Status::Failed, true).map(|p| p.action),
            Some(PrimaryAction::ReviewAndSubmit)
        );
        for status in [
            Status::Applied,
            Status::Interview,
            Status::Rejected,
            Status::Offer,
        ] {
            assert_eq!(idle(status, true), None, "{status:?} is read-only");
        }
    }

    #[test]
    fn next_job_is_offered_once_the_application_was_sent() {
        for status in [
            Status::Applied,
            Status::Interview,
            Status::Rejected,
            Status::Offer,
        ] {
            assert!(next_job_offered(status, &ApplyUi::Idle), "{status:?}");
            assert_eq!(
                idle(status, true),
                None,
                "it replaces the primary button, never joins it"
            );
        }
        // Right after Submit, while the confirmation shows.
        assert!(next_job_offered(Status::Submitting, &ApplyUi::Done));
        for status in [
            Status::Draft,
            Status::CvReady,
            Status::Failed,
            Status::Submitting,
        ] {
            assert!(
                !next_job_offered(status, &ApplyUi::Idle),
                "{status:?} still has work to do"
            );
            assert!(!next_job_offered(status, &ApplyUi::Handoff));
        }
    }

    fn enabled(p: Option<Primary>) -> Option<(PrimaryAction, bool)> {
        p.map(|p| (p.action, p.enabled))
    }

    #[test]
    fn an_open_review_decides_the_button_whatever_the_status() {
        let review = |complete, dry_run, missing| ApplyUi::Review {
            complete,
            dry_run,
            missing,
        };
        // The demo state: a Draft without CV with a review open must not offer Generate.
        let p = primary_for(
            Status::Draft,
            false,
            false,
            false,
            &review(true, false, 0),
            &none,
        );
        assert_eq!(enabled(p), Some((PrimaryAction::Submit, true)));
        for status in [Status::CvReady, Status::Submitting, Status::Failed] {
            let p = primary_for(status, true, false, false, &review(true, false, 0), &none);
            assert_eq!(
                enabled(p),
                Some((PrimaryAction::Submit, true)),
                "{status:?}"
            );
        }
        // Required fields empty: Submit stays disabled and says how many.
        let p = primary_for(
            Status::Submitting,
            true,
            false,
            false,
            &review(true, false, 2),
            &none,
        );
        assert_eq!(enabled(p.clone()), Some((PrimaryAction::Submit, false)));
        assert!(p.and_then(|p| p.hint).is_some_and(|h| h.contains('2')));
        // A dry run can be reviewed but never submitted.
        let p = primary_for(
            Status::CvReady,
            true,
            false,
            false,
            &review(true, true, 0),
            &none,
        );
        assert_eq!(enabled(p), Some((PrimaryAction::Submit, false)));
        // More steps to go: Continue, enabled once the highlighted fields are filled.
        let p = primary_for(
            Status::Submitting,
            true,
            false,
            false,
            &review(false, false, 0),
            &none,
        );
        assert_eq!(enabled(p), Some((PrimaryAction::Continue, true)));
    }

    #[test]
    fn the_button_is_disabled_while_work_runs() {
        let state = |p: Option<Primary>| p.map(|p| (p.action, p.enabled, p.loading));
        let working = ApplyUi::Working { submitting: false };
        let p = primary_for(Status::CvReady, true, false, false, &working, &none);
        assert_eq!(
            state(p),
            Some((PrimaryAction::ReviewAndSubmit, false, true))
        );
        let submitting = ApplyUi::Working { submitting: true };
        let p = primary_for(Status::Submitting, true, false, false, &submitting, &none);
        assert_eq!(state(p), Some((PrimaryAction::Submit, false, true)));
        // A generate / revise job.
        let p = primary_for(Status::Draft, false, false, true, &ApplyUi::Idle, &none);
        assert_eq!(state(p), Some((PrimaryAction::Generate, false, true)));
        let p = primary_for(Status::CvReady, true, false, true, &ApplyUi::Idle, &none);
        assert!(p.is_some_and(|p| !p.enabled));
    }

    #[test]
    fn approve_waits_for_a_compiling_cv_and_review_waits_for_its_prerequisites() {
        let p = primary_for(Status::Draft, true, true, false, &ApplyUi::Idle, &none);
        assert_eq!(enabled(p), Some((PrimaryAction::ApproveCv, false)));

        let blocker = || {
            Some(Blocker {
                message: "No Chrome found".into(),
                settings: Some("Browser sessions"),
            })
        };
        let p = primary_for(
            Status::CvReady,
            true,
            false,
            false,
            &ApplyUi::Idle,
            &blocker,
        );
        assert_eq!(
            p.map(|p| (p.action, p.enabled, p.settings)),
            Some((
                PrimaryAction::ReviewAndSubmit,
                false,
                Some("Browser sessions")
            ))
        );
        // After a handoff the same button retries.
        let p = primary_for(Status::Failed, true, false, false, &ApplyUi::Handoff, &none);
        assert_eq!(enabled(p), Some((PrimaryAction::ReviewAndSubmit, true)));
    }

    #[test]
    fn the_primary_actions_have_labels_and_keys() {
        assert_eq!(PrimaryAction::ReviewAndSubmit.label(), "Review & Submit");
        assert_eq!(PrimaryAction::ApproveCv.key(), "approveCv");
    }

    #[test]
    fn the_stepper_follows_the_status() {
        assert_eq!(STEPS.len(), 5);
        // A draft is a lead until it has a CV, then the CV waits for review.
        assert_eq!(step_index(Status::Draft, false), 0);
        assert_eq!(step_index(Status::Draft, true), 1);
        // An approved CV, a running submit and a failed one are all at Apply.
        for status in [Status::CvReady, Status::Submitting, Status::Failed] {
            assert_eq!(step_index(status, true), 2, "{status:?}");
        }
        // Sent: waiting for replies. Any answer finishes the bar.
        assert_eq!(step_index(Status::Applied, true), 4);
        for status in [Status::Interview, Status::Offer, Status::Rejected] {
            assert_eq!(step_index(status, true), STEPS.len(), "{status:?}");
        }
    }

    #[test]
    fn the_stepper_never_goes_back_as_the_status_advances() {
        let order = [
            Status::Draft,
            Status::CvReady,
            Status::Submitting,
            Status::Applied,
            Status::Interview,
        ];
        let steps: Vec<usize> = order.iter().map(|s| step_index(*s, true)).collect();
        assert!(steps.windows(2).all(|w| w[0] <= w[1]), "{steps:?}");
    }

    #[test]
    fn statuses_have_a_tone() {
        assert_eq!(status_tone(Status::Draft), Tone::Neutral);
        assert_eq!(status_tone(Status::CvReady), Tone::Accent);
        assert_eq!(status_tone(Status::Submitting), Tone::Warn);
        assert_eq!(status_tone(Status::Applied), Tone::Ok);
        assert_eq!(status_tone(Status::Failed), Tone::Danger);
        assert_eq!(status_tone(Status::Rejected), Tone::Danger);
    }

    #[test]
    fn the_subtitle_adds_the_location_when_known() {
        assert_eq!(subtitle("Acme", None), "Acme");
        let blank = Analysis {
            location: "  ".into(),
            ..Analysis::default()
        };
        assert_eq!(subtitle("Acme", Some(&blank)), "Acme");
        let remote = Analysis {
            location: "Remote, EU".into(),
            ..Analysis::default()
        };
        assert_eq!(subtitle("Acme", Some(&remote)), "Acme \u{b7} Remote, EU");
    }

    #[test]
    fn progress_lines_carry_the_detail() {
        let p = Progress {
            app_id: 1,
            stage: JobStage::Tailoring,
            detail: String::new(),
        };
        assert_eq!(progress_label(&p), "Tailoring the CV");
        let p = Progress {
            detail: "repairing 2 problem(s)".into(),
            ..p
        };
        assert_eq!(
            progress_label(&p),
            "Tailoring the CV: repairing 2 problem(s)"
        );
    }

    // ------------------------------------------------------------ Timeline

    fn event(id: i64, kind: &str) -> Event {
        Event {
            id,
            app_id: 1,
            at: Utc::now(),
            kind: kind.into(),
            message: format!("m{id}"),
        }
    }

    #[test]
    fn kinds_map_to_styles() {
        assert_eq!(EventStyle::of("error"), EventStyle::Error);
        assert_eq!(EventStyle::of("email_review"), EventStyle::Review);
        assert_eq!(EventStyle::of("status"), EventStyle::Status);
        assert_eq!(EventStyle::of("round"), EventStyle::Round);
        assert_eq!(EventStyle::of("whatever"), EventStyle::Note);
    }

    #[test]
    fn kinds_have_a_dot_colour() {
        assert_eq!(EventStyle::of("error").tone(), Tone::Danger);
        assert_eq!(EventStyle::of("warning").tone(), Tone::Warn);
        assert_eq!(EventStyle::of("email_review").tone(), Tone::Warn);
        assert_eq!(EventStyle::of("status").tone(), Tone::Accent);
        assert_eq!(EventStyle::of("round").tone(), Tone::Accent);
        assert_eq!(EventStyle::of("created").tone(), Tone::Neutral);
    }

    #[test]
    fn a_row_shows_one_short_line() {
        assert_eq!(
            row_text("CV generated"),
            ("CV generated".to_string(), false)
        );
        let (first, cut) = row_text("Gaps: Haskell\nsecond line");
        assert_eq!((first.as_str(), cut), ("Gaps: Haskell", true));
        let (long, cut) = row_text(&"x".repeat(ROW_CHARS + 20));
        assert!(cut && long.ends_with("...") && long.chars().count() == ROW_CHARS + 3);
    }

    #[test]
    fn newest_event_comes_first() {
        let ids: Vec<i64> =
            newest_first(vec![event(1, "created"), event(2, "cv"), event(3, "error")])
                .iter()
                .map(|e| e.id)
                .collect();
        assert_eq!(ids, [3, 2, 1]);
    }

    fn email() -> Email {
        Email {
            id: "m1".into(),
            app_id: Some(1),
            from_addr: "hr@acme.test".into(),
            subject: "Your application".into(),
            snippet: String::new(),
            received_at: Utc::now(),
            kind: Some("rejection".into()),
            confidence: Some(0.6),
            summary: None,
            applied: false,
        }
    }

    #[test]
    fn review_title_shows_kind_and_confidence() {
        let mut email = email();
        assert_eq!(
            review_title(&email),
            "Rejection (60% sure): Your application"
        );
        email.confidence = None;
        assert_eq!(review_title(&email), "Rejection: Your application");
    }

    #[test]
    fn review_body_names_the_sender_and_the_summary() {
        let mut email = email();
        assert_eq!(review_body(&email), "From hr@acme.test");
        email.summary = Some("They declined".into());
        assert_eq!(review_body(&email), "From hr@acme.test - They declined");
        email.summary = Some(String::new());
        assert_eq!(review_body(&email), "From hr@acme.test");
    }

    // ------------------------------------------------------------ CV checklist

    fn analysis(must: &[&str], nice: &[&str]) -> Analysis {
        Analysis {
            must_have: must.iter().map(|s| s.to_string()).collect(),
            nice_to_have: nice.iter().map(|s| s.to_string()).collect(),
            ..Analysis::default()
        }
    }

    fn gaps(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_requirement_is_covered_unless_a_gap_is_about_it() {
        let a = analysis(
            &["Rust", "PostgreSQL", "Event streaming (Kafka)"],
            &["Terraform"],
        );
        let rows = requirements(Some(&a), &gaps(&["kafka", "Terraform: not in the profile"]));
        let covered: Vec<(&str, bool, bool)> = rows
            .iter()
            .map(|r| (r.text.as_str(), r.nice, r.gap.is_none()))
            .collect();
        assert_eq!(
            covered,
            [
                ("Rust", false, true),
                ("PostgreSQL", false, true),
                ("Event streaming (Kafka)", false, false),
                ("Terraform", true, false)
            ]
        );
        assert_eq!(rows[2].gap.as_deref(), Some("kafka"));
    }

    #[test]
    fn matching_is_by_whole_words() {
        assert!(same_topic("Event streaming (Kafka)", "kafka"));
        assert!(same_topic("Terraform", "Terraform: not in the profile"));
        assert!(same_topic(
            "Event streaming (Kafka)",
            "Kafka: nothing in the profile shows it"
        ));
        // Only the subject counts, not the explanation after the colon.
        assert!(!same_topic(
            "Rust",
            "Haskell: nothing like Rust in the profile"
        ));
        assert!(!same_topic("Go", "Google Cloud"));
        assert!(!same_topic("C++", "C#"));
        assert!(!same_topic("", "Rust"));
    }

    #[test]
    fn gap_lines_that_match_no_requirement_are_still_listed() {
        let a = analysis(&["Rust"], &[]);
        let rows = requirements(Some(&a), &gaps(&["Haskell"]));
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows[1],
            Requirement {
                text: "Haskell".into(),
                nice: false,
                gap: Some("Haskell".into())
            }
        );
        // No analysis at all: the gaps alone are the list.
        assert_eq!(requirements(None, &gaps(&["Haskell"])).len(), 1);
    }

    #[test]
    fn nothing_to_list_gives_an_empty_checklist() {
        assert!(requirements(None, &[]).is_empty());
        assert!(requirements(Some(&analysis(&[], &[])), &[]).is_empty());
        // Blank and repeated requirements are dropped.
        let a = analysis(&["Rust", " ", "rust"], &["RUST"]);
        assert_eq!(requirements(Some(&a), &[]).len(), 1);
    }

    #[test]
    fn an_uncovered_requirement_says_why() {
        let same = Requirement {
            text: "Haskell".into(),
            nice: false,
            gap: Some("haskell".into()),
        };
        assert_eq!(
            same.note().as_deref(),
            Some("Nothing in profile.yaml shows it")
        );
        let other = Requirement {
            text: "Kafka".into(),
            nice: false,
            gap: Some("Kafka: only toy projects".into()),
        };
        assert_eq!(other.note().as_deref(), Some("Kafka: only toy projects"));
        assert_eq!(
            Requirement {
                text: "Rust".into(),
                nice: false,
                gap: None
            }
            .note(),
            None
        );
    }

    #[test]
    fn gaps_come_from_json_strings() {
        assert_eq!(
            parse_gaps(Some(r#"["Kubernetes at scale", " ", "Haskell"]"#)),
            ["Kubernetes at scale", "Haskell"]
        );
        assert!(parse_gaps(Some("not json")).is_empty());
        assert!(parse_gaps(None).is_empty());
    }

    // ------------------------------------------------------------ Apply tab

    fn row(id: &str, kind: FieldKind) -> Row {
        Row {
            id: id.into(),
            step: 0,
            label: format!("Label {id}"),
            kind,
            required: false,
            options: Vec::new(),
            value: String::new(),
            source: Source::Profile,
            needs_input: false,
            warning: None,
            editable: true,
        }
    }

    fn review(rows: Vec<Row>) -> Review {
        Review {
            app_id: 1,
            ats: Ats::Greenhouse,
            url: "https://boards.greenhouse.io/acme/jobs/1".into(),
            dry_run: false,
            complete: true,
            rows,
            attachment: None,
            screenshots: Vec::new(),
            notes: Vec::new(),
            session: None,
        }
    }

    #[test]
    fn a_row_is_edited_with_the_widget_that_fits_it() {
        assert_eq!(field_input(&row("a", FieldKind::Email)), FieldInput::Line);
        assert_eq!(
            field_input(&row("a", FieldKind::Textarea)),
            FieldInput::Area
        );
        assert_eq!(
            field_input(&row("a", FieldKind::Checkbox)),
            FieldInput::Checkbox
        );
        assert_eq!(
            field_input(&row("a", FieldKind::File)),
            FieldInput::ReadOnly
        );
        let locked = Row {
            editable: false,
            ..row("a", FieldKind::Text)
        };
        assert_eq!(
            field_input(&locked),
            FieldInput::ReadOnly,
            "another step's field cannot be changed"
        );
        let long = Row {
            value: "x".repeat(91),
            ..row("a", FieldKind::Text)
        };
        assert_eq!(
            field_input(&long),
            FieldInput::Area,
            "a long answer gets room"
        );

        let few = |kind| Row {
            options: vec!["Yes".into(), "No".into()],
            ..row("a", kind)
        };
        assert_eq!(field_input(&few(FieldKind::Select)), FieldInput::Chips);
        assert_eq!(field_input(&few(FieldKind::Radio)), FieldInput::Chips);
        assert_eq!(field_input(&few(FieldKind::Combobox)), FieldInput::Chips);
        assert_eq!(
            field_input(&few(FieldKind::CheckboxGroup)),
            FieldInput::MultiChips
        );
        let many = Row {
            options: (0..=MAX_CHIPS).map(|i| i.to_string()).collect(),
            ..row("a", FieldKind::Select)
        };
        assert_eq!(field_input(&many), FieldInput::Line, "a long list is typed");
        assert!(placeholder(&many).starts_with("One of: 0 | 1"));
        assert_eq!(placeholder(&row("a", FieldKind::Text)), "Your answer");
    }

    #[test]
    fn required_rows_that_are_empty_block_submitting() {
        let required = |id, kind| Row {
            required: true,
            ..row(id, kind)
        };
        let r = review(vec![
            required("a", FieldKind::Text),
            Row {
                value: "set".into(),
                ..required("b", FieldKind::Text)
            },
            required("cv", FieldKind::File),
            Row {
                editable: false,
                ..required("c", FieldKind::Text)
            },
            row("d", FieldKind::Text),
        ]);
        let typed = HashMap::new();
        assert_eq!(
            missing_count(&r, &typed),
            1,
            "only an empty required row of this step, never a file"
        );
        let typed = HashMap::from([("a".to_string(), "  ".to_string())]);
        assert_eq!(missing_count(&r, &typed), 1, "blanks count as empty");
        let typed = HashMap::from([
            ("a".to_string(), "x".to_string()),
            ("b".to_string(), String::new()),
        ]);
        assert_eq!(
            missing_count(&r, &typed),
            1,
            "what is typed replaces what was filled"
        );
        let typed = HashMap::from([("a".to_string(), "x".to_string())]);
        assert_eq!(missing_count(&r, &typed), 0);
    }

    #[test]
    fn edits_carry_what_is_typed_and_what_to_remember() {
        let r = review(vec![
            Row {
                value: "filled".into(),
                ..row("a", FieldKind::Text)
            },
            row("b", FieldKind::Text),
            Row {
                editable: false,
                ..row("c", FieldKind::Text)
            },
        ]);
        let typed = HashMap::from([("b".to_string(), "typed".to_string())]);
        let remember = std::collections::HashSet::from(["b".to_string()]);
        let edits: Vec<(String, String, bool)> = edits_for(&r, &typed, &remember)
            .into_iter()
            .map(|e| (e.id, e.value, e.remember))
            .collect();
        assert_eq!(
            edits,
            [
                ("a".into(), "filled".into(), false),
                ("b".into(), "typed".into(), true)
            ]
        );
    }

    #[test]
    fn a_row_says_where_its_answer_came_from() {
        let tag = |source, needs_input| {
            source_tag(&Row {
                source,
                needs_input,
                ..row("a", FieldKind::Text)
            })
        };
        assert_eq!(
            tag(Source::Profile, false),
            ("profile".to_string(), Tone::Ok)
        );
        assert_eq!(
            tag(Source::Generated, false),
            ("generated".to_string(), Tone::Warn)
        );
        assert_eq!(
            tag(Source::Unknown, false),
            ("needs input".to_string(), Tone::Danger)
        );
        assert_eq!(
            tag(Source::Profile, true),
            ("needs input".to_string(), Tone::Danger)
        );
        assert_eq!(tag(Source::User, false), ("you".to_string(), Tone::Neutral));
        assert_eq!(tag(Source::Cv, false), ("cv".to_string(), Tone::Neutral));
    }

    #[test]
    fn the_session_line_says_what_the_run_will_use() {
        let (line, tone) = session_line(&SessionState::NotNeeded);
        assert!(line.starts_with("No login needed"));
        assert_eq!(tone, Tone::Neutral);
        let matched = |status| SessionState::Matched {
            id: 1,
            name: "LinkedIn".into(),
            status,
        };
        assert_eq!(
            session_line(&matched(SessionStatus::Ok)),
            ("Session: LinkedIn (logged in)".to_string(), Tone::Neutral)
        );
        assert_eq!(session_line(&matched(SessionStatus::Expired)).1, Tone::Warn);
        assert!(session_line(&matched(SessionStatus::New))
            .0
            .contains("not verified yet"));
        let (line, tone) = session_line(&SessionState::Missing {
            domain: "linkedin.com".into(),
        });
        assert!(line.contains("linkedin.com") && line.contains("Settings > Browser sessions"));
        assert_eq!(tone, Tone::Danger);
    }

    #[test]
    fn a_review_explains_what_stands_in_the_way() {
        let mut r = review(Vec::new());
        assert!(review_notice(&r, 0).is_none());
        assert_eq!(
            review_notice(&r, 2).map(|n| (n.0, n.1)),
            Some((Tone::Warn, "Required fields empty"))
        );
        r.complete = false;
        assert_eq!(review_notice(&r, 2).map(|n| n.1), Some("More steps"));
        r.dry_run = true;
        assert_eq!(review_notice(&r, 0).map(|n| n.1), Some("Dry run"));
    }

    #[test]
    fn files_and_hosts_are_shown_short() {
        assert_eq!(file_name("/tmp/shots/cv-acme.pdf"), "cv-acme.pdf");
        assert_eq!(file_name("plain"), "plain");
        assert_eq!(
            host("https://boards.greenhouse.io/acme/jobs/1"),
            "boards.greenhouse.io"
        );
        assert_eq!(host("the site"), "the site");
    }
}
