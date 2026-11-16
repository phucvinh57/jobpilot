//! Read models of this context (serde, camelCase). Core types stay free of serde and Tauri.
//!
//! Text that comes from a posting, a CV or an email is carried as plain strings: the web app
//! shows it as text, never as markup.

use std::path::PathBuf;

use anyhow::Result;
use crate::engine::apply::{self, FieldKind, Handoff, Review, UrlSource};
use chrono::Local;
use crate::engine::health;
use crate::engine::pipeline::{read_jd_text, Analysis, Insight};
use crate::engine::store::{Application, Event, Status};
use crate::engine::Core;
use serde::Serialize;

use super::files::CvFiles;
use super::rules::{
    activity_text, activity_tone, activity_when, age_label, board, can_submit_manually, card_title, current_value, cv_export_name,
    dismissible, field_input, file_name, host, is_sent, marks_for, newest_first, next_action, next_job_offered,
    outcome_for, parse_gaps, placeholder, primary_for, requirements, review_body, review_notice,
    review_title, round_options, row_text, session_line, short_urls, source_tag, stage_for,
    status_tone, step_index, subtitle, ApplyUi, Blocker, EventStyle, Mark, Outcome, Primary, Tone,
    STEPS,
};
use super::state::{ApplyView, JobView, Phase};
use crate::shared::insight::{interview_steps, is_web_link};
use crate::shared::{CommandError, InsightDto, ProblemDto};

/// How many events "Recent activity" shows.
pub const ACTIVITY_ROWS: usize = 6;

// ---------------------------------------------------------------- events

/// `applications:progress`: the running generate / revise job moved. Labels only.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDto {
    pub app_id: i64,
    pub label: String,
}

/// `applications:apply-progress`: the steps of the form run so far. Labels only.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyProgressDto {
    pub app_id: i64,
    pub lines: Vec<String>,
}

// ---------------------------------------------------------------- Jobs page

/// The application stages of the Jobs page (CVs | Applied | In progress | Done) and the latest events.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardDto {
    pub count: usize,
    pub stages: Vec<StageDto>,
    pub activity: Vec<ActivityDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageDto {
    /// `cv` / `applied` / `inProgress` / `done`.
    pub key: &'static str,
    pub label: &'static str,
    pub cards: Vec<CardDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub label: String,
    pub tone: Tone,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardDto {
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    /// The status as a person reads it (`CV ready`).
    pub status: &'static str,
    /// What the user does next.
    pub next: TagDto,
    /// How long ago the card last changed (`today`, `3 d`).
    pub age: String,
    /// The interview round the user marked (kept once the application is answered).
    pub round: Option<String>,
    /// `success` (offer) / `failed` (rejected); `None` while open.
    pub outcome: Option<&'static str>,
    /// The key of the stage the card is in.
    pub stage: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityDto {
    pub id: i64,
    pub app_id: i64,
    pub text: String,
    pub when: String,
    pub tone: Tone,
}

/// The board and the latest events. `has_cv` says which applications have a `cv.yaml`.
pub fn board_dto(
    apps: &[Application],
    events: &[Event],
    busy: &std::collections::HashSet<i64>,
    has_cv: &dyn Fn(&Application) -> bool,
) -> BoardDto {
    let now = Local::now().naive_local();
    let local = |at: chrono::DateTime<chrono::Utc>| {
        at.with_timezone(&Local).naive_local()
    };
    let stages = board(apps)
        .into_iter()
        .map(|(stage, rows)| StageDto {
            key: stage.key(),
            label: stage.label(),
            cards: rows
                .into_iter()
                .map(|app| {
                    let next = next_action(
                        app.status,
                        has_cv(app),
                        busy.contains(&app.id),
                        app.interview_round.as_deref(),
                    );
                    let (title, subtitle) = card_title(&app.role, &app.company);
                    CardDto {
                        id: app.id,
                        title,
                        subtitle,
                        status: app.status.label(),
                        next: TagDto {
                            label: next.label,
                            tone: next.tone,
                        },
                        age: age_label(now.date(), local(app.updated_at).date()),
                        round: app.interview_round.clone(),
                        outcome: outcome_for(app.status).map(Outcome::key),
                        stage: stage.key(),
                    }
                })
                .collect(),
        })
        .collect();
    // An event of an application that is gone has nothing to open.
    let activity = events
        .iter()
        .filter_map(|e| {
            let app = apps.iter().find(|a| a.id == e.app_id)?;
            Some(ActivityDto {
                id: e.id,
                app_id: app.id,
                text: format!("{}: {}", app.company, activity_text(&e.kind, &e.message)),
                when: activity_when(now, local(e.at)),
                tone: activity_tone(&e.kind),
            })
        })
        .collect();
    BoardDto {
        count: apps.iter().filter(|a| a.status != Status::Dismissed).count(),
        stages,
        activity,
    }
}

// ---------------------------------------------------------------- Application screen

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub key: &'static str,
    pub label: &'static str,
    pub tone: Tone,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrimaryDto {
    /// `generate` / `approveCv` / `reviewAndSubmit` / `continue` / `submit`.
    pub action: &'static str,
    pub label: &'static str,
    pub enabled: bool,
    pub loading: bool,
    pub hint: Option<String>,
    /// The Settings section that fixes what blocks the button.
    pub settings: Option<&'static str>,
}

impl From<Primary> for PrimaryDto {
    fn from(p: Primary) -> Self {
        Self {
            action: p.action.key(),
            label: p.action.label(),
            enabled: p.enabled,
            loading: p.loading,
            hint: p.hint,
            settings: p.settings,
        }
    }
}

/// The header of the Application screen: title, status, what is running, the one primary action.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDto {
    pub id: i64,
    pub company: String,
    pub role: String,
    pub status: StatusDto,
    /// Company, then the location when the analysis found one.
    pub subtitle: String,
    pub has_cv: bool,
    pub steps: Vec<&'static str>,
    /// The step in progress (`steps.len()`: every step is done).
    pub step_index: usize,
    /// The progress line of the running generate / revise job.
    pub job: Option<String>,
    /// The last failure of a job, until the next job starts or it is dismissed.
    pub error: Option<CommandError>,
    /// `None`: read-only (sent, or answered by the company).
    pub primary: Option<PrimaryDto>,
    /// A plain "Next job" button replaces the primary one.
    pub show_next_job: bool,
    /// Changes whenever a job rewrote the CV files.
    pub cv_epoch: u64,
    /// The interview round the user marked; `None`: none marked.
    pub round: Option<String>,
    /// The rounds to pick from (the brief's interview process, or the usual ones); the web app
    /// offers them while the status is `interview` and also lets the user type their own.
    pub round_options: Vec<String>,
    /// What the user can mark by hand (empty unless the status is `applied` or `interview`).
    pub marks: Vec<MarkDto>,
    /// The key of the Jobs page stage (`cv` / `applied` / `inProgress` / `done`).
    pub stage: &'static str,
    /// `success` (offer) / `failed` (rejected); `None` while open.
    pub outcome: Option<&'static str>,
    /// A plain "Dismiss" is offered: the job is in CVs and nothing runs for it.
    pub dismissible: bool,
    /// The job was dismissed (a plain "Restore" brings it back); `None` otherwise.
    pub dismissed: Option<DismissalDto>,
}

/// Why and when a job was dismissed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DismissalDto {
    /// What the user typed; `None` when they gave no reason.
    pub reason: Option<String>,
    /// RFC 3339; `None` when unknown.
    pub at: Option<String>,
}

