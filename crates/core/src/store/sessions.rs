use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// A managed browser session (profile dir `sessions/<id>/`).
#[derive(Clone, Debug)]
pub struct Session {
    pub id: i64,
    pub name: String,
    pub start_url: String,
    /// JSON array of domains the session is logged in to.
    pub domains_json: String,
    /// `new`, `ok` or `expired`.
    pub status: String,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct NewSession {
    pub name: String,
    pub start_url: String,
}

const COLS: &str = "id, name, start_url, domains_json, status, last_verified_at, created_at";

impl Session {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            name: r.get(1)?,
            start_url: r.get(2)?,
            domains_json: r.get(3)?,
            status: r.get(4)?,
            last_verified_at: r.get(5)?,
            created_at: r.get(6)?,
        })
    }
}

impl Store {
    pub fn add_session(&self, new: &NewSession) -> Result<Session> {
        let id = self.with_conn(|c| {
            c.execute(
                "INSERT INTO sessions (name, start_url, created_at) VALUES (?1, ?2, ?3)",
                params![new.name, new.start_url, now()],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.get_session(id)?.context("session vanished after insert")
    }

    pub fn get_session(&self, id: i64) -> Result<Option<Session>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM sessions WHERE id = ?1"), [id], Session::from_row)
                .optional()?)
        })
    }

    pub fn list_sessions(&self) -> Result<Vec<Session>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLS} FROM sessions ORDER BY id"))?;
            let rows = stmt.query_map([], Session::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn update_session_status(&self, id: i64, status: &str, verified_at: Option<DateTime<Utc>>) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE sessions SET status = ?1, last_verified_at = COALESCE(?2, last_verified_at) WHERE id = ?3",
                params![status, verified_at, id],
            )?;
            anyhow::ensure!(n == 1, "session {id} not found");
            Ok(())
        })
    }

    pub fn update_session_domains(&self, id: i64, domains_json: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute("UPDATE sessions SET domains_json = ?1 WHERE id = ?2", params![domains_json, id])?;
            Ok(())
        })
    }

    pub fn delete_session(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}
