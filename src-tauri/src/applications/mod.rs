//! Bounded context `applications`: the application stages of the Jobs page and the Application screen (JD, CV, apply, timeline).
//!
//! `dto` (read models), `rules` (pure rules + tests), `commands` (application services), plus
//! `state` (what is in memory per application: running job, apply session) and `files` (the
//! application folder: `cv.yaml`, previews).

pub mod commands;
pub mod dto;
pub mod files;
pub mod rules;
pub mod state;

use std::sync::Arc;

use jobpilot_engine::Core;
use tauri::{AppHandle, Manager};

/// Called once at startup, before the window opens: the in-memory state of running jobs and open
/// reviews (read and written by the commands only).
pub fn setup(app: &AppHandle, _core: &Arc<Core>) {
    app.manage(state::Applications::default());
}
