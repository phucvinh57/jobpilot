use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::{now, Store};
use crate::prefs::Preferences;

/// The one saved `Preferences` document (`preferences.id = 1`).
impl Store {
    /// The saved preferences, or the default when none were saved yet.
    pub fn preferences(&self) -> Result<Preferences> {
        self.with_conn(read)
    }

    /// Whether the user saved preferences at least once.
    pub fn has_preferences(&self) -> Result<bool> {
        self.with_conn(|c| Ok(c.query_row("SELECT EXISTS (SELECT 1 FROM preferences)", [], |r| r.get(0))?))
    }

    /// Replace the saved preferences.
    pub fn save_preferences(&self, prefs: &Preferences) -> Result<()> {
        self.with_conn(|c| write(c, prefs))
    }

    /// Change the saved preferences in place: read, `change`, tidy ([`Preferences::normalized`]),
    /// write, in one transaction, so a concurrent writer's edit is never lost. Returns what was stored.
    pub fn update_preferences(&self, change: impl FnOnce(&mut Preferences)) -> Result<Preferences> {
        self.with_tx(|tx| {
            let mut prefs = read(tx)?;
            change(&mut prefs);
            let prefs = prefs.normalized();
            write(tx, &prefs)?;
            Ok(prefs)
        })
    }
}

fn read(c: &rusqlite::Connection) -> Result<Preferences> {
    let json: Option<String> = c.query_row("SELECT json FROM preferences WHERE id = 1", [], |r| r.get(0)).optional()?;
    match json {
        Some(json) => serde_json::from_str(&json).context("the saved preferences are not valid JSON"),
        None => Ok(Preferences::default()),
    }
}

fn write(c: &rusqlite::Connection, prefs: &Preferences) -> Result<()> {
    let json = serde_json::to_string(prefs)?;
    c.execute(
        "INSERT INTO preferences (id, json, updated_at) VALUES (1, ?1, ?2)
         ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
        rusqlite::params![json, now()],
    )?;
    Ok(())
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
    fn an_update_is_one_transaction_so_concurrent_edits_are_not_lost() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        store.save_preferences(&Preferences { roles: vec!["SRE".into()], ..Default::default() }).unwrap();
        std::thread::scope(|scope| {
            for t in 0..8 {
                let store = &store;
                scope.spawn(move || {
                    for n in 0..10 {
                        store.update_preferences(|p| p.exclude_companies.push(format!("Company {t}-{n}"))).unwrap();
                    }
                });
            }
        });
        let prefs = store.preferences().unwrap();
        assert_eq!(prefs.exclude_companies.len(), 80, "every writer's company is there");
        assert_eq!(prefs.roles, ["SRE"], "other fields are untouched");

        // The result is tidied like any saved document, and returned.
        let stored = store.update_preferences(|p| p.exclude_companies.push(" company 0-0 ".into())).unwrap();
        assert_eq!(stored.exclude_companies.len(), 80, "duplicates (ignoring case) are removed");
        assert_eq!(stored, store.preferences().unwrap());
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
        assert!(store.update_preferences(|_| {}).is_err(), "never overwrites what it could not read");
    }
}
