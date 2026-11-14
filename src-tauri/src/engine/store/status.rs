use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use super::{now, Store};
use anyhow::{bail, Context, Result};

/// `Draft -> CvReady -> Submitting -> Applied -> {Interview | Rejected | Offer}`, plus `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    Draft,
    CvReady,
    Submitting,
    Applied,
    Interview,
    Rejected,
    Offer,
    Failed,
}

impl Status {
    pub const ALL: [Status; 8] = [
        Status::Draft,
        Status::CvReady,
        Status::Submitting,
        Status::Applied,
        Status::Interview,
        Status::Rejected,
        Status::Offer,
        Status::Failed,
    ];

    /// Stable identifier stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Draft => "draft",
            Status::CvReady => "cv_ready",
            Status::Submitting => "submitting",
            Status::Applied => "applied",
            Status::Interview => "interview",
            Status::Rejected => "rejected",
            Status::Offer => "offer",
            Status::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|st| st.as_str() == s)
    }

    /// Human label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Status::Draft => "Draft",
            Status::CvReady => "CV ready",
            Status::Submitting => "Submitting",
            Status::Applied => "Applied",
            Status::Interview => "Interview",
            Status::Rejected => "Rejected",
            Status::Offer => "Offer",
            Status::Failed => "Failed",
        }
    }

    /// Position on the forward-only ladder. `Failed` is off the ladder (rank 0 like `Draft`).
    pub fn rank(self) -> u8 {
        match self {
            Status::Draft | Status::Failed => 0,
            Status::CvReady => 1,
            Status::Submitting => 2,
            Status::Applied => 3,
            Status::Interview => 4,
            Status::Rejected => 5,
            Status::Offer => 6,
        }
    }

    /// Forward-only rule. `Failed` may go back to `CvReady`/`Submitting` and is reachable
    /// from `Draft`/`CvReady`/`Submitting` only. Everything else must strictly increase in rank
    /// (so `Offer -> Rejected` is refused).
    pub fn can_advance_to(self, next: Status) -> bool {
        match (self, next) {
            (Status::Failed, n) => matches!(n, Status::CvReady | Status::Submitting),
            (s, Status::Failed) => matches!(s, Status::Draft | Status::CvReady | Status::Submitting),
            (s, n) => n.rank() > s.rank(),
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Status {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Status::parse(s).with_context(|| format!("unknown status `{s}`"))
    }
}

impl ToSql for Status {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(self.as_str().into())
    }
}

impl FromSql for Status {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let s = value.as_str()?;
        Status::parse(s).ok_or_else(|| FromSqlError::Other(format!("unknown status `{s}`").into()))
    }
}

impl Store {
    /// Move an application to `to` if the forward-only rule allows it (same status is a no-op),
    /// and append a `status` event. Use [`Store::force_status`] for explicit user overrides.
    pub fn set_status(&self, app_id: i64, to: Status, message: &str) -> Result<()> {
        self.change_status(app_id, to, message, false)
    }

    /// Like [`Store::set_status`] but skips the forward-only rule (manual user override).
    pub fn force_status(&self, app_id: i64, to: Status, message: &str) -> Result<()> {
        self.change_status(app_id, to, message, true)
    }

    fn change_status(&self, app_id: i64, to: Status, message: &str, force: bool) -> Result<()> {
        self.with_tx(|tx| {
            let from: Status = tx
                .query_row("SELECT status FROM applications WHERE id = ?1", [app_id], |r| r.get(0))
                .with_context(|| format!("application {app_id} not found"))?;
            if from == to {
                return Ok(());
            }
            if !force && !from.can_advance_to(to) {
                bail!("cannot move application {app_id} from {from} to {to}");
            }
            let at = now();
            tx.execute(
                "UPDATE applications SET status = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![to, at, app_id],
            )?;
            let text = if message.is_empty() {
                format!("{from} -> {to}")
            } else {
                format!("{from} -> {to}: {message}")
            };
            super::events::insert_event(tx, app_id, "status", &text)?;
            Ok(())
        })
    }
}
