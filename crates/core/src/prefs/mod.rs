//! P9 - job-search preferences. [`model`] is the saved document; the chat that fills it and the
//! readers (discovery, scoring, trends) share it through [`Core::preferences`].
//!
//! - [`chat`]: one model turn, the patch merge and the prompt context (no I/O).
//! - [`chat_graph`]: the `prefs_chat` graph and [`PrefsChat`], the calls the UI makes.
//! - [`cv_upload`]: the CV attached to the chat (chat context only).
//! - `undo`: [`Core::update_preferences`] / [`Core::undo_preferences`], one level of undo.
//! - [`demo`]: sample chat and preferences for `JOBPILOT_DEMO=1`.

pub mod chat;
pub mod chat_graph;
pub mod cv_upload;
pub mod demo;
pub mod model;
mod undo;

use anyhow::Result;

pub use chat::{merge_patch, PrefsPatch, Question, TurnOutput};
pub use chat_graph::{ChatState, ChatStatus, ChatView, PrefsChat, THREAD};
pub use cv_upload::UploadedCv;
pub use demo::seed_demo;
pub use model::{Preferences, Remote, Weights};

use crate::Core;

impl Core {
    /// The saved preferences, or the default when none were saved yet.
    pub fn preferences(&self) -> Result<Preferences> {
        self.store.preferences()
    }

    /// Save `prefs` after tidying them ([`Preferences::normalized`]); returns what was stored.
    pub fn save_preferences(&self, prefs: Preferences) -> Result<Preferences> {
        let prefs = prefs.normalized();
        self.store.save_preferences(&prefs)?;
        Ok(prefs)
    }
}
