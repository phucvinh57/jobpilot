//! Bounded context `discovery`: leads (the Leads stage of the Jobs page), job sources and their
//! daily search. `dto` (read models), `rules` (pure rules + tests), `commands` (application
//! services), `daily` (the daily search timer), `demo` (sample answers for `JOBPILOT_DEMO=1`).

pub mod commands;
mod daily;
mod demo;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use crate::engine::Core;
use tauri::AppHandle;

/// Called once at startup, before the window opens: start the daily search timer.
pub fn setup(app: &AppHandle, core: &Arc<Core>) {
    daily::start(app.clone(), core.clone());
}
