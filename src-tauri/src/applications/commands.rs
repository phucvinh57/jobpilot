//! Application services of this context: thin `#[tauri::command]`s named `applications_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Long work (generate, revise, filling and submitting a form) returns when it is done; progress goes
//! out as `applications:progress` / `applications:apply-progress`, and the moments the stored data
//! moves as `shared:data-changed` (scope `applications`). The state of a running job or of an open
//! review lives in [`Applications`], not in the web app.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{bail, Context as _};
use jobpilot_engine::apply::{self, ApplyOptions, Live, Stage};
use jobpilot_engine::browser::fetch::generate_with_browser;
use jobpilot_engine::pipeline::{self, Progress};
use jobpilot_engine::store::Status;
use jobpilot_engine::Core;
use tauri::ipc::{Invoke, Response};
use tauri::{AppHandle, State};

use super::dto::{
    application_dto, apply_dto, apply_image_path, board_dto, cv_dto, jd_dto, primary_of,
    require_application, timeline_dto, ApplicationDto, ApplyDto, ApplyProgressDto, BoardDto, CvDto,
    JdDto, PrimaryDto, ProgressDto, RenderedDto, TimelineDto, ACTIVITY_ROWS,
};
use super::files::{self, generate_plan, GeneratePlan};
use super::rules::{can_submit_manually, dismissible, marks_for, ApplyUi, Mark};
use super::state::{Applications, Confirm, Job, Phase};
use crate::shared::events::{data_changed, emit, Scope};
use crate::shared::{run, run_sync, CommandError, CommandResult, DismissedDto, RestoredDto};

/// `applications:progress`.
pub const PROGRESS: &str = "applications:progress";
/// `applications:apply-progress`.
pub const APPLY_PROGRESS: &str = "applications:apply-progress";

fn handles(
    core: &State<'_, Arc<Core>>,
    state: &State<'_, Applications>,
) -> (Arc<Core>, Applications) {
    (core.inner().clone(), state.inner().clone())
}

/// What the core is doing for `id` when this context has nothing open: an open review or a handoff.
fn live_ui(core: &Core, id: i64) -> ApplyUi {
    match apply::live(core, id) {
        Live::Idle => ApplyUi::Idle,
        Live::Review(r) => ApplyUi::Review {
            complete: r.complete,
            dry_run: r.dry_run,
            missing: r.missing_required(),
        },
        Live::Handoff { .. } => ApplyUi::Handoff,
    }
}

// ------------------------------------------------------------------ Jobs page

/// The Jobs page's application stages (CVs, Applied, In progress, Done) and the latest events across all of them.
#[tauri::command]
pub async fn applications_board(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
) -> CommandResult<BoardDto> {
    let (core, state) = handles(&core, &state);
    run_sync(move || {
        let apps = core.store.list_applications()?;
        // The newest events of every application, not only of the ones changed last.
        let events = core.store.recent_events(ACTIVITY_ROWS)?;
        let busy = state.busy_ids();
        Ok(board_dto(&apps, &events, &busy, &|a| {
            a.cv_yaml_path().is_file()
        }))
    })
    .await
}

// ------------------------------------------------------------------ Application screen

