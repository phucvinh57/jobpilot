//! Bounded context `discovery`: `dto` (read models), `rules` (pure rules + tests), `commands` (application services),
//! `demo` (sample answers for `JOBPILOT_DEMO=1`).
//! Leads, the queue shown on Today, job Sources and their daily search.

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
