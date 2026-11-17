//! Application services of this context: thin `#[tauri::command]`s named `discovery_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Events (`discovery:<event>`):
//! - [`PREPARED`] (`PrepareOutcomeDto`): one lead's "Prepare CV" ended, while the batch goes on.
//! - [`SEARCH_FINISHED`] (`SearchFinishedDto`): the daily search ended (see `daily.rs`).

use std::path::PathBuf;
use std::sync::Arc;

use crate::engine::discover::suggest::SuggestOutcome;
use crate::engine::discover::DismissedKind;
use crate::engine::Core;
use tauri::ipc::Invoke;
use tauri::{AppHandle, State};

use super::dto::{
    AddAllDto, AddedLeadDto, BoardHintDto, DismissedJobDto, JobSourcesDto, LeadInput, PrepareOutcomeDto,
    QueueDto, RunSummaryDto, SessionOptionDto, SourceDto, SourceKindDto, SourceSuggestionDto,
    SuggestRunDto,
};
use super::rules;
use crate::shared::events::Scope;
use crate::shared::{events, run, run_sync, CommandError, CommandResult, DismissedDto, RestoredDto};

/// One lead's "Prepare CV" ended: payload `PrepareOutcomeDto`.
pub const PREPARED: &str = "discovery:prepared";
/// The daily search ended: payload `SearchFinishedDto`.
pub const SEARCH_FINISHED: &str = "discovery:search-finished";

// ------------------------------------------------------------------ Leads: reading

/// Everything the Leads stage shows: the leads best match first, failed CVs, the sources, the ones
/// paused for the user and the last search.
#[tauri::command]
pub async fn discovery_queue(core: State<'_, Arc<Core>>) -> CommandResult<QueueDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let snapshot = core.queue_snapshot()?;
        Ok(QueueDto::new(&snapshot, core.discover_running()?))
    })
    .await
}

/// The skill names of the profile, to tell the skills a posting asks for that the profile has from
/// those it lacks (empty when there is none or it cannot be read).
#[tauri::command]
pub async fn discovery_profile_skills(core: State<'_, Arc<Core>>) -> CommandResult<Vec<String>> {
    let core = core.inner().clone();
    run_sync(move || Ok(core.profile_skills())).await
}

// ------------------------------------------------------------------ Leads: the link box

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

// ------------------------------------------------------------------ Leads: acting on them

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

/// The "Dismissed" list: every dismissed job (leads the user dismissed, postings the preferences
/// filtered out, jobs dismissed from the CVs stage), most recently dismissed first.
#[tauri::command]
pub async fn discovery_dismissed(core: State<'_, Arc<Core>>) -> CommandResult<Vec<DismissedJobDto>> {
    let core = core.inner().clone();
    run_sync(move || Ok(core.dismissed_jobs()?.iter().map(DismissedJobDto::from).collect())).await
}

/// "Restore" a dismissed job: a lead (`kind` `lead`) goes back to the queue (hidden while the
/// saved preferences filter it out: `hidden` says why), an application (`application`) back to the
/// status it had. An excluded company stays excluded.
#[tauri::command]
pub async fn discovery_restore(
    app: AppHandle,
    core: State<'_, Arc<Core>>,
    kind: String,
    id: i64,
) -> CommandResult<RestoredDto> {
    let Some(kind) = DismissedKind::parse(&kind) else {
        return Err(CommandError::new(format!("unknown kind `{kind}`")));
    };
    let core = core.inner().clone();
    let out = run_sync(move || core.restore_dismissed(kind, id)).await?;
    events::data_changed(&app, Scope::Leads);
    if kind == DismissedKind::Application {
        events::data_changed(&app, Scope::Applications);
    }
    Ok(RestoredDto::from(out))
}

// ------------------------------------------------------------------ Leads: searching

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
        let suggest_waiting = match core.suggest_waiting() {
            Ok(waiting) => waiting.is_some(),
            Err(e) => {
                tracing::warn!("reading the paused source suggestion search failed: {e:#}");
                false
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
            // Suggestions are a nicety: an unreadable list never hides the sources.
            suggestions: suggestion_dtos(&core).unwrap_or_else(|e| {
                tracing::warn!("reading the source suggestions failed: {e:#}");
                Vec::new()
            }),
            suggest_waiting,
            suggest_running: core.suggest_running().unwrap_or_else(|e| {
                tracing::warn!("reading the source suggestion search failed: {e:#}");
                false
            }),
        })
    })
    .await
}

