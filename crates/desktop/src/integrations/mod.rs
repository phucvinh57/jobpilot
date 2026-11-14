//! Bounded context `integrations`: what JobPilot connects to (AI providers, Gmail, browser sessions).
//! `dto` (read models), `rules` (pure rules + tests), `commands` (application services), `download`
//! (the browser download as a service with progress events).

pub mod commands;
pub mod download;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use jobpilot_core::{mail, runtime, Core};
use tauri::{AppHandle, Manager};

use crate::shared::events::emit;

/// A sync that stored mail or changed a status finished (payload `null`): the Gmail card re-reads
/// its "last sync" line.
pub const GMAIL_CHANGED: &str = "integrations:gmail-changed";

/// Called once at startup, before the window opens: the browser download slot, and the listener that
/// turns the Gmail sync's change signal into [`GMAIL_CHANGED`].
pub fn setup(app: &AppHandle, _core: &Arc<Core>) {
    app.manage(download::Downloads::default());

    let mut changes = mail::changes();
    let handle = app.clone();
    drop(runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            emit(&handle, GMAIL_CHANGED, ());
        }
    }));
}
