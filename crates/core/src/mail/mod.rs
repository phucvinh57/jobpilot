//! P6 - Gmail status tracking: OAuth, sync, matching, applying status changes.
//!
//! - [`oauth`]: loopback + PKCE sign-in by hand; [`gmail`]: read-only REST client and message
//!   parsing; [`classify`]: deterministic matching + LLM classification; [`sync`]: the sync,
//!   the poller and the apply/review logic; `account`: `impl Core` (connect, disconnect, state).
//!
//! Public API (UI and other sub-plans):
//! ```text
//! core.gmail_save_client(client_id, Some(secret))?;      // client id -> kv, secret -> keyring
//! let email = core.gmail_connect().await?;                // opens the system browser, <= 5 min
//! let report = mail::sync_once(&core).await?;             // SyncReport { fetched, matched, status_changes, needs_review, .. }
//! mail::start_poller(core.clone(), interval, |report| ..); // or mail::autostart(&core)
//! let mut rx = mail::changes();                           // rx.changed().await: a sync touched the data
//! core.emails_needing_review(Some(app_id))?;              // timeline: one-click accept / dismiss
//! mail::accept_email(&core, &email_id)?; mail::dismiss_email(&core, &email_id)?;
//! core.gmail_disconnect().await?;
//! ```
//! Secrets: refresh token (`gmail:refresh_token`) and client secret (`gmail:client_secret`) only
//! in the keyring; the account address, history id and sync times are plain kv entries.

mod account;
pub mod classify;
pub mod gmail;
mod notify;
pub mod oauth;
pub mod sync;

pub use account::{
    GmailState, KV_ACCOUNT, KV_CLIENT_ID, KV_HISTORY_ID, KV_LAST_REPORT, KV_LAST_SYNC, KV_POLL_MINUTES,
    SECRET_CLIENT_SECRET, SECRET_REFRESH_TOKEN,
};
pub use classify::{Classification, MailKind};
pub use sync::{
    accept_email, autostart, changes, dismiss_email, needs_review, start_poller, sync_once, ReportSummary,
    StatusChange, SyncReport, AUTO_APPLY_CONFIDENCE,
};
