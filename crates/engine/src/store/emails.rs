use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::Store;

/// A Gmail message seen by the sync, with its classification. Bodies are not stored.
#[derive(Clone, Debug)]
pub struct Email {
    /// Gmail message id.
    pub id: String,
    pub app_id: Option<i64>,
    pub from_addr: String,
    pub subject: String,
    pub snippet: String,
    pub received_at: DateTime<Utc>,
    /// `confirmation | rejection | interview | offer | other`.
    pub kind: Option<String>,
    pub confidence: Option<f64>,
    pub summary: Option<String>,
    /// Whether it already moved an application's status.
    pub applied: bool,
}

const COLS: &str = "id, app_id, from_addr, subject, snippet, received_at, kind, confidence, summary, applied";

impl Email {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            app_id: r.get(1)?,
            from_addr: r.get(2)?,
            subject: r.get(3)?,
            snippet: r.get(4)?,
            received_at: r.get(5)?,
            kind: r.get(6)?,
            confidence: r.get(7)?,
            summary: r.get(8)?,
            applied: r.get(9)?,
        })
    }
}

impl Store {
    /// Insert or replace by Gmail id.
    pub fn upsert_email(&self, e: &Email) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT OR REPLACE INTO emails (id, app_id, from_addr, subject, snippet, received_at, kind, confidence, summary, applied)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    e.id,
                    e.app_id,
                    e.from_addr,
                    e.subject,
                    e.snippet,
                    e.received_at,
                    e.kind,
                    e.confidence,
                    e.summary,
                    e.applied
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_email(&self, id: &str) -> Result<Option<Email>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM emails WHERE id = ?1"), [id], Email::from_row).optional()?)
        })
    }

    /// Newest first; `Some(app_id)` filters to one application.
    pub fn list_emails(&self, app_id: Option<i64>) -> Result<Vec<Email>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM emails WHERE (?1 IS NULL OR app_id = ?1) ORDER BY received_at DESC"
            ))?;
            let rows = stmt.query_map([app_id], Email::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }
}