impl DismissalDto {
    /// The dismissal of `app`, if it is dismissed.
    pub fn of(app: &Application) -> Option<Self> {
        (app.status == Status::Dismissed).then(|| Self {
            reason: app
                .dismiss_reason
                .as_deref()
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(str::to_string),
            at: app.dismissed_at.map(|at| at.to_rfc3339()),
        })
    }
}

/// A manual marker: `interview` ("Got an interview") / `offer` / `rejected`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkDto {
    pub key: &'static str,
    pub label: &'static str,
}

impl From<Mark> for MarkDto {
    fn from(m: Mark) -> Self {
        Self {
            key: m.key(),
            label: m.label(),
        }
    }
}

/// The interview stages of the application's company brief; empty without a brief.
fn insight_steps(app: &Application) -> Vec<String> {
    app.insight_json
        .as_deref()
        .and_then(Insight::from_json)
        .map(|i| interview_steps(&i.interview))
        .unwrap_or_default()
}

fn blocker_of(core: &Core, app: &Application) -> Option<Blocker> {
    let info = apply::apply_info(core, app);
    Some(Blocker {
        message: info.blocker()?,
        settings: info.blocker_settings(),
    })
}

/// The primary action of `app`, from its status and the live state.
pub fn primary_of(
    core: &Core,
    app: &Application,
    job: &JobView,
    apply_ui: &ApplyUi,
) -> Option<Primary> {
    let has_cv = app.cv_yaml_path().is_file();
    primary_for(
        app.status,
        has_cv,
        job.cv_problem.is_some(),
        job.label.is_some(),
        apply_ui,
        &|| blocker_of(core, app),
    )
}

pub fn application_dto(
    core: &Core,
    app: &Application,
    job: JobView,
    apply_ui: ApplyUi,
) -> ApplicationDto {
    let has_cv = app.cv_yaml_path().is_file();
    let analysis = app
        .analysis_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<Analysis>(j).ok());
    let primary = primary_of(core, app, &job, &apply_ui);
    let can_dismiss = dismissible(app.status, job.label.is_some(), &apply_ui);
    let show_next_job = primary.is_none() && next_job_offered(app.status, &apply_ui);
    ApplicationDto {
        id: app.id,
        company: app.company.clone(),
        role: app.role.clone(),
        status: StatusDto {
            key: app.status.as_str(),
            label: app.status.label(),
            tone: status_tone(app.status),
        },
        subtitle: subtitle(&app.company, analysis.as_ref()),
        has_cv,
        steps: STEPS.to_vec(),
        step_index: step_index(app.status, has_cv),
        job: job.label,
        error: job.error.map(CommandError::new),
        primary: primary.map(PrimaryDto::from),
        show_next_job,
        cv_epoch: job.cv_epoch,
        round: app.interview_round.clone(),
        round_options: round_options(&insight_steps(app)),
        marks: marks_for(app.status).into_iter().map(MarkDto::from).collect(),
        stage: stage_for(app.status).key(),
        outcome: outcome_for(app.status).map(Outcome::key),
        dismissible: can_dismiss,
        dismissed: DismissalDto::of(app),
    }
}

