//! Bounded context `integrations`: what JobPilot connects to (AI providers, Gmail, the user's browser
//! and the sites they are signed in to). `dto` (read models), `rules` (pure rules + tests),
//! `commands` (application services).

pub mod commands;
pub mod dto;
pub mod rules;

use std::sync::Arc;

use jobpilot_engine::browser::BridgeEvent;
use jobpilot_engine::{mail, runtime, Core};
use tauri::AppHandle;
use tokio::sync::broadcast::error::RecvError;

use crate::shared::events::{data_changed, emit, Scope};

/// A sync that stored mail or changed a status finished (payload `null`): the Gmail card re-reads
/// its "last sync" line.
pub const GMAIL_CHANGED: &str = "integrations:gmail-changed";
/// The browser connection changed (connected, a pairing code to approve, paired, forgotten): payload
/// `null`; the Browser section re-reads `integrations_browser`. A `shared:data-changed` with scope
/// `integrations` goes out with it, so the shell's health line follows too.
pub const BROWSER_STATUS: &str = "integrations:browser-status";

/// Called once at startup, before the window opens: the listeners that turn the Gmail sync's change
/// signal into [`GMAIL_CHANGED`] and the browser bridge's status changes into [`BROWSER_STATUS`].
pub fn setup(app: &AppHandle, core: &Arc<Core>) {
    let mut changes = mail::changes();
    let handle = app.clone();
    drop(runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            emit(&handle, GMAIL_CHANGED, ());
        }
    }));

    let mut events = core.browser().subscribe();
    let handle = app.clone();
    drop(runtime::spawn(async move {
        loop {
            match events.recv().await {
                Ok(BridgeEvent::StatusChanged) | Err(RecvError::Lagged(_)) => {
                    emit(&handle, BROWSER_STATUS, ());
                    data_changed(&handle, Scope::Integrations);
                }
                Ok(_) => {}
                Err(RecvError::Closed) => break,
            }
        }
    }));
}
