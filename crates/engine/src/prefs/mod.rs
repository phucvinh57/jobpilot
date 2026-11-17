//! Job-search preferences. [`model`] is the saved document; the chat that fills it and the
//! readers (discovery, scoring, trends) share it through [`Core::preferences`].
//!
//! - [`chat`]: one model turn, the patch merge and the prompt context (no I/O).
//! - [`chat_graph`]: the `prefs_chat` graph and [`PrefsChat`], the calls the UI makes.
//! - `undo`: [`Core::update_preferences`] / [`Core::undo_preferences`], one level of undo.
//! - [`edit`]: [`Core::edit_preferences`], [`Core::read_preference_notes`] and
//!   [`Core::undo_preference_edit`], the app's changes to the document one at a time (one lock).
//! - [`demo`]: sample chat and preferences for `JOBPILOT_DEMO=1`.
//! - [`suggest`]: skills, locations and domains the preferences screen can offer ([`PrefSuggestions`]).
//! - [`appearance`]: the saved colour scheme ([`Appearance`]).

pub mod appearance;
pub mod chat;
pub mod chat_graph;
pub mod demo;
pub mod edit;
pub mod model;
pub mod suggest;
mod undo;

use anyhow::Result;

pub use appearance::Appearance;
pub use chat::{merge_patch, PrefsPatch, Question, TurnOutput};
pub use chat_graph::{ChatState, ChatStatus, ChatView, PrefsChat, THREAD};
pub use demo::seed_demo;
pub use edit::{clean_notes, NotesRead};
pub use model::{Preferences, Remote, Weights};
pub use suggest::PrefSuggestions;

use crate::Core;

impl Core {
    /// The saved preferences, or the default when none were saved yet.
    pub fn preferences(&self) -> Result<Preferences> {
        self.store.preferences()
    }

    /// Save `prefs` after tidying them ([`Preferences::normalized`]); returns what was stored.
    /// The waiting leads are filtered again with them ([`Core::refilter_leads`]) when a field the
    /// filters read changed.
    pub fn save_preferences(&self, prefs: Preferences) -> Result<Preferences> {
        let prefs = prefs.normalized();
        let old = self.store.preferences()?;
        self.store.save_preferences(&prefs)?;
        self.refilter_after_save(&old, &prefs);
        Ok(prefs)
    }
}