/// The header of one application; `None` when it does not exist (any more).
#[tauri::command]
pub async fn applications_get(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<Option<ApplicationDto>> {
    let (core, state) = handles(&core, &state);
    run_sync(move || {
        let Some(app) = core.store.get_application(id)? else {
            return Ok(None);
        };
        let job = state.job_view(id);
        let apply_ui = state.with_apply(id, |v| v.ui(&|| live_ui(&core, id)));
        Ok(Some(application_dto(&core, &app, job, apply_ui)))
    })
    .await
}

/// The header of `id` after a change made from it (the application must exist).
fn header(core: &Core, state: &Applications, id: i64) -> anyhow::Result<ApplicationDto> {
    let app = require_application(core, id)?;
    let job = state.job_view(id);
    let apply_ui = state.with_apply(id, |v| v.ui(&|| live_ui(core, id)));
    Ok(application_dto(core, &app, job, apply_ui))
}

/// Mark (or clear, with `None` or blank text) the interview round; only while the status is Interview.
#[tauri::command]
pub async fn applications_set_round(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    round: Option<String>,
) -> CommandResult<ApplicationDto> {
    let (core, state) = handles(&core, &state);
    let dto = run_sync(move || {
        core.store.set_interview_round(id, round.as_deref())?;
        tracing::info!(app = id, "interview round marked");
        header(&core, &state, id)
    })
    .await?;
    data_changed(&app, Scope::Applications);
    Ok(dto)
}

/// A manual marker on a sent application: `interview` ("Got an interview"), `offer` or `rejected`.
/// Only the marks the header offers (`rules::marks_for`) are accepted; the status only moves forward.
#[tauri::command]
pub async fn applications_mark(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    mark: String,
) -> CommandResult<ApplicationDto> {
    let Some(mark) = Mark::parse(&mark) else {
        return Err(CommandError::new(format!("unknown mark `{mark}`")));
    };
    let (core, state) = handles(&core, &state);
    let dto = run_sync(move || {
        let current = require_application(&core, id)?.status;
        if !marks_for(current).contains(&mark) {
            bail!(
                "an application in {} cannot be marked {}",
                current.label(),
                mark.label()
            );
        }
        core.store.set_status(id, mark.status(), "marked by you")?;
        tracing::info!(app = id, mark = mark.key(), "application marked");
        header(&core, &state, id)
    })
    .await?;
    data_changed(&app, Scope::Applications);
    Ok(dto)
}

/// "Dismiss" a job in the CVs stage, with an optional reason; `exclude_company` also keeps the company
/// out of later searches (and dismisses its leads in the queue). Refused while a job runs for it or its
/// form is being filled or reviewed (`rules::dismissible`). The application is kept as `Dismissed`:
/// the Leads stage's "Dismissed" list shows it, and Restore brings it back.
#[tauri::command]
pub async fn applications_dismiss(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    reason: String,
    exclude_company: bool,
) -> CommandResult<DismissedDto> {
    let (core, state) = handles(&core, &state);
    let out = run(async move {
        // Held until the job is dismissed: no generate / revise starts in between. The engine holds
        // the apply gate and the "Prepare CV" gate the same way while the status changes.
        let Some(_slot) = state.hold_slot(id) else {
            bail!("the CV is still being written: wait for it to finish");
        };
        let current = require_application(&core, id)?.status;
        let apply_ui = state.with_apply(id, |v| v.ui(&|| live_ui(&core, id)));
        if !dismissible(current, false, &apply_ui) {
            if !current.can_advance_to(Status::Dismissed) {
                bail!("an application in {} cannot be dismissed", current.label());
            }
            bail!("its application form is open: close it first");
        }
        Ok(DismissedDto::from(
            core.dismiss_application(id, &reason, exclude_company).await?,
        ))
    })
    .await?;
    data_changed(&app, Scope::Applications);
    data_changed(&app, Scope::Leads);
    Ok(out)
}

/// "Restore" a dismissed job from its drawer: back at the status it had before (the header is read
/// again on `shared:data-changed`). Says whether its company is still excluded.
#[tauri::command]
pub async fn applications_restore(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    id: i64,
) -> CommandResult<RestoredDto> {
    let core = core.inner().clone();
    let dto = run_sync(move || Ok(RestoredDto::from(core.restore_application(id)?))).await?;
    data_changed(&app, Scope::Applications);
    data_changed(&app, Scope::Leads);
    Ok(dto)
}

/// A progress step of a pipeline job: the header's line moves, and a new stage may have renamed the row.
fn progress_sink(
    app: &AppHandle,
    state: &Applications,
) -> impl Fn(Progress) + Send + Sync + 'static {
    let (app, state) = (app.clone(), state.clone());
    move |p: Progress| {
        let note = state.on_progress(&p);
        emit(
            &app,
            PROGRESS,
            ProgressDto {
                app_id: p.app_id,
                label: note.label,
            },
        );
        if note.stage_changed {
            data_changed(&app, Scope::Applications);
        }
    }
}

