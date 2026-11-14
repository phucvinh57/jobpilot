//! Application services of this context: thin `#[tauri::command]`s named `discovery_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Events (`discovery:<event>`):
//! - [`PREPARED`] (`PrepareOutcomeDto`): one lead's "Prepare CV" ended, while the batch goes on.
//! - [`SEARCH_FINISHED`] (`SearchFinishedDto`): the daily search ended (see `daily.rs`).

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::Core;
use tauri::ipc::Invoke;
use tauri::{AppHandle, State};

use super::dto::{
    AddedLeadDto, BoardHintDto, DismissedDto, JobSourcesDto, LeadInput, PrepareOutcomeDto, QueueDto,
    RunSummaryDto, SessionOptionDto, SourceDto, SourceKindDto,
};
use super::rules;
use crate::shared::{events, run, run_sync, CommandResult};

/// One lead's "Prepare CV" ended: payload `PrepareOutcomeDto`.
pub const PREPARED: &str = "discovery:prepared";
/// The daily search ended: payload `SearchFinishedDto`.
pub const SEARCH_FINISHED: &str = "discovery:search-finished";

// ------------------------------------------------------------------ Today: reading

/// Everything Today shows: the leads best match first, failed CVs, the sources, the ones paused
/// for the user and the last search.
#[tauri::command]
pub async fn discovery_queue(core: State<'_, Arc<Core>>) -> CommandResult<QueueDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let snapshot = core.queue_snapshot()?;
        Ok(QueueDto::new(&snapshot, core.discover_running()?))
    })
    .await
}

/// The skill names of the master profile, to tell the skills a posting asks for that the profile
/// has from those it lacks (empty when the profile cannot be read). The profile is a file the user
/// edits outside the app, so Today reads it when it is shown.
#[tauri::command]
pub async fn discovery_profile_skills(core: State<'_, Arc<Core>>) -> CommandResult<Vec<String>> {
    let core = core.inner().clone();
    run_sync(move || Ok(core.profile_skills())).await
}

// ------------------------------------------------------------------ Today: the link box

/// What a pasted link, path or text is. `None` for a blank box.
#[tauri::command]
pub async fn discovery_parse_input(raw: String) -> CommandResult<Option<LeadInput>> {
    run_sync(move || Ok(rules::parse_input(&raw))).await
}

/// A file dropped on the window: a lead input, or why the file is refused (as the error message).
#[tauri::command]
pub async fn discovery_file_input(path: String) -> CommandResult<LeadInput> {
    run_sync(move || rules::file_input(&[PathBuf::from(path)]).map_err(anyhow::Error::msg)).await
}

/// Read the posting behind `input`, score it, brief it and add it as a Lead (never an Application:
/// Prepare CV does that). Takes seconds to about a minute. A job that is a lead already is returned
/// as it stands (check `state`).
#[tauri::command]
pub async fn discovery_add_lead(
    core: State<'_, Arc<Core>>,
    input: LeadInput,
) -> CommandResult<AddedLeadDto> {
    let core = core.inner().clone();
    run(async move {
        let lead = core.add_lead(input.into_jd()).await?;
        tracing::info!(lead = lead.id, "lead added from the link box");
        Ok(AddedLeadDto::from(&lead))
    })
    .await
}

// ------------------------------------------------------------------ Today: acting on leads

/// "Prepare CV" on `lead_ids`, one after the other (the core queues across calls too). Each lead's
/// outcome goes out as the `discovery:prepared` event as soon as it is known; the command returns
/// every outcome when the batch is done. One failing does not stop the next.
#[tauri::command]
pub async fn discovery_prepare_cvs(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    lead_ids: Vec<i64>,
) -> CommandResult<Vec<PrepareOutcomeDto>> {
    let core = core.inner().clone();
    run(async move {
        let mut outcomes = Vec::with_capacity(lead_ids.len());
        core.prepare_cvs_each(&lead_ids, |id, result| {
            if let Ok(application) = &result {
                tracing::info!(lead = id, app = application.id, "CV prepared");
            }
            let outcome = PrepareOutcomeDto::new(id, result);
            events::emit(&app, PREPARED, outcome.clone());
            outcomes.push(outcome);
        })
        .await;
        Ok(outcomes)
    })
    .await
}

/// Forget a failed "Prepare CV" (its application stays as a Draft).
#[tauri::command]
pub async fn discovery_forget_failure(
    core: State<'_, Arc<Core>>,
    lead_id: i64,
) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.prepare_forget(lead_id)).await
}

/// "Dismiss": the lead leaves the queue, with an optional reason; `exclude_company` also keeps the
/// company out of later searches (and dismisses its other leads, except those being prepared).
#[tauri::command]
pub async fn discovery_dismiss_lead(
    core: State<'_, Arc<Core>>,
    lead_id: i64,
    reason: String,
    exclude_company: bool,
) -> CommandResult<DismissedDto> {
    let core = core.inner().clone();
    run(async move {
        Ok(DismissedDto::from(
            core.dismiss_lead_with_reason(lead_id, &reason, exclude_company)
                .await?,
        ))
    })
    .await
}

// ------------------------------------------------------------------ Today: searching

