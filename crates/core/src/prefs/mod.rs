//! P9 - job-search preferences. [`model`] is the saved document; the chat that fills it (P9) and
//! the readers (discovery, scoring, trends) share it through [`Core::preferences`].

pub mod model;

use anyhow::Result;

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
