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

    /// The newest `limit` events across all applications, newest first (ids grow with time).
    pub fn recent_events(&self, limit: usize) -> Result<Vec<Event>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        self.with_conn(|c| {
            let mut stmt =
                c.prepare("SELECT id, app_id, at, kind, message FROM events ORDER BY id DESC LIMIT ?1")?;
            let rows = stmt.query_map([limit], Event::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::store::{NewApplication, Store};

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, store)
    }

    fn app(store: &Store, slug: &str) -> i64 {
        let new = NewApplication {
            slug: slug.into(),
            company: slug.into(),
            role: "Engineer".into(),
            dir: format!("/tmp/{slug}").into(),
            ..Default::default()
        };
        store.create_application(&new).unwrap().id
    }

    #[test]
    fn recent_events_span_all_applications_newest_first() {
        let (_dir, store) = store();
        assert!(store.recent_events(5).unwrap().is_empty());

        let (a, b) = (app(&store, "alpha"), app(&store, "beta"));
        store.add_event(a, "note", "a1").unwrap();
        store.add_event(b, "email", "b1").unwrap();
        store.add_event(a, "note", "a2").unwrap();

        let messages = |limit| {
            store.recent_events(limit).unwrap().into_iter().map(|e| (e.app_id, e.message)).collect::<Vec<_>>()
        };
        assert_eq!(messages(2), [(a, "a2".to_string()), (b, "b1".to_string())], "the newest first, whichever application");
        assert_eq!(messages(100).len(), 5, "two `created` events and three notes");
        assert_eq!(messages(100).last().map(|(app_id, _)| *app_id), Some(a), "the first `created` is last");
        assert!(messages(0).is_empty());
    }
}
