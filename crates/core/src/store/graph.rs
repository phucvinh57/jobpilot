//! `graph_checkpoints`: the saved progress of graph runs (see `crate::graph`).
//!
//! Plain rows only; the graph module converts them to and from the library's checkpoint type.
//! A thread has several rows (the last few steps); the newest row carries the thread's status.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// Thread statuses stored in the `status` column.
pub const GRAPH_RUNNING: &str = "running";
pub const GRAPH_WAITING: &str = "waiting";
pub const GRAPH_FAILED: &str = "failed";
pub const GRAPH_DONE: &str = "done";

/// One saved step of a graph thread.
#[derive(Clone, Debug)]
pub struct GraphCheckpointRow {
    /// Unique checkpoint id.
    pub id: String,
    pub thread: String,
    /// Name of the graph the thread runs.
    pub graph: String,
    pub step: i64,
    /// The typed state, as a JSON object.
    pub state_json: String,
    /// Everything else a resume needs (pending nodes, retry budget, ...), opaque to the store.
    pub rest_json: String,
    pub created_at: DateTime<Utc>,
}

/// A thread as seen through its newest checkpoint.
#[derive(Clone, Debug)]
pub struct GraphThread {
    pub thread: String,
    pub graph: String,
    pub step: i64,
    /// `running`, `waiting`, `failed` or `done`.
    pub status: String,
    /// JSON about the status: the wait reason, or the failure.
    pub detail_json: Option<String>,
    pub updated_at: DateTime<Utc>,
}

const ROW_COLS: &str = "id, thread, graph, step, state_json, rest_json, created_at";
const THREAD_COLS: &str = "thread, graph, step, status, detail_json, updated_at";

impl GraphCheckpointRow {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            thread: r.get(1)?,
            graph: r.get(2)?,
            step: r.get(3)?,
            state_json: r.get(4)?,
            rest_json: r.get(5)?,
            created_at: r.get(6)?,
        })
    }
}

impl GraphThread {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            thread: r.get(0)?,
            graph: r.get(1)?,
            step: r.get(2)?,
            status: r.get(3)?,
            detail_json: r.get(4)?,
            updated_at: r.get(5)?,
        })
    }
}

impl Store {
    /// Append a checkpoint and keep only the `keep` newest of its thread, in one transaction; the
    /// thread's status becomes `running`.
    ///
    /// `live` is asked inside the transaction: when it says no (the thread was cancelled), nothing
    /// is written and the answer is `false`. Because [`graph_delete_thread`](Self::graph_delete_thread)
    /// takes the same lock, a cancelled thread whose token is set before the delete cannot get a
    /// row written afterwards.
    pub fn graph_save_checkpoint(
        &self,
        row: &GraphCheckpointRow,
        keep: usize,
        live: impl FnOnce() -> bool,
    ) -> Result<bool> {
        self.with_tx(|tx| {
            if !live() {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO graph_checkpoints (id, thread, graph, step, state_json, rest_json, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    row.id,
                    row.thread,
                    row.graph,
                    row.step,
                    row.state_json,
                    row.rest_json,
                    GRAPH_RUNNING,
                    row.created_at,
                    now()
                ],
            )?;
            tx.execute(
                "DELETE FROM graph_checkpoints WHERE thread = ?1 AND seq NOT IN
                 (SELECT seq FROM graph_checkpoints WHERE thread = ?1 ORDER BY seq DESC LIMIT ?2)",
                params![row.thread, i64::try_from(keep).unwrap_or(i64::MAX)],
            )?;
            Ok(true)
        })
    }