// ---------------------------------------------------------------- JD tab

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JdDto {
    /// `None`: no JD stored (yet).
    pub text: Option<String>,
    /// The link or file path the JD came from.
    pub source: Option<String>,
    pub source_is_link: bool,
    pub must_have: Vec<String>,
    pub nice_to_have: Vec<String>,
    /// The company brief; `None`: not researched (yet).
    pub insight: Option<InsightDto>,
}

pub fn jd_dto(app: &Application) -> JdDto {
    let analysis = app
        .analysis_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<Analysis>(j).ok())
        .unwrap_or_default();
    let source = app.source.clone().filter(|s| !s.trim().is_empty());
    JdDto {
        text: read_jd_text(app).ok(),
        source_is_link: source.as_deref().is_some_and(is_web_link),
        source,
        must_have: analysis.must_have,
        nice_to_have: analysis.nice_to_have,
        insight: app
            .insight_json
            .as_deref()
            .and_then(Insight::from_json)
            .map(|i| InsightDto::from(&i)),
    }
}

// ---------------------------------------------------------------- CV tab

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequirementDto {
    pub text: String,
    pub nice: bool,
    /// Why the profile does not cover it; `None`: covered.
    pub note: Option<String>,
    pub covered: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CvDto {
    pub has_cv: bool,
    pub yaml: Option<String>,
    pub page_count: usize,
    /// `cv.yaml` is newer than its previews (or they are missing): render it once.
    pub stale: bool,
    /// The fact guard ran on this CV and found nothing.
    pub guard_clean: bool,
    /// What the posting asks for, checked against the profile; empty when the CV has no gap list yet.
    pub requirements: Vec<RequirementDto>,
    /// The tailoring step reported gaps: offer to add to the profile (Profile > About you).
    pub has_gaps: bool,
}

pub fn cv_dto(app: &Application, files: CvFiles, guard_clean: bool) -> CvDto {
    let has_cv = files.yaml.is_some();
    let gaps = parse_gaps(app.gaps_json.as_deref());
    let analysis = app
        .analysis_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<Analysis>(j).ok());
    // The checklist needs the tailoring step's gap list (written with the CV): without it nothing
    // can be called covered.
    let checked = app.gaps_json.is_some() && has_cv;
    let requirements = if checked {
        requirements(analysis.as_ref(), &gaps)
            .into_iter()
            .map(|r| RequirementDto {
                note: r.note(),
                covered: r.gap.is_none(),
                nice: r.nice,
                text: r.text,
            })
            .collect()
    } else {
        Vec::new()
    };
    CvDto {
        has_cv,
        stale: has_cv && files.stale,
        page_count: files.pages.len(),
        yaml: files.yaml,
        guard_clean,
        requirements,
        has_gaps: !gaps.is_empty(),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedDto {
    pub page_count: usize,
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------- Timeline tab

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDto {
    pub id: i64,
    pub tone: Tone,
    /// The first line of the event, cut short.
    pub text: String,
    /// The whole event when the row leaves something out.
    pub full: Option<String>,
    pub when: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailReviewDto {
    pub id: String,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineDto {
    /// Newest first.
    pub events: Vec<EventDto>,
    /// Emails waiting for an accept / dismiss.
    pub reviews: Vec<EmailReviewDto>,
}

pub fn timeline_dto(events: Vec<Event>, reviews: &[crate::engine::store::Email]) -> TimelineDto {
    let events = newest_first(events)
        .into_iter()
        .map(|e| {
            // Links in an event (an error's URL can carry a token) are shown as host and path.
            let message = short_urls(&e.message);
            let (text, cut) = row_text(&message);
            EventDto {
                id: e.id,
                tone: EventStyle::of(&e.kind).tone(),
                text,
                full: cut.then_some(message),
                when: e
                    .at
                    .with_timezone(&Local)
                    .format("%Y-%m-%d %H:%M")
                    .to_string(),
            }
        })
        .collect();
    let reviews = reviews
        .iter()
        .map(|email| EmailReviewDto {
            id: email.id.clone(),
            title: review_title(email),
            body: review_body(email),
        })
        .collect();
    TimelineDto { events, reviews }
}

// ---------------------------------------------------------------- Apply tab

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdleDto {
    /// The detected ATS (`Greenhouse`, `Generic form`).
    pub ats: &'static str,
    /// The application was sent.
    pub applied: bool,
    pub apply_url: Option<String>,
    pub url_note: Option<&'static str>,
    pub session: String,
    pub session_tone: Tone,
    /// The "Dry run" box (checked and locked when `JOBPILOT_DRY_RUN` is set).
    pub dry_run: bool,
    pub dry_run_forced: bool,
    /// A confirmation screenshot of the sent application exists.
    pub has_proof: bool,
    /// The user may apply by hand (save the CV as a PDF, send it on the site) and mark it as applied.
    pub can_submit_manually: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkingDto {
    pub title: &'static str,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeDto {
    pub tone: Tone,
    pub title: &'static str,
    pub body: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteDto {
    pub text: String,
    pub problem: ProblemDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RowDto {
    pub id: String,
    pub label: String,
    pub required: bool,
    /// The step of the form the field is on (0-based).
    pub step: usize,
    pub editable: bool,
    /// `text` / `email` / `file` / ... (`FieldKind::as_str`).
    pub kind: &'static str,
    /// `readonly` / `checkbox` / `chips` / `multiChips` / `area` / `line`.
    pub input: &'static str,
    pub options: Vec<String>,
    pub placeholder: String,
    /// What the field has now (what the user typed, else what the form was filled with).
    pub value: String,
    /// The file name for a file field, else the value.
    pub shown: String,
    pub tag: TagDto,
    pub warning: Option<String>,
    /// "Needs input" or a warning: highlighted.
    pub flagged: bool,
    /// The answer can be remembered for the next applications.
    pub rememberable: bool,
    pub remember: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewDto {
    pub title: String,
    pub subtitle: String,
    /// `Close dry run` or `Discard`.
    pub close_label: &'static str,
    pub dry_run: bool,
    /// More steps to go: Continue instead of Submit.
    pub complete: bool,
    pub missing: usize,
    pub notice: Option<NoticeDto>,
    pub notes: Vec<NoteDto>,
    pub rows: Vec<RowDto>,
    /// The file attached to the form (its name).
    pub attached: Option<String>,
    /// Screenshots of the filled form (see `applications_apply_image`, kind `filled`).
    pub screenshots: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DoneDto {
    pub body: String,
    pub has_proof: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandoffDto {
    pub title: &'static str,
    pub reason: String,
    pub captcha: bool,
    /// The form's tab was brought to the front for the user.
    pub headed: bool,
    pub has_screenshot: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyDto {
    /// `idle` / `preparing` / `review` / `submitting` / `done` / `handoff`.
    pub phase: &'static str,
    /// Changes when a new review arrives (the web app starts its editors from it once).
    pub generation: u64,
    pub idle: Option<IdleDto>,
    pub working: Option<WorkingDto>,
    pub review: Option<ReviewDto>,
    pub done: Option<DoneDto>,
    pub handoff: Option<HandoffDto>,
    /// The failure of the last action.
    pub error: Option<CommandError>,
    /// Retry (open and fill the form again) makes sense for `error`.
    pub can_retry: bool,
    /// A short action is running (`Closing the browser...`).
    pub busy: Option<&'static str>,
    /// The file name offered by "Save CV as PDF"; `None` while there is no rendered CV.
    pub cv_file: Option<String>,
}

fn row_dto(row: &apply::Row, view: &ApplyView) -> RowDto {
    let value = current_value(row, &view.typed).to_string();
    let input = field_input(row);
    let (label, tone) = source_tag(row);
    RowDto {
        id: row.id.clone(),
        label: row.label.clone(),
        required: row.required,
        step: row.step,
        editable: row.editable,
        kind: row.kind.as_str(),
        input: input.key(),
        options: row.options.clone(),
        placeholder: placeholder(row),
        shown: if row.kind == FieldKind::File {
            file_name(&row.value)
        } else {
            value.clone()
        },
        value,
        tag: TagDto { label, tone },
        warning: row.warning.clone(),
        flagged: row.editable && (row.needs_input || row.warning.is_some()),
        rememberable: row.rememberable(),
        remember: view.remember.contains(&row.id),
    }
}

fn review_dto(review: &Review, view: &ApplyView) -> ReviewDto {
    let missing = view.missing();
    let editable = review.rows.iter().filter(|r| r.editable).count();
    let needs = review
        .rows
        .iter()
        .filter(|r| r.editable && (r.needs_input || r.warning.is_some()))
        .count();
    ReviewDto {
        title: format!("{} - {}", review.ats.label(), host(&review.url)),
        subtitle: format!(
            "{editable} fields on this step, {needs} to check{}",
            review
                .session
                .as_ref()
                .map(|s| format!(" - session {s}"))
                .unwrap_or_default()
        ),
        close_label: if review.dry_run {
            "Close dry run"
        } else {
            "Discard"
        },
        dry_run: review.dry_run,
        complete: review.complete,
        missing,
        notice: review_notice(review, missing).map(|(tone, title, body)| NoticeDto {
            tone,
            title,
            body,
        }),
        notes: review
            .notes
            .iter()
            .map(|n| NoteDto {
                text: n.clone(),
                problem: ProblemDto::from(health::classify(n)),
            })
            .collect(),
        rows: review.rows.iter().map(|r| row_dto(r, view)).collect(),
        attached: review
            .attachment
            .as_ref()
            .map(|p| file_name(&p.to_string_lossy())),
        screenshots: review.screenshots.len(),
    }
}

fn handoff_dto(h: &Handoff, headed: bool) -> HandoffDto {
    HandoffDto {
        title: if h.captcha {
            "A CAPTCHA is in the way"
        } else {
            "The form needs you"
        },
        reason: h.reason.clone(),
        captcha: h.captcha,
        headed,
        has_screenshot: h.screenshot.is_some(),
    }
}

/// What the Apply tab shows for `app`.
pub fn apply_dto(core: &Core, app: &Application, view: &ApplyView, dry_run: bool) -> ApplyDto {
    let base = ApplyDto {
        phase: "idle",
        generation: view.generation,
        idle: None,
        working: None,
        review: None,
        done: None,
        handoff: None,
        error: view.error.clone().map(CommandError::new),
        can_retry: matches!(view.phase, Phase::Idle),
        busy: view.busy,
        cv_file: super::files::pdf_is_current(app).then(|| cv_export_name(&app.company)),
    };
    match &view.phase {
        Phase::Review if view.review.is_some() => ApplyDto {
            phase: "review",
            review: view.review.as_ref().map(|r| review_dto(r, view)),
            ..base
        },
        Phase::Preparing => ApplyDto {
            phase: "preparing",
            working: Some(WorkingDto {
                title: "Preparing the application",
                lines: view.progress.clone(),
            }),
            ..base
        },
        Phase::Submitting => ApplyDto {
            phase: "submitting",
            working: Some(WorkingDto {
                title: "Submitting",
                lines: view.progress.clone(),
            }),
            ..base
        },
        Phase::Done(proof) => ApplyDto {
            phase: "done",
            done: Some(DoneDto {
                body: format!(
                    "{} confirmed the application for {}. Replies will show up from your email.",
                    host(app.apply_url.as_deref().unwrap_or("the site")),
                    app.role
                ),
                has_proof: proof.is_some(),
            }),
            ..base
        },
        Phase::Handoff { handoff, headed } => ApplyDto {
            phase: "handoff",
            handoff: Some(handoff_dto(handoff, *headed)),
            ..base
        },
        // Nothing open, or a review that is gone: what would happen.
        Phase::Idle | Phase::Review => {
            let info = apply::apply_info(core, app);
            let applied = is_sent(app.status);
            let (session, session_tone) = session_line(&info.session);
            ApplyDto {
                idle: Some(IdleDto {
                    ats: info.ats.label(),
                    applied,
                    url_note: info.apply_url.as_ref().map(|_| match info.url_source {
                        Some(UrlSource::Analysis) => "found in the job description",
                        _ => "set for this application",
                    }),
                    apply_url: info.apply_url,
                    session,
                    session_tone,
                    dry_run: dry_run || info.dry_run_forced,
                    dry_run_forced: info.dry_run_forced,
                    has_proof: applied && apply::latest_proof(core, &app.slug).is_some(),
                    can_submit_manually: can_submit_manually(app.status),
                }),
                ..base
            }
        }
    }
}

/// The screenshot the web app asks for: `proof` (the confirmation of a sent application), `filled`
/// (the filled form, `index`-th step), `handoff` (what the page looked like when the run stopped) or
/// `done` (the proof of the submit that just happened). Only files the run itself produced.
pub fn apply_image_path(
    core: &Core,
    app: &Application,
    view: &ApplyView,
    kind: &str,
    index: usize,
) -> Option<PathBuf> {
    let path = match (kind, &view.phase) {
        ("proof", _) => apply::latest_proof(core, &app.slug),
        ("filled", _) => view
            .review
            .as_ref()
            .and_then(|r| r.screenshots.get(index).cloned()),
        ("handoff", Phase::Handoff { handoff, .. }) => handoff.screenshot.clone(),
        ("done", Phase::Done(proof)) => proof.clone(),
        _ => None,
    }?;
    // Runs only ever write screenshots to the shots folder.
    (path.starts_with(&core.paths.shots_dir) && path.extension().is_some_and(|e| e == "png"))
        .then_some(path)
}

/// `Result` helper for the commands that read a stored application.
pub fn require_application(core: &Core, id: i64) -> Result<Application> {
    use anyhow::Context as _;
    core.store
        .get_application(id)?
        .with_context(|| format!("application {id} not found"))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use crate::engine::apply::{Ats, FieldKind, Row, Source};
    use chrono::{TimeZone, Utc};
    use crate::engine::store::{Email, Status};

    use super::*;

    fn app(id: i64, status: Status) -> Application {
        let moment = Utc::now();
        Application {
            id,
            slug: format!("a{id}"),
            company: format!("Company {id}"),
            role: "Engineer".into(),
            source: Some("https://acme.test/jobs/1".into()),
            apply_url: None,
            status,
            dir: Default::default(),
            analysis_json: None,
            gaps_json: None,
            created_at: moment,
            updated_at: moment,
            insight_json: None,
            interview_round: None,
            dismissed_from: None,
            dismiss_reason: None,
            dismissed_at: None,
        }
    }

    fn event(id: i64, app_id: i64, kind: &str, message: &str) -> Event {
        Event {
            id,
            app_id,
            at: Utc
                .with_ymd_and_hms(2026, 11, 1, 9, 0, 0)
                .single()
                .expect("a moment"),
            kind: kind.into(),
            message: message.into(),
        }
    }

    #[test]
    fn the_board_marks_busy_applications_and_drops_events_of_gone_ones() {
        let apps = vec![app(1, Status::Draft), app(2, Status::Applied)];
        let events = vec![
            event(3, 2, "status", "cv_ready -> applied"),
            event(2, 9, "note", "ghost"),
            event(1, 1, "created", "application created"),
        ];
        let dto = board_dto(&apps, &events, &HashSet::from([1]), &|a| a.id == 2);
        assert_eq!(dto.count, 2);
        assert_eq!(
            dto.stages.iter().map(|s| s.key).collect::<Vec<_>>(),
            ["cv", "applied", "inProgress", "done"]
        );
        assert_eq!(
            dto.stages[0].cards[0].next.label, "Generating",
            "a running job wins"
        );
        let applied = &dto.stages[1].cards[0];
        assert_eq!(applied.next.label, "No reply yet");
        assert_eq!(applied.subtitle, "Company 2");
        assert_eq!(applied.age, "today");
        assert_eq!((applied.stage, applied.outcome, applied.round.as_deref()), ("applied", None, None));
        let texts: Vec<&str> = dto.activity.iter().map(|a| a.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "Company 2: moved to Applied",
                "Company 1: application created"
            ]
        );
    }

    #[test]
    fn cards_carry_the_round_and_the_outcome() {
        let mut interview = app(1, Status::Interview);
        interview.interview_round = Some("Technical interview".into());
        let mut offer = app(2, Status::Offer);
        offer.interview_round = Some("Final interview".into());
        let apps = vec![interview, offer, app(3, Status::Rejected)];
        let events = vec![event(1, 1, "error", "fetching https://grab.careers/jobs/1?afd_azwaf_tok=abc failed")];
        let dto = board_dto(&apps, &events, &HashSet::new(), &|_| true);
        let card = &dto.stages[2].cards[0];
        assert_eq!((card.stage, card.next.label.as_str(), card.next.tone), ("inProgress", "Technical interview", Tone::Warn));
        assert_eq!(card.round.as_deref(), Some("Technical interview"));
        let done: Vec<(i64, Option<&str>, &str)> =
            dto.stages[3].cards.iter().map(|c| (c.id, c.outcome, c.next.label.as_str())).collect();
        assert_eq!(done, [(2, Some("success"), "Offer"), (3, Some("failed"), "Rejected")]);
        assert_eq!(dto.activity[0].text, "Company 1: fetching grab.careers/jobs/1 failed");
    }

    #[test]
    fn the_timeline_is_newest_first_and_cuts_long_events() {
        let long = format!("{}\nsecond line", "x".repeat(200));
        let events = vec![
            event(1, 1, "created", "application created"),
            event(2, 1, "error", &long),
        ];
        let email = Email {
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
        };
        let dto = timeline_dto(events, &[email]);
        assert_eq!(dto.events.iter().map(|e| e.id).collect::<Vec<_>>(), [2, 1]);
        assert_eq!(dto.events[0].tone, Tone::Danger);
        assert!(dto.events[0].text.ends_with("..."));
        assert_eq!(dto.events[0].full.as_deref(), Some(long.as_str()));
        assert_eq!(dto.events[1].full, None);
        assert_eq!(
            dto.reviews[0].title,
            "Rejection (60% sure): Your application"
        );
        assert_eq!(dto.reviews[0].body, "From hr@acme.test");

        // Links lose their query, in the row and in the whole text.
        let token = format!("blocked at https://a.test/jobs/1?tok=secret\n{}", "y".repeat(10));
        let dto = timeline_dto(vec![event(3, 1, "error", &token)], &[]);
        assert_eq!(dto.events[0].text, "blocked at a.test/jobs/1");
        assert_eq!(
            dto.events[0].full.as_deref(),
            Some(format!("blocked at a.test/jobs/1\n{}", "y".repeat(10)).as_str())
        );
        assert!(!format!("{:?}", dto.events).contains("secret"));
    }

    #[test]
    fn only_web_sources_are_links() {
        let mut a = app(1, Status::Draft);
        assert!(jd_dto(&a).source_is_link);
        a.source = Some("HTTP://acme.test".into());
        assert!(jd_dto(&a).source_is_link);
        a.source = Some("/home/me/jd.pdf".into());
        assert!(!jd_dto(&a).source_is_link);
        a.source = Some("  ".into());
        assert_eq!(jd_dto(&a).source, None);
    }

    #[test]
    fn the_checklist_needs_the_gap_list_of_the_tailoring_step() {
        let mut a = app(1, Status::Draft);
        a.analysis_json = Some(r#"{"must_have": ["Rust", "Kafka"]}"#.into());
        let files = || CvFiles {
            yaml: Some("basics: {}".into()),
            pages: Vec::new(),
            stale: true,
        };
        assert!(
            cv_dto(&a, files(), true).requirements.is_empty(),
            "no gap list yet: nothing can be called covered"
        );
        a.gaps_json = Some(r#"["Kafka: nothing shows it"]"#.into());
        let dto = cv_dto(&a, files(), true);
        let rows: Vec<(&str, bool)> = dto
            .requirements
            .iter()
            .map(|r| (r.text.as_str(), r.covered))
            .collect();
        assert_eq!(rows, [("Rust", true), ("Kafka", false)]);
        assert!(dto.has_gaps && dto.stale && dto.guard_clean);
        let none = CvFiles {
            yaml: None,
            pages: Vec::new(),
            stale: true,
        };
        let dto = cv_dto(&a, none, false);
        assert!(
            dto.requirements.is_empty() && !dto.stale,
            "no CV: no checklist, nothing to render"
        );
    }

    fn row(id: &str) -> Row {
        Row {
            id: id.into(),
            step: 0,
            label: "Email".into(),
            kind: FieldKind::Email,
            required: true,
            options: Vec::new(),
            value: "me@example.com".into(),
            source: Source::Profile,
            needs_input: false,
            warning: None,
            editable: true,
        }
    }

    #[test]
    fn a_row_shows_what_is_typed_and_what_can_be_remembered() {
        let mut view = ApplyView::new();
        view.set_review(Review {
            app_id: 1,
            ats: Ats::Lever,
            url: "https://jobs.lever.co/acme/1".into(),
            dry_run: false,
            complete: true,
            rows: vec![
                row("a"),
                Row {
                    source: Source::Generated,
                    warning: Some("check".into()),
                    ..row("b")
                },
            ],
            attachment: Some(PathBuf::from("/tmp/cv.pdf")),
            screenshots: vec![PathBuf::from("/tmp/one.png")],
            notes: vec!["No Chrome found".into()],
            session: Some("LinkedIn".into()),
        });
        view.sync(
            HashMap::from([("b".to_string(), "typed".to_string())]),
            HashSet::from(["b".to_string()]),
        );
        let Some(review) = view.review.clone() else {
            panic!("a review is open")
        };
        let dto = review_dto(&review, &view);
        assert_eq!(dto.title, "Lever - jobs.lever.co");
        assert_eq!(
            dto.subtitle,
            "2 fields on this step, 1 to check - session LinkedIn"
        );
        assert_eq!(dto.close_label, "Discard");
        assert_eq!(dto.attached.as_deref(), Some("cv.pdf"));
        assert_eq!(dto.screenshots, 1);
        assert_eq!(dto.notes[0].problem.kind, "noChrome");
        let (a, b) = (&dto.rows[0], &dto.rows[1]);
        assert_eq!(
            (
                a.value.as_str(),
                a.input,
                a.flagged,
                a.remember,
                a.rememberable
            ),
            ("me@example.com", "line", false, false, false)
        );
        assert_eq!(
            (b.value.as_str(), b.flagged, b.remember, b.rememberable),
            ("typed", true, true, true)
        );
        assert_eq!(b.tag.label, "generated");
    }

    #[test]
    fn only_files_of_the_run_are_served() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = Core::open_at(
            crate::engine::Paths::new(dir.path().join("jp")),
            crate::engine::Secrets::memory(),
        )
        .expect("a core");
        let application = app(1, Status::Submitting);
        let mut view = ApplyView::new();
        assert_eq!(
            apply_image_path(&core, &application, &view, "handoff", 0),
            None,
            "no handoff open"
        );
        assert_eq!(
            apply_image_path(&core, &application, &view, "/etc/passwd", 0),
            None,
            "an unknown kind"
        );
        let shot = core.paths.shots_dir.join("a1-handoff-1.png");
        view.phase = Phase::Handoff {
            handoff: Handoff {
                reason: "CAPTCHA".into(),
                captcha: true,
                screenshot: Some(shot.clone()),
            },
            headed: false,
        };
        assert_eq!(
            apply_image_path(&core, &application, &view, "handoff", 0),
            Some(shot)
        );
        let proof = core.paths.shots_dir.join("a1-proof-1.png");
        view.phase = Phase::Done(Some(proof.clone()));
        assert_eq!(
            apply_image_path(&core, &application, &view, "done", 0),
            Some(proof)
        );
        assert_eq!(
            apply_image_path(&core, &application, &view, "filled", 3),
            None,
            "no such screenshot"
        );
        // Whatever else a run might name is not a screenshot of the run.
        view.phase = Phase::Done(Some(PathBuf::from("/etc/passwd")));
        assert_eq!(
            apply_image_path(&core, &application, &view, "done", 0),
            None
        );
        view.phase = Phase::Done(Some(PathBuf::from("/tmp/elsewhere.png")));
        assert_eq!(
            apply_image_path(&core, &application, &view, "done", 0),
            None,
            "only the shots folder"
        );
    }

    #[test]
    fn the_header_offers_next_job_instead_of_the_primary_button_once_sent() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = Core::open_at(
            crate::engine::Paths::new(dir.path().join("jp")),
            crate::engine::Secrets::memory(),
        )
        .expect("a core");
        let none = || JobView {
            label: None,
            error: None,
            cv_epoch: 0,
            cv_problem: None,
        };
        let sent = application_dto(&core, &app(1, Status::Applied), none(), ApplyUi::Idle);
        assert!(sent.primary.is_none() && sent.show_next_job);
        assert_eq!(sent.status.label, "Applied");
        assert_eq!(sent.step_index, 4);

        let draft = application_dto(&core, &app(2, Status::Draft), none(), ApplyUi::Idle);
        assert_eq!(draft.primary.as_ref().map(|p| p.action), Some("generate"));
        assert!(!draft.show_next_job);
        assert_eq!(draft.steps.len(), 5);

        let failed = JobView {
            label: None,
            error: Some("OpenAI: the request timed out".into()),
            cv_epoch: 3,
            cv_problem: None,
        };
        let dto = application_dto(&core, &app(3, Status::Draft), failed, ApplyUi::Idle);
        assert_eq!(dto.error.as_ref().map(|e| e.problem.kind), Some("network"));
        assert_eq!(dto.cv_epoch, 3);
    }

    #[test]
    fn the_header_offers_marks_and_rounds() {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core = Core::open_at(
            crate::engine::Paths::new(dir.path().join("jp")),
            crate::engine::Secrets::memory(),
        )
        .expect("a core");
        let none = || JobView {
            label: None,
            error: None,
            cv_epoch: 0,
            cv_problem: None,
        };
        let keys = |dto: &ApplicationDto| dto.marks.iter().map(|m| m.key).collect::<Vec<_>>();

        let applied = application_dto(&core, &app(1, Status::Applied), none(), ApplyUi::Idle);
        assert_eq!(keys(&applied), ["interview", "offer", "rejected"]);
        assert_eq!(applied.marks[0].label, "Got an interview");
        assert_eq!((applied.stage, applied.outcome, applied.round.as_deref()), ("applied", None, None));
        assert_eq!(applied.round_options, crate::applications::rules::DEFAULT_ROUNDS, "no brief: the usual rounds");

        let mut interview = app(2, Status::Interview);
        interview.interview_round = Some("System design".into());
        interview.insight_json = Some(
            r#"{"interview": {"stages": ["Recruiter call", "Pairing", "Offer"], "notes": "", "found": true}}"#.into(),
        );
        let dto = application_dto(&core, &interview, none(), ApplyUi::Idle);
        assert_eq!(keys(&dto), ["offer", "rejected"]);
        assert_eq!(dto.round_options, ["Recruiter call", "Pairing"]);
        assert_eq!((dto.stage, dto.round.as_deref()), ("inProgress", Some("System design")));

        let offer = application_dto(&core, &app(3, Status::Offer), none(), ApplyUi::Idle);
        assert!(offer.marks.is_empty());
        assert_eq!((offer.stage, offer.outcome), ("done", Some("success")));
        let draft = application_dto(&core, &app(4, Status::Draft), none(), ApplyUi::Idle);
        assert!(draft.marks.is_empty());
        assert_eq!((draft.stage, draft.outcome), ("cv", None));
        assert!(draft.dismissible && draft.dismissed.is_none());
        assert!(!applied.dismissible, "a sent application cannot be dismissed");
        let running = JobView { label: Some("Writing".into()), ..none() };
        assert!(!application_dto(&core, &app(4, Status::Draft), running, ApplyUi::Idle).dismissible);

        let mut gone = app(5, Status::Dismissed);
        gone.dismiss_reason = Some(" ".into());
        gone.dismissed_at = Some(Utc.with_ymd_and_hms(2026, 11, 14, 9, 30, 0).unwrap());
        let dto = application_dto(&core, &gone, none(), ApplyUi::Idle);
        assert!(dto.primary.is_none() && !dto.dismissible && !dto.show_next_job);
        assert_eq!(
            dto.dismissed,
            Some(DismissalDto { reason: None, at: Some("2026-11-14T09:30:00+00:00".into()) })
        );
        assert_eq!(dto.status.key, "dismissed");
    }

    #[test]
    fn the_board_counts_no_dismissed_application() {
        let apps = vec![app(1, Status::Draft), app(2, Status::Dismissed)];
        let dto = board_dto(&apps, &[], &Default::default(), &|_| false);
        assert_eq!(dto.count, 1);
        assert!(dto.stages.iter().all(|s| s.cards.iter().all(|c| c.id != 2)));
    }
}
