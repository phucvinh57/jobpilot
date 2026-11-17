use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// What a source feeds: job postings or market-trend items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SourcePurpose {
    #[default]
    Jobs,
    Trends,
}

impl SourcePurpose {
    /// `jobs` / `trends` (as stored).
    pub fn as_str(self) -> &'static str {
        match self {
            SourcePurpose::Jobs => "jobs",
            SourcePurpose::Trends => "trends",
        }
    }

    /// Inverse of [`SourcePurpose::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "jobs" => Some(SourcePurpose::Jobs),
            "trends" => Some(SourcePurpose::Trends),
            _ => None,
        }
    }
}

/// A place to look for jobs or trend items. `kind` names the adapter (`greenhouse`, `hn_hiring`,
/// `rss`, ...); the adapter interprets `url`.
#[derive(Clone, Debug)]
pub struct Source {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub url: String,
    pub purpose: SourcePurpose,
    pub enabled: bool,
    /// Session (a site the user is signed in to) used to reach a login-walled site.
    pub session_id: Option<i64>,
    pub last_run_at: Option<DateTime<Utc>>,
    /// Free text set by the runner: `ok`, `needs_human`, `failed: ...`.
    pub last_status: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct NewSource {
    pub kind: String,
    pub name: String,
    pub url: String,
    pub purpose: SourcePurpose,
    pub session_id: Option<i64>,
}

const COLS: &str = "id, kind, name, url, purpose, enabled, session_id, last_run_at, last_status";

impl Source {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let purpose: String = r.get(4)?;
        Ok(Self {
            id: r.get(0)?,
            kind: r.get(1)?,
            name: r.get(2)?,
            url: r.get(3)?,
            purpose: SourcePurpose::parse(&purpose).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, format!("purpose `{purpose}`").into())
            })?,
            enabled: r.get(5)?,
            session_id: r.get(6)?,
            last_run_at: r.get(7)?,
            last_status: r.get(8)?,
        })
    }
}

impl Store {
    /// Add an enabled source.
    pub fn add_source(&self, new: &NewSource) -> Result<Source> {
        let id = self.with_conn(|c| {
            c.execute(
                "INSERT INTO sources (kind, name, url, purpose, session_id) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![new.kind, new.name, new.url, new.purpose.as_str(), new.session_id],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.get_source(id)?.context("source vanished after insert")
    }

    pub fn get_source(&self, id: i64) -> Result<Option<Source>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM sources WHERE id = ?1"), [id], Source::from_row).optional()?)
        })
    }

    /// Sources of one purpose, oldest first.
    pub fn list_sources(&self, purpose: SourcePurpose) -> Result<Vec<Source>> {
        self.query_sources(purpose, false)
    }

    /// Only the enabled sources of one purpose (what a run iterates over).
    pub fn list_enabled_sources(&self, purpose: SourcePurpose) -> Result<Vec<Source>> {
        self.query_sources(purpose, true)
    }

    fn query_sources(&self, purpose: SourcePurpose, enabled_only: bool) -> Result<Vec<Source>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM sources WHERE purpose = ?1 AND (?2 = 0 OR enabled = 1) ORDER BY id"
            ))?;
            let rows = stmt.query_map(params![purpose.as_str(), enabled_only], Source::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn set_source_enabled(&self, id: i64, enabled: bool) -> Result<()> {
        self.update_source("enabled = ?1", params![enabled, id], id)
    }

    /// Bind (or unbind) the managed session used for this source.
    pub fn set_source_session(&self, id: i64, session_id: Option<i64>) -> Result<()> {
        self.update_source("session_id = ?1", params![session_id, id], id)
    }

    /// A run reached its end: stamp `last_run_at` with now and store `status`.
    pub fn set_source_run(&self, id: i64, status: &str) -> Result<()> {
        self.update_source("last_run_at = ?1, last_status = ?2", params![now(), status, id], id)
    }

    /// Change only the status (a run paused for the user is not finished).
    pub fn set_source_status(&self, id: i64, status: &str) -> Result<()> {
        self.update_source("last_status = ?1", params![status, id], id)
    }

    /// Leads and trend items of the source stay (their `source_id` becomes NULL).
    pub fn delete_source(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM sources WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// `SET <assignments> WHERE id = ?N`, where N is the last parameter.
    fn update_source(&self, set: &str, args: &[&dyn rusqlite::ToSql], id: i64) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(&format!("UPDATE sources SET {set} WHERE id = ?{}", args.len()), args)?;
            anyhow::ensure!(n == 1, "source {id} not found");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NewSession;

    fn new(kind: &str, purpose: SourcePurpose) -> NewSource {
        NewSource { kind: kind.into(), name: kind.into(), url: format!("https://{kind}.test"), purpose, session_id: None }
    }

    #[test]
    fn crud_filters_and_run_status() {
        let store = Store::open_in_memory().unwrap();
        let gh = store.add_source(&new("greenhouse", SourcePurpose::Jobs)).unwrap();
        let hn = store.add_source(&new("hn_hiring", SourcePurpose::Jobs)).unwrap();
        let rss = store.add_source(&new("rss", SourcePurpose::Trends)).unwrap();
        assert!(gh.enabled && gh.last_run_at.is_none() && gh.last_status.is_none());

        assert_eq!(store.list_sources(SourcePurpose::Jobs).unwrap().len(), 2);
        assert_eq!(store.list_sources(SourcePurpose::Trends).unwrap()[0].id, rss.id);

        store.set_source_enabled(hn.id, false).unwrap();
        let enabled = store.list_enabled_sources(SourcePurpose::Jobs).unwrap();
        assert_eq!(enabled.iter().map(|s| s.id).collect::<Vec<_>>(), [gh.id]);
        assert_eq!(store.list_sources(SourcePurpose::Jobs).unwrap().len(), 2, "disabled ones are still listed");

        store.set_source_status(gh.id, "needs_human").unwrap();
        let got = store.get_source(gh.id).unwrap().unwrap();
        assert_eq!(got.last_status.as_deref(), Some("needs_human"));
        assert!(got.last_run_at.is_none(), "paused, not finished");
        store.set_source_run(gh.id, "ok").unwrap();
        let got = store.get_source(gh.id).unwrap().unwrap();
        assert_eq!((got.last_status.as_deref(), got.last_run_at.is_some()), (Some("ok"), true));

        assert!(store.set_source_enabled(999, true).is_err());
        store.delete_source(rss.id).unwrap();
        assert!(store.get_source(rss.id).unwrap().is_none());
    }

    #[test]
    fn session_binding_survives_until_the_session_is_deleted() {
        let store = Store::open_in_memory().unwrap();
        let session = store.add_session(&NewSession { name: "ITviec".into(), start_url: "https://itviec.com".into() }).unwrap();
        let src = store.add_source(&NewSource { session_id: Some(session.id), ..new("itviec", SourcePurpose::Jobs) }).unwrap();
        assert_eq!(src.session_id, Some(session.id));
        store.delete_session(session.id).unwrap();
        assert_eq!(store.get_source(src.id).unwrap().unwrap().session_id, None);
        store.set_source_session(src.id, None).unwrap();
    }

    #[test]
    fn purpose_is_checked_by_the_schema() {
        let store = Store::open_in_memory().unwrap();
        let bad = store.with_conn(|c| Ok(c.execute("INSERT INTO sources (kind, purpose) VALUES ('x', 'other')", [])?));
        assert!(bad.is_err());
    }
}