/// Record how the job ended; the web app reads the failure from the header.
fn end_job(state: &Applications, id: i64, result: CommandResult<()>) -> CommandResult<()> {
    state.finish_job(
        id,
        result.as_ref().map(|_| ()).map_err(|e| e.message.clone()),
    );
    result
}

async fn generate(
    app: AppHandle,
    core: Arc<Core>,
    state: Applications,
    id: i64,
) -> CommandResult<()> {
    // One job per application at a time.
    if !state.begin_job(id, Job::Generate, None) {
        return Ok(());
    }
    tracing::info!(app = id, "generate started");
    data_changed(&app, Scope::Applications);
    let sink = progress_sink(&app, &state);
    let kept = state.clone();
    let result = run(async move {
        let application = require_application(&core, id)?;
        match generate_plan(&application, kept.cached_input(id).as_ref()) {
            GeneratePlan::Regenerate => pipeline::regenerate(&core, id, sink).await.map(|_| ()),
            GeneratePlan::Reingest(input) => {
                kept.remember_input(id, input.clone());
                generate_with_browser(&core, Some(id), input, sink)
                    .await
                    .map(|_| ())
            }
            GeneratePlan::Unavailable(message) => bail!(message),
        }
    })
    .await;
    end_job(&state, id, result)
}

async fn revise(
    app: AppHandle,
    core: Arc<Core>,
    state: Applications,
    id: i64,
    instruction: String,
    yaml: Option<String>,
) -> CommandResult<()> {
    if instruction.trim().is_empty() || !state.begin_job(id, Job::Revise(instruction.clone()), None)
    {
        return Ok(());
    }
    tracing::info!(app = id, "revise started");
    data_changed(&app, Scope::Applications);
    let sink = progress_sink(&app, &state);
    let result = run(async move {
        // `revise` starts from the CV on disk, so write the editor text first.
        if let Some(yaml) = yaml {
            files::write_cv(&core, id, yaml).await?;
        }
        pipeline::revise(&core, id, &instruction, sink)
            .await
            .map(|_| ())
    })
    .await;
    end_job(&state, id, result)
}

/// "Generate" on a Draft without a CV: the CV is made from the stored posting (or from where it came from).
#[tauri::command]
pub async fn applications_generate(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    generate(app, core, state, id).await
}

/// "Ask AI": a targeted change to the CV. `yaml` is the editor text, written to `cv.yaml` first.
#[tauri::command]
pub async fn applications_revise(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    instruction: String,
    yaml: Option<String>,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    revise(app, core, state, id, instruction, yaml).await
}

/// Retry: run the failed job again (Generate, or the same "Ask AI" instruction).
#[tauri::command]
pub async fn applications_retry(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    match state.last_job(id) {
        Some(Job::Revise(instruction)) => revise(app, core, state, id, instruction, None).await,
        Some(Job::Approve) => approve_cv(core, state, id).await,
        _ => generate(app, core, state, id).await,
    }
}

/// Hide the failure of the last job.
#[tauri::command]
pub async fn applications_dismiss_error(
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    state.dismiss_error(id);
    Ok(())
}

/// "Approve CV": Draft -> CV ready. The one place the user's approval is recorded.
#[tauri::command]
pub async fn applications_approve_cv(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    approve_cv(core, state, id).await
}

async fn approve_cv(core: Arc<Core>, state: Applications, id: i64) -> CommandResult<()> {
    let job = state.job_view(id);
    if job.label.is_some() {
        return Err(CommandError::new(
            "the CV is still being written: wait for it to finish",
        ));
    }
    if job.cv_problem.is_some() {
        return Err(CommandError::new(
            "The CV has an error: fix it before approving",
        ));
    }
    let kept = state.clone();
    let result = run_sync(move || {
        pipeline::require_open(&core, id)?;
        core.store.set_status(id, Status::CvReady, "CV approved")
    })
    .await;
    kept.approval_ended(id, result.as_ref().err().map(|e| e.message.clone()));
    result
}

// ------------------------------------------------------------------ JD tab

