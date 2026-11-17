use std::fmt;
use std::str::FromStr;

use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};

use super::{now, Store};
use anyhow::{bail, Context, Result};

/// `Draft -> CvReady -> Submitting -> Applied -> {Interview | Rejected | Offer}`, plus `Failed` and
/// `Dismissed` (the user is no longer interested in a job whose CV was prepared; only Restore,
/// through [`Store::force_status`], takes it back).
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
    Dismissed,
}

impl Status {
    pub const ALL: [Status; 9] = [
        Status::Draft,
        Status::CvReady,
        Status::Submitting,
        Status::Applied,
        Status::Interview,
        Status::Rejected,
        Status::Offer,
        Status::Failed,
        Status::Dismissed,
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
            Status::Dismissed => "dismissed",
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
            Status::Dismissed => "Dismissed",
        }
    }

    /// Position on the forward-only ladder. `Failed` and `Dismissed` are off the ladder (rank 0
    /// like `Draft`).
    pub fn rank(self) -> u8 {
        match self {
            Status::Draft | Status::Failed | Status::Dismissed => 0,
            Status::CvReady => 1,
            Status::Submitting => 2,
            Status::Applied => 3,
            Status::Interview => 4,
            Status::Rejected => 5,
            Status::Offer => 6,
        }
    }

    /// Forward-only rule. `Failed` may go back to `CvReady`/`Submitting` and is reachable
    /// from `Draft`/`CvReady`/`Submitting` only. `Dismissed` is reachable from `Draft`/`CvReady`/
    /// `Failed` only (never while a submit is in flight) and is left only by force (Restore).
    /// Everything else must strictly increase in rank (so `Offer -> Rejected` is refused).
    pub fn can_advance_to(self, next: Status) -> bool {
        match (self, next) {
            (Status::Dismissed, _) => false,
            (s, Status::Dismissed) => matches!(s, Status::Draft | Status::CvReady | Status::Failed),
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
        self.with_tx(|tx| change_status_in(tx, app_id, to, message, force).map(|_| ()))
    }
}

/// The status change of [`Store::set_status`] / [`Store::force_status`] inside the caller's
/// transaction. Returns the status the application had.
pub(super) fn change_status_in(
    tx: &rusqlite::Transaction,
    app_id: i64,
    to: Status,
    message: &str,
    force: bool,
) -> Result<Status> {
    let from: Status = tx
        .query_row("SELECT status FROM applications WHERE id = ?1", [app_id], |r| r.get(0))
        .with_context(|| format!("application {app_id} not found"))?;
    if from == to {
        return Ok(from);
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
    Ok(from)
}

#[cfg(test)]
mod tests {
    use super::Status;

    #[test]
    fn dismissed_is_reached_only_before_a_submit_and_left_only_by_force() {
        for from in Status::ALL {
            let allowed = matches!(from, Status::Draft | Status::CvReady | Status::Failed);
            assert_eq!(from.can_advance_to(Status::Dismissed), allowed, "{from:?} -> Dismissed");
            assert!(!Status::Dismissed.can_advance_to(from), "Dismissed -> {from:?}");
        }
        assert_eq!(Status::parse("dismissed"), Some(Status::Dismissed));
        assert_eq!(Status::Dismissed.label(), "Dismissed");
        assert!(Status::ALL.contains(&Status::Dismissed));
    }
}