/// Add a job source. The adapter of `kind` must accept `url`, and the source is read once first: a
/// job page that lists no jobs, or a board that does not exist, is refused with the reason (a board
/// address given as a job page is added as that board). A blank `name` becomes the board name or
/// the host. Takes seconds (a page may be rendered in the user's browser).
#[tauri::command]
pub async fn discovery_add_source(
    core: State<'_, Arc<Core>>,
    kind: String,
    name: String,
    url: String,
) -> CommandResult<SourceDto> {
    let core = core.inner().clone();
    run(async move {
        let source = if super::demo::skips_check(&kind, &url) {
            core.add_job_source(&kind, &name, &url, None)?
        } else {
            core.add_job_source_checked(&kind, &name, &url, None).await?
        };
        Ok(SourceDto::from(&source))
    })
    .await
}

/// The job boards a job page points to (one plain read of the page, no browser), for the add
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

/// Bind (or with `None` unbind) the session (a site the user is signed in to) a source is read in.
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

// ------------------------------------------------------------------ Settings > Job sources: suggestions

fn suggestion_dtos(core: &Core) -> anyhow::Result<Vec<SourceSuggestionDto>> {
    Ok(core.source_suggestions()?.iter().map(SourceSuggestionDto::from).collect())
}

/// The suggestions after a search (or its "Continue") ended this way.
fn suggest_run(core: &Core, outcome: SuggestOutcome) -> anyhow::Result<SuggestRunDto> {
    let waiting = matches!(outcome, SuggestOutcome::Waiting { .. });
    Ok(SuggestRunDto { suggestions: suggestion_dtos(core)?, waiting })
}

/// "Suggest sources": search the web in the user's Chrome for job boards and pages that fit the
/// wanted roles, check each one and keep those with jobs as suggestions. Takes minutes; the steps
/// go out as graph progress (`source_suggest`). A search paused at a page the user must deal with
/// returns `waiting`. In demo mode three canned suggestions are stored without any search.
#[tauri::command]
pub async fn discovery_suggest_sources(core: State<'_, Arc<Core>>) -> CommandResult<SuggestRunDto> {
    let core = core.inner().clone();
    run(async move {
        let outcome = match super::demo::suggestions() {
            Some(canned) => {
                let mut saved = 0;
                for suggestion in &canned {
                    saved += usize::from(core.store.upsert_suggestion(suggestion)?.is_some());
                }
                SuggestOutcome::Done { saved }
            }
            None => core.suggest_sources().await?,
        };
        suggest_run(&core, outcome)
    })
    .await
}

/// "Continue" the paused search after the user dealt with the page in Chrome (or retry a failed one).
#[tauri::command]
pub async fn discovery_suggest_resume(core: State<'_, Arc<Core>>) -> CommandResult<SuggestRunDto> {
    let core = core.inner().clone();
    run(async move {
        let outcome = core.suggest_resume().await?;
        suggest_run(&core, outcome)
    })
    .await
}

/// "Stop" the search for sources (the suggestions found so far stay).
#[tauri::command]
pub async fn discovery_suggest_cancel(core: State<'_, Arc<Core>>) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.suggest_cancel()).await
}

/// "Open browser" for the paused search: a focused tab of the user's Chrome on the page the search
/// engine showed. The user deals with it; the app never does.
#[tauri::command]
pub async fn discovery_suggest_open_browser(core: State<'_, Arc<Core>>) -> CommandResult<()> {
    let core = core.inner().clone();
    run(async move { core.suggest_open_browser().await }).await
}

/// "Add" on a suggestion: it becomes a job source (only the address is checked again: the search
/// read it already).
#[tauri::command]
pub async fn discovery_add_suggestion(core: State<'_, Arc<Core>>, id: i64) -> CommandResult<SourceDto> {
    let core = core.inner().clone();
    run_sync(move || Ok(SourceDto::from(&core.add_suggested_source(id)?))).await
}

/// "Add all": every suggestion waiting for the user, one after the other. One that fails does not
/// stop the others; the first failure is reported with the count.
#[tauri::command]
pub async fn discovery_add_all_suggestions(core: State<'_, Arc<Core>>) -> CommandResult<AddAllDto> {
    let core = core.inner().clone();
    run_sync(move || {
        let pending = core.source_suggestions()?;
        let dto = AddAllDto::tally(pending.iter().map(|s| core.add_suggested_source(s.id)));
        tracing::info!(added = dto.added, failed = dto.failed, "source suggestions added");
        Ok(dto)
    })
    .await
}

/// "Dismiss" a suggestion: no later search suggests it again.
#[tauri::command]
pub async fn discovery_dismiss_suggestion(core: State<'_, Arc<Core>>, id: i64) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.dismiss_source_suggestion(id)).await
}

/// This context's commands; `lib.rs` routes every `discovery_*` call here.
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
        discovery_dismissed,
        discovery_restore,
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
        discovery_suggest_sources,
        discovery_suggest_resume,
        discovery_suggest_cancel,
        discovery_suggest_open_browser,
        discovery_add_suggestion,
        discovery_add_all_suggestions,
        discovery_dismiss_suggestion,
    ]
}