/// The posting text, its source, the requirements and the company brief.
#[tauri::command]
pub async fn applications_jd(core: State<'_, Arc<Core>>, id: i64) -> CommandResult<JdDto> {
    let core = core.inner().clone();
    run_sync(move || Ok(jd_dto(&require_application(&core, id)?))).await
}

// ------------------------------------------------------------------ CV tab

/// `cv.yaml`, the checklist of what the posting asks for, and whether the facts trace to the profile.
#[tauri::command]
pub async fn applications_cv(core: State<'_, Arc<Core>>, id: i64) -> CommandResult<CvDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let app = require_application(&core, id)?;
        let files = files::load_cv_files(&app);
        let clean = files::guard_clean(&core, files.yaml.as_deref());
        // A read never clears a CV problem: only a save that compiles (or a new CV) does, so a
        // broken CV cannot be approved after switching tabs.
        Ok(cv_dto(&app, files, clean))
    })
    .await
}

/// Write `cv.yaml`, then parse, fact-check and render it. A CV that does not parse or compile is an
/// error (the edit is still saved); fact-guard findings are warnings.
#[tauri::command]
pub async fn applications_cv_save(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    yaml: String,
) -> CommandResult<RenderedDto> {
    let (core, state) = handles(&core, &state);
    let kept = state.clone();
    let result = run(async move { files::save_and_render(&core, id, yaml).await }).await;
    kept.set_cv_problem(id, result.as_ref().err().map(|e| e.message.clone()));
    result.map(|r| RenderedDto {
        page_count: r.page_count,
        warnings: r.warnings,
    })
}

/// The PNG of preview page `page` (0-based), as bytes for a blob URL.
#[tauri::command]
pub async fn applications_cv_page(
    core: State<'_, Arc<Core>>,
    id: i64,
    page: usize,
) -> CommandResult<Response> {
    let core = core.inner().clone();
    let bytes = run_sync(move || files::read_page(&require_application(&core, id)?, page)).await?;
    Ok(Response::new(bytes))
}

// ------------------------------------------------------------------ Timeline tab

/// The events (newest first) and the emails waiting for an accept / dismiss.
#[tauri::command]
pub async fn applications_timeline(
    core: State<'_, Arc<Core>>,
    id: i64,
) -> CommandResult<TimelineDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let events = core.store.list_events(id)?;
        let reviews = core.emails_needing_review(Some(id))?;
        Ok(timeline_dto(events, &reviews))
    })
    .await
}

/// Accept (apply the suggested status) or dismiss a low-confidence email.
#[tauri::command]
pub async fn applications_resolve_email(
    core: State<'_, Arc<Core>>,
    email_id: String,
    accept: bool,
) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || {
        if accept {
            core.accept_email(&email_id).map(|_| ())
        } else {
            core.dismiss_email(&email_id)
        }
    })
    .await
}

// ------------------------------------------------------------------ Apply tab

/// What the Apply tab shows; an open review in the core (switching tabs, selecting another
/// application) is taken over first.
#[tauri::command]
pub async fn applications_apply_state(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<ApplyDto> {
    let (core, state) = handles(&core, &state);
    run_sync(move || {
        let app = require_application(&core, id)?;
        let view = state.with_apply(id, |v| {
            v.adopt_live(apply::live(&core, id));
            v.clone()
        });
        Ok(apply_dto(&core, &app, &view, state.dry_run()))
    })
    .await
}

/// The "Dry run" box: fill the form but never submit.
#[tauri::command]
pub async fn applications_apply_set_dry_run(
    state: State<'_, Applications>,
    on: bool,
) -> CommandResult<()> {
    state.set_dry_run(on);
    Ok(())
}

/// What the user typed in the review so far (a full snapshot), so the primary button knows whether
/// required fields are still empty. Returns the primary action as it is now.
#[tauri::command]
pub async fn applications_apply_sync(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    typed: HashMap<String, String>,
    remember: Vec<String>,
) -> CommandResult<Option<PrimaryDto>> {
    let (core, state) = handles(&core, &state);
    run_sync(move || {
        state.with_apply(id, |v| v.sync(typed, remember.into_iter().collect()));
        let app = require_application(&core, id)?;
        let job = state.job_view(id);
        let apply_ui = state.with_apply(id, |v| v.ui(&|| live_ui(&core, id)));
        Ok(primary_of(&core, &app, &job, &apply_ui).map(PrimaryDto::from))
    })
    .await
}

/// One line of the form run's progress.
fn apply_sink(
    app: &AppHandle,
    state: &Applications,
    id: i64,
) -> impl Fn(Stage) + Send + Sync + 'static {
    let (app, state) = (app.clone(), state.clone());
    move |stage: Stage| {
        let lines = state.with_apply(id, |v| {
            v.push_progress(stage.label());
            v.progress.clone()
        });
        emit(&app, APPLY_PROGRESS, ApplyProgressDto { app_id: id, lines });
    }
}

