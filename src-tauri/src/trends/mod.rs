//! Bounded context `trends`: the Trends page and its sources. `dto` (read models), `rules` (pure
//! rules + tests), `commands` (application services), `demo` (sample data for `JOBPILOT_DEMO=1`).

pub mod commands;
pub mod demo;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use jobpilot_engine::Core;
use tauri::AppHandle;

/// Called once at startup, before the window opens. This context keeps no state of its own.
pub fn setup(_app: &AppHandle, _core: &Arc<Core>) {}
