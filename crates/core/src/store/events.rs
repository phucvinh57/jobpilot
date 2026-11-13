use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, Row};

use super::{now, Store};

/// One line of an application's timeline.
#[derive(Clone, Debug)]
pub struct Event {
    pub id: i64,
    pub app_id: i64,
    pub at: DateTime<Utc>,
    /// Free-form short kind: `created`, `status`, `note`, `email`, `error`, ...
    pub kind: String,
    pub message: String,
}

impl Event {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self { id: r.get(0)?, app_id: r.get(1)?, at: r.get(2)?, kind: r.get(3)?, message: r.get(4)? })
    }
}

pub(crate) fn insert_event(conn: &Connection, app_id: i64, kind: &str, message: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO events (app_id, at, kind, message) VALUES (?1, ?2, ?3, ?4)",
        params![app_id, now(), kind, message],
    )?;
    Ok(conn.last_insert_rowid())
}

impl Store {
    pub fn add_event(&self, app_id: i64, kind: &str, message: &str) -> Result<Event> {
        self.with_conn(|c| {
            let id = insert_event(c, app_id, kind, message)?;
            Ok(c.query_row("SELECT id, app_id, at, kind, message FROM events WHERE id = ?1", [id], Event::from_row)?)
        })
    }

    /// Oldest first.
    pub fn list_events(&self, app_id: i64) -> Result<Vec<Event>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(
                "SELECT id, app_id, at, kind, message FROM events WHERE app_id = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([app_id], Event::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}
