//! Bounded context `profile`: Profile > About you (the documents the profile is read from) and the
//! job-search preferences. `dto` (read models), `rules` (pure rules + tests), `commands` (application
//! services).

pub mod commands;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use jobpilot_engine::Core;
use tauri::AppHandle;

/// Called once at startup, before the window opens. This context keeps no state of its own.
pub fn setup(_app: &AppHandle, _core: &Arc<Core>) {}