/// Open the form in a tab of the user's browser, read and answer its fields, fill it. Never submits.
async fn prepare(
    app: AppHandle,
    core: Arc<Core>,
    state: Applications,
    id: i64,
) -> CommandResult<()> {
    if !state.with_apply(id, |v| v.begin_prepare()) {
        return Ok(());
    }
    data_changed(&app, Scope::Applications);
    let options = ApplyOptions {
        dry_run: state.dry_run(),
    };
    let sink = apply_sink(&app, &state, id);
    let result = run(async move { apply::prepare(&core, id, options, sink).await }).await;
    state.with_apply(id, |v| {
        v.busy = None;
        v.finish_prepare(result.map_err(|e| e.message));
    });
    Ok(())
}

/// "Review & Submit": open and fill the form for a review. Nothing is sent.
#[tauri::command]
pub async fn applications_apply_prepare(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    prepare(app, core, state, id).await
}

/// The explicit confirmation, the click on Submit (or Continue on a form with more steps) while
/// the filled form is open. `typed` / `remember` are what the review shows right now. Anything
/// that is not an open review opens and fills the form instead (never submits).
#[tauri::command]
pub async fn applications_apply_confirm(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    typed: HashMap<String, String>,
    remember: Vec<String>,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    state.with_apply(id, |v| v.sync(typed, remember.into_iter().collect()));
    let phase = state.with_apply(id, |v| v.phase.clone());
    match phase {
        Phase::Preparing | Phase::Submitting => return Ok(()),
        Phase::Review => {}
        Phase::Idle | Phase::Done(_) | Phase::Handoff { .. } => {
            return prepare(app, core, state, id).await
        }
    }
    match state.with_apply(id, |v| v.begin_confirm()) {
        Confirm::Ignore | Confirm::Blocked => Ok(()),
        Confirm::Submit(edits) => {
            tracing::info!(app = id, "submit confirmed in the review");
            data_changed(&app, Scope::Applications);
            let result = run(async move { apply::submit(&core, id, edits).await }).await;
            state.with_apply(id, |v| v.finish_submit(result.map_err(|e| e.message)));
            Ok(())
        }
        Confirm::Advance(edits) => {
            data_changed(&app, Scope::Applications);
            let sink = apply_sink(&app, &state, id);
            let result = run(async move { apply::advance(&core, id, edits, sink).await }).await;
            state.with_apply(id, |v| v.finish_prepare(result.map_err(|e| e.message)));
            Ok(())
        }
    }
}

/// Close the browser and drop the review without submitting.
#[tauri::command]
pub async fn applications_apply_discard(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    state.with_apply(id, |v| v.busy = Some("Closing the browser..."));
    data_changed(&app, Scope::Applications);
    let result = run(async move { apply::discard(&core, id).await }).await;
    state.with_apply(id, |v| v.finish_discard(result.map_err(|e| e.message)));
    Ok(())
}

/// After a handoff: bring the form's tab in the user's Chrome to the front, to finish by hand.
#[tauri::command]
pub async fn applications_apply_open_visible(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    state.with_apply(id, |v| {
        v.busy = Some("Opening the browser...");
        v.error = None;
    });
    data_changed(&app, Scope::Applications);
    let result = run(async move { apply::open_visible(&core, id).await }).await;
    state.with_apply(id, |v| v.finish_open_visible(result.map_err(|e| e.message)));
    Ok(())
}

