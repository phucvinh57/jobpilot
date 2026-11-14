//! Changing the saved preferences with one level of undo.
//!
//! Every change (a merged chat patch or a hand edit) goes through [`Core::update_preferences`],
//! which keeps the document it replaced in kv `prefs_prev`; [`Core::undo_preferences`] puts it back.
//! Only the last change can be undone.

use anyhow::{Context, Result};

use super::Preferences;
use crate::engine::Core;

/// kv key of the document before the last change.
const PREV_KEY: &str = "prefs_prev";

impl Core {
    /// Save `prefs` (tidied) as the current preferences and keep the previous ones for
    /// [`undo_preferences`](Core::undo_preferences). Saving what is already saved changes nothing
    /// (and keeps the undo point). Returns what is stored.
    pub fn update_preferences(&self, prefs: Preferences) -> Result<Preferences> {
        let new = prefs.normalized();
        if self.store.has_preferences()? {
            let current = self.store.preferences()?;
            if current == new {
                return Ok(current);
            }
            self.store.kv_set(PREV_KEY, &serde_json::to_string(&current)?)?;
        } else {
            // Nothing was saved yet: undoing the first change goes back to the default document.
            self.store.kv_set(PREV_KEY, &serde_json::to_string(&Preferences::default())?)?;
        }
        self.store.save_preferences(&new)?;
        Ok(new)
    }

    /// Whether there is a change to undo.
    pub fn can_undo_preferences(&self) -> Result<bool> {
        Ok(self.store.kv_get(PREV_KEY)?.is_some())
    }

    /// Put back the preferences from before the last change; `None` when there is nothing to undo.
    pub fn undo_preferences(&self) -> Result<Option<Preferences>> {
        let Some(json) = self.store.kv_get(PREV_KEY)? else { return Ok(None) };
        let prev: Preferences = serde_json::from_str(&json).context("the saved undo point is not valid JSON")?;
        self.store.save_preferences(&prev)?;
        self.store.kv_delete(PREV_KEY)?;
        tracing::info!("preferences: last change undone");
        Ok(Some(prev))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::prefs::Remote;
    use crate::engine::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    #[test]
    fn undo_restores_the_document_before_the_last_change() {
        let (_dir, core) = core();
        assert!(!core.can_undo_preferences().unwrap());
        assert_eq!(core.undo_preferences().unwrap(), None);

        let first = core.update_preferences(Preferences { roles: vec!["SRE".into()], ..Default::default() }).unwrap();
        assert!(core.can_undo_preferences().unwrap());
        let second = core.update_preferences(Preferences { remote: Remote::Hybrid, ..first.clone() }).unwrap();
        assert_eq!(core.preferences().unwrap(), second);

        assert_eq!(core.undo_preferences().unwrap(), Some(first.clone()), "back to the document before the last change");
        assert_eq!(core.preferences().unwrap(), first);
        assert!(!core.can_undo_preferences().unwrap(), "one level only");
        assert_eq!(core.undo_preferences().unwrap(), None);
    }

    #[test]
    fn saving_the_same_document_keeps_the_undo_point_and_the_first_change_undoes_to_default() {
        let (_dir, core) = core();
        let a = core.update_preferences(Preferences { roles: vec!["SRE".into()], ..Default::default() }).unwrap();
        let b = core.update_preferences(Preferences { roles: vec!["SRE".into(), "Backend".into()], ..Default::default() }).unwrap();
        core.update_preferences(b.clone()).unwrap();
        assert_eq!(core.undo_preferences().unwrap(), Some(a), "the repeated save did not replace the undo point");

        let (_dir2, fresh) = self::core();
        fresh.update_preferences(Preferences { roles: vec!["SRE".into()], ..Default::default() }).unwrap();
        assert_eq!(fresh.undo_preferences().unwrap(), Some(Preferences::default()));
        assert!(fresh.store.has_preferences().unwrap());
    }

    #[test]
    fn updates_are_tidied() {
        let (_dir, core) = core();
        let saved = core.update_preferences(Preferences { roles: vec![" SRE ".into(), "sre".into()], ..Default::default() }).unwrap();
        assert_eq!(saved.roles, ["SRE"]);
        assert_eq!(core.preferences().unwrap(), saved);
    }
}
