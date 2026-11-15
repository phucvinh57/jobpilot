//! Application services of this context: thin `#[tauri::command]`s named `trends_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! A run (collect, regenerate, resume) returns when it is done; its steps go out as
//! `shared:graph-progress` events (graphs `trends` and `trends_digest`).

use std::sync::Arc;

use crate::engine::trends::{Period, SourceKind, TrendConfig};
use crate::engine::Core;
use tauri::ipc::Invoke;
use tauri::State;

use super::dto::{self, AddedDto, OverviewDto, RunDto};
use super::rules::{Cadence, Field, Presence};
use crate::shared::{run, run_sync, CommandError, CommandResult};

type Cx<'a> = State<'a, Arc<Core>>;

fn period_of(label: &str) -> CommandResult<Period> {
    Period::parse(label).ok_or_else(|| CommandError::new(format!("unknown period: {label}")))
}

fn kind_of(kind: &str) -> CommandResult<SourceKind> {
    SourceKind::parse(kind).ok_or_else(|| CommandError::new(format!("unknown source kind: {kind}")))
}

// ------------------------------------------------------------------ reads

/// Everything the screen shows: the digest `selected` (the newest when `None`), its chart, the
/// history, the waiting collections, the sources and the saved search keys. `cadence`: `weekly` /
/// `quarterly`, the period a new run would cover.
#[tauri::command]
pub async fn trends_overview(
    core: Cx<'_>,
    selected: Option<i64>,
    cadence: String,
) -> CommandResult<OverviewDto> {
    let cadence = Cadence::parse(&cadence)
        .ok_or_else(|| CommandError::new(format!("unknown cadence: {cadence}")))?;
    let core = core.inner().clone();
    run_sync(move || dto::overview(&core, selected, cadence)).await
}

// ------------------------------------------------------------------ runs

/// "Collect & build digest": collect the period's items, then write the digest. A source at a login
/// or captcha pauses the run (`status: "waiting"`).
#[tauri::command]
pub async fn trends_collect(core: Cx<'_>, period: String) -> CommandResult<RunDto> {
    let period = period_of(&period)?;
    let core = core.inner().clone();
    tracing::info!("trends: collecting {}", period.label());
    dto::run_result(run(async move { core.collect_trends_and_digest(period).await }).await?)
}

/// "Refresh": the digest alone, from the stored items.
#[tauri::command]
pub async fn trends_regenerate(core: Cx<'_>, period: String) -> CommandResult<RunDto> {
    let period = period_of(&period)?;
    let core = core.inner().clone();
    tracing::info!("trends: regenerating {}", period.label());
    dto::run_result(run(async move { core.build_trends_digest(period).await }).await?)
}

/// "Continue" (after the user dealt with the page) and "Retry" of a collection: carry on where it stopped.
#[tauri::command]
pub async fn trends_resume(core: Cx<'_>, period: String) -> CommandResult<RunDto> {
    let period = period_of(&period)?;
    let core = core.inner().clone();
    tracing::info!("trends: resuming the collection of {}", period.label());
    dto::run_result(
        run(async move {
            core.resume_trends_collection(period, TrendConfig::default())
                .await
        })
        .await?,
    )
}

/// "Retry" of a digest-only run: carry on where it stopped.
#[tauri::command]
pub async fn trends_resume_digest(core: Cx<'_>, period: String) -> CommandResult<RunDto> {
    let period = period_of(&period)?;
    let core = core.inner().clone();
    tracing::info!("trends: resuming the digest of {}", period.label());
    dto::run_result(run(async move { core.resume_trends_digest(period).await }).await?)
}

// ------------------------------------------------------------------ sources

/// Add a source. Hacker News and YC need nothing more; a feed or page needs `url`.
#[tauri::command]
pub async fn trends_add_source(
    core: Cx<'_>,
    kind: String,
    name: String,
    url: String,
) -> CommandResult<()> {
    let kind = kind_of(&kind)?;
    let core = core.inner().clone();
    run_sync(move || {
        let (name, url) = if kind.needs_url() {
            (name.trim().to_string(), url.trim().to_string())
        } else {
            (String::new(), String::new())
        };
        let source = core.add_trend_source(kind, &name, &url)?;
        tracing::info!("trends: added source {} ({})", source.id, kind.as_str());
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn trends_remove_source(core: Cx<'_>, id: i64) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.remove_trend_source(id)).await
}

/// Turn a source on or off for the next run.
#[tauri::command]
pub async fn trends_set_source_enabled(core: Cx<'_>, id: i64, enabled: bool) -> CommandResult<()> {
    let core = core.inner().clone();
    run_sync(move || core.set_trend_source_enabled(id, enabled)).await
}

// ------------------------------------------------------------------ search keys (write-only)

/// Save the API key of a search provider (`brave` / `tavily`) in the OS keyring. The key is never
/// read back, logged or returned.
#[tauri::command]
pub async fn trends_save_key(core: Cx<'_>, provider: String, key: String) -> CommandResult<()> {
    let provider = dto::parse_provider(&provider)
        .ok_or_else(|| CommandError::new(format!("unknown search provider: {provider}")))?;
    let core = core.inner().clone();
    run_sync(move || core.set_search_key(provider, &key)).await
}

#[tauri::command]
pub async fn trends_clear_key(core: Cx<'_>, provider: String) -> CommandResult<()> {
    let provider = dto::parse_provider(&provider)
        .ok_or_else(|| CommandError::new(format!("unknown search provider: {provider}")))?;
    let core = core.inner().clone();
    run_sync(move || core.clear_search_key(provider)).await
}

// ------------------------------------------------------------------ preferences

/// "Add to preferences": put a skill or domain of the digest into the saved preferences at once.
/// Nothing is cancelled and no digest changes. Says where the topic stands afterwards, and why it
/// was not added when it was not.
#[tauri::command]
pub async fn trends_add_to_preferences(
    core: Cx<'_>,
    field: String,
    topic: String,
) -> CommandResult<AddedDto> {
    let field = Field::parse(&field)
        .ok_or_else(|| CommandError::new(format!("unknown preference list: {field}")))?;
    let core = core.inner().clone();
    run(async move {
        let wanted = topic.clone();
        // `add` says whether it changed anything: a no-op saves nothing and keeps the undo point.
        let prefs = core
            .edit_preferences(move |prefs| field.add(prefs, &wanted))
            .await?;
        let presence = field.presence(&prefs, &topic);
        let message = (presence != Presence::Added).then(|| field.why_not_added(&prefs, &topic));
        Ok(AddedDto {
            presence: presence.as_str(),
            message,
        })
    })
    .await
}

/// This context's commands; `lib.rs` routes every `trends_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        trends_overview,
        trends_collect,
        trends_regenerate,
        trends_resume,
        trends_resume_digest,
        trends_add_source,
        trends_remove_source,
        trends_set_source_enabled,
        trends_save_key,
        trends_clear_key,
        trends_add_to_preferences,
    ]
}

#[cfg(test)]
mod tests {
    use super::{kind_of, period_of};

    #[test]
    fn arguments_from_the_web_app_are_checked() {
        assert_eq!(
            period_of("2026-W46").map(|p| p.label()).ok().as_deref(),
            Some("2026-W46")
        );
        assert_eq!(
            period_of("2026-Q4").map(|p| p.label()).ok().as_deref(),
            Some("2026-Q4")
        );
        assert!(period_of("2026-W99").is_err());
        assert!(period_of("").is_err());
        assert_eq!(kind_of("rss").map(|k| k.as_str()).ok(), Some("rss"));
        assert!(kind_of("greenhouse").is_err());
    }
}
