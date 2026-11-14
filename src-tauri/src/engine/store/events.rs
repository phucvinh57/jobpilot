use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{now, Store};
use crate::engine::redact::{redact, PATTERNS_VERSION};

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
        // Masked on the way out too: rows written before events were redacted on the way in.
        let message: String = r.get(4)?;
        Ok(Self { id: r.get(0)?, app_id: r.get(1)?, at: r.get(2)?, kind: r.get(3)?, message: redact(&message).into_owned() })
    }
}

/// Every event is stored with its secrets masked ([`redact`]): an error text may carry a URL with a
/// token in its query.
pub(crate) fn insert_event(conn: &Connection, app_id: i64, kind: &str, message: &str) -> Result<i64> {
    conn.execute(
        "INSERT INTO events (app_id, at, kind, message) VALUES (?1, ?2, ?3, ?4)",
        params![app_id, now(), kind, redact(message).as_ref()],
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

/// The kv key holding the [`PATTERNS_VERSION`] the stored events were last scrubbed with.
const EVENTS_REDACTED_KEY: &str = "store.events_redacted";

/// Mask the secrets of events stored before they were redacted on insert, or with older patterns:
/// once per database and pattern version. Returns how many rows changed.
pub(crate) fn scrub_old_events(conn: &mut Connection) -> Result<usize> {
    let done: Option<String> = conn
        .query_row("SELECT value FROM kv WHERE key = ?1", [EVENTS_REDACTED_KEY], |r| r.get(0))
        .optional()?;
    if done.as_deref() == Some(PATTERNS_VERSION.to_string().as_str()) {
        return Ok(0);
    }
    let tx = conn.transaction()?;
    let changed = {
        let mut stmt = tx.prepare("SELECT id, message FROM events")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
        let mut changed = Vec::new();
        for row in rows {
            let (id, message) = row?;
            let masked = redact(&message);
            if masked != message {
                changed.push((id, masked.into_owned()));
            }
        }
        changed
    };
    for (id, masked) in &changed {
        tx.execute("UPDATE events SET message = ?1 WHERE id = ?2", params![masked, id])?;
    }
    tx.execute("INSERT OR REPLACE INTO kv (key, value) VALUES (?1, ?2)", params![EVENTS_REDACTED_KEY, PATTERNS_VERSION.to_string()])?;
    tx.commit()?;
    Ok(changed.len())
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

    const TOKEN_URL: &str = "fetching https://a.test/jobs/1?afd_azwaf_tok=SECRETVALUE&x=1 failed";

    fn raw_messages(store: &Store) -> Vec<String> {
        store
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT message FROM events ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .unwrap()
    }

    #[test]
    fn an_event_is_stored_and_read_with_its_url_token_masked() {
        let (_dir, store) = store();
        let a = app(&store, "alpha");
        let event = store.add_event(a, "error", TOKEN_URL).unwrap();
        assert!(!event.message.contains("SECRETVALUE"), "{}", event.message);
        assert!(event.message.contains("afd_azwaf_tok=[REDACTED]&x=1"), "{}", event.message);
        assert!(raw_messages(&store).iter().all(|m| !m.contains("SECRETVALUE")), "nothing secret at rest");
    }

    #[test]
    fn an_old_row_is_masked_when_read_and_scrubbed_once_at_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let store = Store::open(&path).unwrap();
        let a = app(&store, "alpha");
        // As an older build wrote it: raw, and the scrub not yet run on this database.
        store
            .with_conn(|c| {
                c.execute("INSERT INTO events (app_id, at, kind, message) VALUES (?1, ?2, 'error', ?3)", rusqlite::params![a, super::now(), TOKEN_URL])?;
                c.execute("DELETE FROM kv WHERE key = ?1", [super::EVENTS_REDACTED_KEY])?;
                Ok(())
            })
            .unwrap();
        assert!(raw_messages(&store).iter().any(|m| m.contains("SECRETVALUE")));
        assert!(store.list_events(a).unwrap().iter().all(|e| !e.message.contains("SECRETVALUE")), "masked when read");
        drop(store);

        let store = Store::open(&path).unwrap();
        assert!(raw_messages(&store).iter().all(|m| !m.contains("SECRETVALUE")), "scrubbed at open");
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(super::scrub_old_events(&mut conn).unwrap(), 0, "runs once per database");
        conn.execute("UPDATE kv SET value = '1' WHERE key = ?1", [super::EVENTS_REDACTED_KEY]).unwrap();
        assert_eq!(super::scrub_old_events(&mut conn).unwrap(), 0, "a newer pattern version re-scrubs; nothing is left to mask");
        let version: String =
            conn.query_row("SELECT value FROM kv WHERE key = ?1", [super::EVENTS_REDACTED_KEY], |r| r.get(0)).unwrap();
        assert_eq!(version, crate::engine::redact::PATTERNS_VERSION.to_string());
    }

}