/// "Search now": every enabled job source. Returns when each source is done, paused for the user or
/// failed.
#[tauri::command]
pub async fn discovery_search_now(core: State<'_, Arc<Core>>) -> CommandResult<RunSummaryDto> {
    let core = core.inner().clone();
    run(async move { Ok(RunSummaryDto::from(&core.discover_now().await?)) }).await
}

/// "Open browser" for a source paused at a login, captcha or block page: a visible window on that
/// page, in the source's bound session if it has one. The user deals with the page; the app never
/// does.
#[tauri::command]
pub async fn discovery_open_browser(
    core: State<'_, Arc<Core>>,
    thread: String,
) -> CommandResult<()> {
    let core = core.inner().clone();
    run(async move { core.discover_open_browser(&thread).await }).await
}

/// "Continue" a paused source (after the user dealt with the page) or retry a failed one.
#[tauri::command]
pub async fn discovery_resume(core: State<'_, Arc<Core>>, thread: String) -> CommandResult<()> {
    let core = core.inner().clone();
    run(async move { core.discover_resume(&thread).await.map(|_| ()) }).await
}

/// "Skip" a paused source for this run (it stays enabled and is searched by the next run).
#[tauri::command]
pub async fn discovery_skip(core: State<'_, Arc<Core>>, thread: String) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.discover_cancel(&thread)).await
}

// ------------------------------------------------------------------ Settings > Job sources

/// Sources with their last run, the sessions they can be read in, the paused ones and the daily
/// setting.
#[tauri::command]
pub async fn discovery_job_sources(core: State<'_, Arc<Core>>) -> CommandResult<JobSourcesDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let waiting = match core.discover_waiting() {
            Ok(list) => list.into_iter().map(|w| w.source_id).collect(),
            Err(e) => {
                tracing::warn!("reading the paused sources failed: {e:#}");
                Vec::new()
            }
        };
        Ok(JobSourcesDto {
            sources: core.job_sources()?.iter().map(SourceDto::from).collect(),
            waiting,
            sessions: core
                .sessions()
                .list()?
                .iter()
                .map(SessionOptionDto::from)
                .collect(),
            daily: core.discover_daily_enabled()?,
            kinds: crate::engine::discover::JOB_SOURCE_KINDS
                .iter()
                .map(|(kind, label)| SourceKindDto {
                    kind,
                    label,
                    hint: crate::engine::discover::source_url_hint(kind),
                })
                .collect(),
        })
    })
    .await
}

/// Add a job source. The adapter of `kind` must accept `url`; a blank `name` becomes the board name.
#[tauri::command]
pub async fn discovery_add_source(
    core: State<'_, Arc<Core>>,
    kind: String,
    name: String,
    url: String,
) -> CommandResult<SourceDto> {
    let core = core.inner().clone();
    run_sync(move || {
        Ok(SourceDto::from(
            &core.add_job_source(&kind, &name, &url, None)?,
        ))
    })
    .await
}

/// The job boards a careers page points to (one plain read of the page, no browser), for the add
/// form to offer as sources instead. Read-only: nothing is added and no board is read.
#[tauri::command]
pub async fn discovery_suggest_boards(
    core: State<'_, Arc<Core>>,
    url: String,
) -> CommandResult<Vec<BoardHintDto>> {
    let core = core.inner().clone();
    run(async move {
        let hints = match super::demo::board_hints(&url) {
            Some(demo) => demo,
            None => core.suggest_boards(&url).await?,
        };
        Ok(hints.iter().map(BoardHintDto::from).collect())
    })
    .await
}

#[tauri::command]
pub async fn discovery_set_source_enabled(
    core: State<'_, Arc<Core>>,
    id: i64,
    enabled: bool,
) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.set_job_source_enabled(id, enabled)).await
}

/// Bind (or with `None` unbind) the managed browser session a source is read in.
#[tauri::command]
pub async fn discovery_set_source_session(
    core: State<'_, Arc<Core>>,
    id: i64,
    session_id: Option<i64>,
) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.set_job_source_session(id, session_id)).await
}

/// Remove a source. A run of it that waits for the user is skipped; the leads it found stay.
#[tauri::command]
pub async fn discovery_remove_source(core: State<'_, Arc<Core>>, id: i64) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.remove_job_source(id)).await
}

/// "Search daily while JobPilot is open" (the daily timer in `daily.rs` honours it).
#[tauri::command]
pub async fn discovery_set_daily(core: State<'_, Arc<Core>>, on: bool) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.set_discover_daily(on)).await
}

/// This context's commands; `main.rs` routes every `discovery_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        discovery_queue,
        discovery_profile_skills,
        discovery_parse_input,
        discovery_file_input,
        discovery_add_lead,
        discovery_prepare_cvs,
        discovery_forget_failure,
        discovery_dismiss_lead,
        discovery_search_now,
        discovery_open_browser,
        discovery_resume,
        discovery_skip,
        discovery_job_sources,
        discovery_add_source,
        discovery_suggest_boards,
        discovery_set_source_enabled,
        discovery_set_source_session,
        discovery_remove_source,
        discovery_set_daily,
    ]
}
