//! Bounded context `integrations`: `dto` (read models), `rules` (pure rules + tests), `commands` (application services).

pub mod commands;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use jobpilot_core::Core;
use tauri::AppHandle;

/// Called once at startup, before the window opens: register this context's own state
/// (`app.manage(..)`) and start its background listeners here.
pub fn setup(_app: &AppHandle, _core: &Arc<Core>) {}
