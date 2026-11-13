use anyhow::Result;
use rusqlite::{params, OptionalExtension};

use super::Store;

/// Key/value settings (e.g. `profile_path`). Never store secrets here.
impl Store {
    pub fn kv_get(&self, key: &str) -> Result<Option<String>> {
        self.with_conn(|c| {
            Ok(c.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
        })
    }

    pub fn kv_set(&self, key: &str, value: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO kv (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?;
            Ok(())
        })
    }

    pub fn kv_delete(&self, key: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM kv WHERE key = ?1", [key])?;
            Ok(())
        })
    }
}
