//! The optional daily search (setting in Settings > Job sources): every few minutes ask the core
//! whether one is due (setting on, a source to search, nothing running, a day since the last run)
//! and, if so, run it like "Search now". It lives here, not in a screen, so it runs for as long as
//! the app does, whichever screen is shown.

use std::sync::Arc;

use jobpilot_core::chrono::Utc;
use jobpilot_core::{runtime, Core};
use tauri::AppHandle;

use super::commands::SEARCH_FINISHED;
use super::dto::SearchFinishedDto;
use super::rules::{DAILY_CHECK_EVERY, DAILY_FIRST_CHECK};
use crate::shared::events::{self, Scope};

/// Start the timer. It never ends before the app does.
pub fn start(app: AppHandle, core: Arc<Core>) {
    drop(runtime::spawn(async move {
        let mut wait = DAILY_FIRST_CHECK;
        loop {
            tokio::time::sleep(wait).await;
            wait = DAILY_CHECK_EVERY;
            check(&app, &core).await;
        }
    }));
}

/// One look: run the search when it is due, then tell the screens.
async fn check(app: &AppHandle, core: &Core) {
    match core.discover_due(Utc::now()) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::warn!("checking whether the daily search is due failed: {e:#}");
            return;
        }
    }
    // Recorded before the run, so a search that fails at once (no provider, no network) is not
    // started again every few minutes.
    if let Err(e) = core.discover_daily_started(Utc::now()) {
        tracing::warn!("recording the daily search failed: {e:#}");
    }
    let result = core.discover_now().await;
    match &result {
        Ok(summary) => tracing::info!(
            saved = summary.saved,
            waiting = summary.waiting,
            failed = summary.failed,
            "daily search done"
        ),
        Err(e) => tracing::warn!("the daily search failed: {e:#}"),
    }
    events::emit(app, SEARCH_FINISHED, SearchFinishedDto::new(result));
    events::data_changed(app, Scope::Leads);
    events::data_changed(app, Scope::Sources);
}