    /// The newest checkpoint of a thread.
    pub fn graph_latest_checkpoint(&self, thread: &str) -> Result<Option<GraphCheckpointRow>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {ROW_COLS} FROM graph_checkpoints WHERE thread = ?1 ORDER BY seq DESC LIMIT 1"),
                [thread],
                GraphCheckpointRow::from_row,
            )
            .optional()?)
        })
    }

    pub fn graph_checkpoint_by_id(&self, id: &str) -> Result<Option<GraphCheckpointRow>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {ROW_COLS} FROM graph_checkpoints WHERE id = ?1"),
                [id],
                GraphCheckpointRow::from_row,
            )
            .optional()?)
        })
    }

    /// All kept checkpoints of a thread, oldest first.
    pub fn graph_checkpoints(&self, thread: &str) -> Result<Vec<GraphCheckpointRow>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare(&format!("SELECT {ROW_COLS} FROM graph_checkpoints WHERE thread = ?1 ORDER BY seq"))?;
            let rows = stmt.query_map([thread], GraphCheckpointRow::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Forget a thread completely.
    pub fn graph_delete_thread(&self, thread: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM graph_checkpoints WHERE thread = ?1", [thread])?;
            Ok(())
        })
    }

    /// Set the status (and its detail) of a thread: written on its newest checkpoint, if `live`
    /// (asked inside the lock, see [`graph_save_checkpoint`](Self::graph_save_checkpoint)) says so.
    /// Returns whether it was written.
    pub fn graph_set_status(
        &self,
        thread: &str,
        status: &str,
        detail_json: Option<&str>,
        live: impl FnOnce() -> bool,
    ) -> Result<bool> {
        self.with_conn(|c| {
            if !live() {
                return Ok(false);
            }
            c.execute(
                "UPDATE graph_checkpoints SET status = ?2, detail_json = ?3, updated_at = ?4
                 WHERE seq = (SELECT MAX(seq) FROM graph_checkpoints WHERE thread = ?1)",
                params![thread, status, detail_json, now()],
            )?;
            Ok(true)
        })
    }

    pub fn graph_thread(&self, thread: &str) -> Result<Option<GraphThread>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {THREAD_COLS} FROM graph_checkpoints WHERE thread = ?1 ORDER BY seq DESC LIMIT 1"),
                [thread],
                GraphThread::from_row,
            )
            .optional()?)
        })
    }

    /// Threads whose newest checkpoint has `status`, oldest first.
    pub fn graph_threads_with_status(&self, status: &str) -> Result<Vec<GraphThread>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {THREAD_COLS} FROM graph_checkpoints c
                 WHERE status = ?1 AND seq = (SELECT MAX(seq) FROM graph_checkpoints WHERE thread = c.thread)
                 ORDER BY updated_at, seq"
            ))?;
            let rows = stmt.query_map([status], GraphThread::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Delete finished (`done` or `failed`) threads, and `running` ones orphaned by a crash, last
    /// touched before `before`; returns the number of threads removed. Waiting threads never expire.
    pub fn graph_prune(&self, before: DateTime<Utc>) -> Result<usize> {
        self.with_tx(|tx| {
            let threads: Vec<String> = {
                let mut stmt = tx.prepare(
                    "SELECT thread FROM graph_checkpoints c
                     WHERE status IN (?1, ?2, ?4) AND updated_at < ?3
                       AND seq = (SELECT MAX(seq) FROM graph_checkpoints WHERE thread = c.thread)",
                )?;
                let rows = stmt.query_map(params![GRAPH_DONE, GRAPH_FAILED, before, GRAPH_RUNNING], |r| r.get(0))?;
                rows.collect::<rusqlite::Result<_>>()?
            };
            for t in &threads {
                tx.execute("DELETE FROM graph_checkpoints WHERE thread = ?1", [t])?;
            }
            Ok(threads.len())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(thread: &str, step: i64) -> GraphCheckpointRow {
        GraphCheckpointRow {
            id: format!("{thread}-{step}"),
            thread: thread.into(),
            graph: "g".into(),
            step,
            state_json: format!(r#"{{"step":{step}}}"#),
            rest_json: "{}".into(),
            created_at: now(),
        }
    }

    fn save(store: &Store, row: &GraphCheckpointRow) {
        assert!(store.graph_save_checkpoint(row, 100, || true).unwrap());
    }

    #[test]
    fn migration_applies_on_a_database_at_the_previous_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(super::super::MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute("INSERT INTO kv (key, value) VALUES ('keep', 'me')", []).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.kv_get("keep").unwrap().as_deref(), Some("me"));
        save(&store, &row("t", 0));
        assert!(store.graph_thread("t").unwrap().is_some());
        let version: usize =
            store.with_conn(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?)).unwrap();
        assert_eq!(version, super::super::MIGRATIONS.len());
    }

    #[test]
    fn checkpoints_threads_and_status_round_trip() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.graph_latest_checkpoint("t").unwrap().is_none());
        for step in 0..4 {
            save(&store, &row("t", step));
        }
        save(&store, &row("other", 0));

        assert_eq!(store.graph_latest_checkpoint("t").unwrap().unwrap().step, 3);
        assert_eq!(store.graph_checkpoint_by_id("t-1").unwrap().unwrap().state_json, r#"{"step":1}"#);
        assert_eq!(store.graph_checkpoints("t").unwrap().len(), 4);

        let thread = store.graph_thread("t").unwrap().unwrap();
        assert_eq!((thread.graph.as_str(), thread.step, thread.status.as_str()), ("g", 3, GRAPH_RUNNING));

        assert!(store.graph_set_status("t", GRAPH_WAITING, Some(r#"{"kind":"user_message"}"#), || true).unwrap());
        let waiting = store.graph_threads_with_status(GRAPH_WAITING).unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!(waiting[0].detail_json.as_deref(), Some(r#"{"kind":"user_message"}"#));
        assert_eq!(store.graph_thread("other").unwrap().unwrap().status, GRAPH_RUNNING, "other thread untouched");

        // A new checkpoint makes the thread `running` again: only the newest row counts.
        assert!(store.graph_save_checkpoint(&row("t", 4), 2, || true).unwrap(), "saved, and trimmed to two");
        assert!(store.graph_threads_with_status(GRAPH_WAITING).unwrap().is_empty());
        let kept: Vec<_> = store.graph_checkpoints("t").unwrap().iter().map(|r| r.step).collect();
        assert_eq!(kept, [3, 4]);

        store.graph_delete_thread("t").unwrap();
        assert!(store.graph_thread("t").unwrap().is_none());
        assert!(store.graph_thread("other").unwrap().is_some());
    }

    #[test]
    fn prune_removes_finished_threads_only() {
        let store = Store::open_in_memory().unwrap();
        for (thread, status) in [("done", GRAPH_DONE), ("failed", GRAPH_FAILED), ("waiting", GRAPH_WAITING)] {
            save(&store, &row(thread, 0));
            save(&store, &row(thread, 1));
            store.graph_set_status(thread, status, None, || true).unwrap();
        }
        assert_eq!(store.graph_prune(now() - chrono::Duration::days(1)).unwrap(), 0, "too recent");
        assert_eq!(store.graph_prune(now() + chrono::Duration::days(1)).unwrap(), 2);
        assert!(store.graph_thread("done").unwrap().is_none() && store.graph_thread("failed").unwrap().is_none());
        assert_eq!(store.graph_checkpoints("waiting").unwrap().len(), 2);
    }

    #[test]
    fn a_dead_thread_gets_no_writes() {
        let store = Store::open_in_memory().unwrap();
        save(&store, &row("t", 0));
        assert!(!store.graph_save_checkpoint(&row("t", 1), 3, || false).unwrap());
        assert!(!store.graph_set_status("t", GRAPH_DONE, None, || false).unwrap());
        assert_eq!(store.graph_checkpoints("t").unwrap().len(), 1);
        assert_eq!(store.graph_thread("t").unwrap().unwrap().status, GRAPH_RUNNING);
    }

    #[test]
    fn running_threads_orphaned_by_a_crash_are_pruned_too() {
        let store = Store::open_in_memory().unwrap();
        save(&store, &row("orphan", 0));
        assert_eq!(store.graph_prune(now() + chrono::Duration::days(1)).unwrap(), 1);
    }
}
