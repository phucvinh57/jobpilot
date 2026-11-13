use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::{now, Store};
use crate::prefs::Preferences;

/// The one saved `Preferences` document (`preferences.id = 1`).
impl Store {
    /// The saved preferences, or the default when none were saved yet.
    pub fn preferences(&self) -> Result<Preferences> {
        let json: Option<String> =
            self.with_conn(|c| Ok(c.query_row("SELECT json FROM preferences WHERE id = 1", [], |r| r.get(0)).optional()?))?;
        match json {
            Some(json) => serde_json::from_str(&json).context("the saved preferences are not valid JSON"),
            None => Ok(Preferences::default()),
        }
    }

    /// Whether the user saved preferences at least once.
    pub fn has_preferences(&self) -> Result<bool> {
        self.with_conn(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM preferences)", [], |r| r.get(0))?))
    }

    /// Replace the saved preferences.
    pub fn save_preferences(&self, prefs: &Preferences) -> Result<()> {
        let json = serde_json::to_string(prefs)?;
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO preferences (id, json, updated_at) VALUES (1, ?1, ?2)
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
                rusqlite::params![json, now()],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::Remote;

    #[test]
    fn default_until_saved_then_replaced() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.preferences().unwrap(), Preferences::default());
        assert!(!store.has_preferences().unwrap());

        let prefs = Preferences { roles: vec!["Backend Engineer".into()], remote: Remote::Remote, ..Default::default() };
        store.save_preferences(&prefs).unwrap();
        assert!(store.has_preferences().unwrap());
        assert_eq!(store.preferences().unwrap(), prefs);

        let newer = Preferences { salary_min: Some(90_000), ..prefs };
        store.save_preferences(&newer).unwrap();
        assert_eq!(store.preferences().unwrap().salary_min, Some(90_000));
        let rows: i64 = store.with_conn(|c| Ok(c.query_row("SELECT COUNT(*) FROM preferences", [], |r| r.get(0))?)).unwrap();
        assert_eq!(rows, 1, "one document");
    }

    #[test]
    fn corrupt_json_is_an_error_not_a_default() {
        let store = Store::open_in_memory().unwrap();
        store
            .with_conn(|c| {
                c.execute("INSERT INTO preferences (id, json, updated_at) VALUES (1, 'not json', 'x')", [])?;
                Ok(())
            })
            .unwrap();
        assert!(store.preferences().unwrap_err().to_string().contains("not valid JSON"));
    }
}