/// "Mark as applied / failed" after finishing by hand in the browser.
#[tauri::command]
pub async fn applications_apply_mark(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    applied: bool,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    let handoff = state.with_apply(id, |v| matches!(v.phase, Phase::Handoff { .. }));
    if !handoff {
        // The user applied on the site by hand: only for an approved, unsent CV. A failure here is the
        // mark's own (a toast without Retry), not the form run's: it never goes into the Apply view.
        let check = core.clone();
        let status = run_sync(move || Ok(require_application(&check, id)?.status)).await?;
        if !can_submit_manually(status) {
            return Err(CommandError::new("approve the CV before marking the application as sent"));
        }
    }
    state.with_apply(id, |v| v.busy = Some("Saving..."));
    data_changed(&app, Scope::Applications);
    let result = run(async move { apply::finish_manually(&core, id, applied).await }).await;
    if handoff {
        state.with_apply(id, |v| v.finish_mark(result.map_err(|e| e.message)));
        return Ok(());
    }
    state.with_apply(id, |v| v.finish_mark(Ok(())));
    data_changed(&app, Scope::Applications);
    result
}

/// "Save CV as PDF": copy the rendered `cv.pdf` to the path the user chose in the save dialog.
#[tauri::command]
pub async fn applications_cv_export(core: State<'_, Arc<Core>>, id: i64, path: String) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || {
        let app = require_application(&core, id)?;
        if !files::pdf_is_current(&app) {
            bail!("the CV PDF is missing or older than the last edit: save the CV again to render it");
        }
        let from = app.cv_pdf_path();
        let mut to = std::path::PathBuf::from(path);
        if !to.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
            to.as_mut_os_string().push(".pdf");
        }
        std::fs::copy(&from, &to).with_context(|| format!("saving the CV to {}", to.display()))?;
        tracing::info!(app = id, "cv exported as pdf");
        Ok(())
    })
    .await
}

/// Save the apply URL the user typed (http/https only).
#[tauri::command]
pub async fn applications_apply_save_url(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    url: String,
) -> CommandResult<()> {
    let (core, state) = handles(&core, &state);
    run_sync(move || {
        let error = core.set_apply_url(id, &url).err().map(|e| format!("{e:#}"));
        state.with_apply(id, |v| v.error = error);
        Ok(())
    })
    .await
}

/// A screenshot of the run (see `apply_image_path` for the kinds), as bytes for a blob URL.
#[tauri::command]
pub async fn applications_apply_image(
    core: State<'_, Arc<Core>>,
    state: State<'_, Applications>,
    id: i64,
    kind: String,
    index: usize,
) -> CommandResult<Response> {
    let (core, state) = handles(&core, &state);
    let bytes = run_sync(move || {
        let app = require_application(&core, id)?;
        let view = state.with_apply(id, |v| v.clone());
        let path = apply_image_path(&core, &app, &view, &kind, index)
            .context("there is no such screenshot")?;
        std::fs::read(&path).with_context(|| format!("reading {}", path.display()))
    })
    .await?;
    Ok(Response::new(bytes))
}

/// This context's commands; `lib.rs` routes every `applications_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        applications_board,
        applications_get,
        applications_set_round,
        applications_mark,
        applications_cv_export,
        applications_dismiss,
        applications_restore,
        applications_generate,
        applications_revise,
        applications_retry,
        applications_dismiss_error,
        applications_approve_cv,
        applications_jd,
        applications_cv,
        applications_cv_save,
        applications_cv_page,
        applications_timeline,
        applications_resolve_email,
        applications_apply_state,
        applications_apply_set_dry_run,
        applications_apply_sync,
        applications_apply_prepare,
        applications_apply_confirm,
        applications_apply_discard,
        applications_apply_open_visible,
        applications_apply_mark,
        applications_apply_save_url,
        applications_apply_image,
    ]
}
