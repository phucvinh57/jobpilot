use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// Where a found job stands. A lead becomes an application only through `Promoted`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeadState {
    /// Waiting in the queue.
    #[default]
    New,
    /// Filtered out or dismissed by the user; kept so it is not fetched again.
    Dismissed,
    /// A CV was prepared: `app_id` points at the application.
    Promoted,
}

impl LeadState {
    /// `new` / `dismissed` / `promoted` (as stored).
    pub fn as_str(self) -> &'static str {
        match self {
            LeadState::New => "new",
            LeadState::Dismissed => "dismissed",
            LeadState::Promoted => "promoted",
        }
    }

    /// Inverse of [`LeadState::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "new" => Some(LeadState::New),
            "dismissed" => Some(LeadState::Dismissed),
            "promoted" => Some(LeadState::Promoted),
            _ => None,
        }
    }
}

/// A job posting found by discovery, scored against the preferences.
#[derive(Clone, Debug)]
pub struct Lead {
    pub id: i64,
    /// The source that found it (`None` once that source was deleted).
    pub source_id: Option<i64>,
    /// Canonical posting URL, unique.
    pub url: String,
    /// `slug(company) + slug(role)`, to catch the same job on another board.
    pub dedup_key: String,
    pub company: String,
    pub role: String,
    pub location: String,
    /// `<data_dir>/leads/<id>/jd.md`.
    pub jd_path: Option<PathBuf>,
    /// Weighted match, 0 to 100; `None` when it was never scored (filtered out).
    pub score: Option<f64>,
    /// Per-criterion scores with a reason each, as JSON text.
    pub score_json: Option<String>,
    /// Job insight as JSON text (P11).
    pub insight_json: Option<String>,
    pub state: LeadState,
    pub dismiss_reason: Option<String>,
    pub app_id: Option<i64>,
    pub found_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Default)]
pub struct NewLead {
    pub source_id: Option<i64>,
    pub url: String,
    pub dedup_key: String,
    pub company: String,
    pub role: String,
    pub location: String,
    pub jd_path: Option<PathBuf>,
    pub score: Option<f64>,
    pub score_json: Option<String>,
    pub insight_json: Option<String>,
    /// `New`, or `Dismissed` (with `dismiss_reason`) for a lead the hard filters rejected.
    pub state: LeadState,
    pub dismiss_reason: Option<String>,
}

const COLS: &str = "id, source_id, url, dedup_key, company, role, location, jd_path, score, score_json, \
                    insight_json, state, dismiss_reason, app_id, found_at";

impl Lead {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let state: String = r.get(11)?;
        Ok(Self {
            id: r.get(0)?,
            source_id: r.get(1)?,
            url: r.get(2)?,
            dedup_key: r.get(3)?,
            company: r.get(4)?,
            role: r.get(5)?,
            location: r.get(6)?,
            jd_path: r.get::<_, Option<String>>(7)?.map(PathBuf::from),
            score: r.get(8)?,
            score_json: r.get(9)?,
            insight_json: r.get(10)?,
            state: LeadState::parse(&state).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(11, rusqlite::types::Type::Text, format!("state `{state}`").into())
            })?,
            dismiss_reason: r.get(12)?,
            app_id: r.get(13)?,
            found_at: r.get(14)?,
        })
    }
}

impl Store {
    /// Insert a lead. Fails when the URL is already known (check with [`Store::lead_exists`]).
    pub fn add_lead(&self, new: &NewLead) -> Result<Lead> {
        let id = self
            .with_conn(|c| {
                c.execute(
                    "INSERT INTO leads (source_id, url, dedup_key, company, role, location, jd_path, score,
                                        score_json, insight_json, state, dismiss_reason, found_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                    params![
                        new.source_id,
                        new.url,
                        new.dedup_key,
                        new.company,
                        new.role,
                        new.location,
                        new.jd_path.as_ref().map(|p| p.to_string_lossy().into_owned()),
                        new.score,
                        new.score_json,
                        new.insight_json,
                        new.state.as_str(),
                        new.dismiss_reason,
                        now()
                    ],
                )?;
                Ok(c.last_insert_rowid())
            })
            .with_context(|| format!("saving lead `{}`", new.url))?;
        self.get_lead(id)?.context("lead vanished after insert")
    }

    pub fn get_lead(&self, id: i64) -> Result<Option<Lead>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM leads WHERE id = ?1"), [id], Lead::from_row).optional()?)
        })
    }

    /// The lead with this exact `url`, or (when `dedup_key` is not empty) the same company and role.
    pub fn find_lead(&self, url: &str, dedup_key: &str) -> Result<Option<Lead>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLS} FROM leads WHERE url = ?1 OR (?2 <> '' AND dedup_key = ?2) ORDER BY id LIMIT 1"),
                params![url, dedup_key],
                Lead::from_row,
            )
            .optional()?)
        })
    }

    /// Is this posting already known, in any state? (see [`Store::find_lead`])
    pub fn lead_exists(&self, url: &str, dedup_key: &str) -> Result<bool> {
        Ok(self.find_lead(url, dedup_key)?.is_some())
    }

    /// Leads in `state`, best score first (unscored last), then newest.
    pub fn list_leads(&self, state: LeadState) -> Result<Vec<Lead>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM leads WHERE state = ?1 ORDER BY score DESC, found_at DESC, id DESC"
            ))?;
            let rows = stmt.query_map([state.as_str()], Lead::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Record where the posting text was saved.
    pub fn set_lead_jd_path(&self, id: i64, jd_path: &std::path::Path) -> Result<()> {
        self.update_lead("jd_path = ?1", params![jd_path.to_string_lossy(), id], id)
    }

    /// The user (or a filter) rejected the lead; `reason` may be empty.
    pub fn dismiss_lead(&self, id: i64, reason: &str) -> Result<()> {
        self.update_lead("state = 'dismissed', dismiss_reason = ?1", params![reason, id], id)
    }

    /// The lead became application `app_id`.
    pub fn promote_lead(&self, id: i64, app_id: i64) -> Result<()> {
        self.update_lead("state = 'promoted', app_id = ?1", params![app_id, id], id)
    }

    /// `SET <assignments> WHERE id = ?N`, where N is the last parameter.
    fn update_lead(&self, set: &str, args: &[&dyn rusqlite::ToSql], id: i64) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(&format!("UPDATE leads SET {set} WHERE id = ?{}", args.len()), args)?;
            anyhow::ensure!(n == 1, "lead {id} not found");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{NewApplication, NewSource, SourcePurpose};

    fn new(url: &str, key: &str, score: Option<f64>) -> NewLead {
        NewLead {
            url: url.into(),
            dedup_key: key.into(),
            company: "Acme".into(),
            role: "Backend Engineer".into(),
            score,
            ..Default::default()
        }
    }

    #[test]
    fn queue_is_sorted_by_score_and_filtered_by_state() {
        let store = Store::open_in_memory().unwrap();
        let low = store.add_lead(&new("https://a.test/1", "a-1", Some(41.5))).unwrap();
        let unscored = store.add_lead(&new("https://a.test/2", "a-2", None)).unwrap();
        let high = store.add_lead(&new("https://a.test/3", "a-3", Some(88.0))).unwrap();
        let mid = store.add_lead(&new("https://a.test/4", "a-4", Some(60.0))).unwrap();
        assert_eq!(low.state, LeadState::New);
        assert_eq!(low.score, Some(41.5));

        let ids = |state| store.list_leads(state).unwrap().iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(ids(LeadState::New), [high.id, mid.id, low.id, unscored.id]);

        store.dismiss_lead(mid.id, "too junior").unwrap();
        assert_eq!(ids(LeadState::New), [high.id, low.id, unscored.id]);
        let dismissed = store.get_lead(mid.id).unwrap().unwrap();
        assert_eq!((dismissed.state, dismissed.dismiss_reason.as_deref()), (LeadState::Dismissed, Some("too junior")));
        assert_eq!(ids(LeadState::Dismissed), [mid.id]);
        assert!(ids(LeadState::Promoted).is_empty());
    }

    #[test]
    fn exists_by_url_or_dedup_key() {
        let store = Store::open_in_memory().unwrap();
        store.add_lead(&new("https://a.test/1", "acme-backend-engineer", Some(70.0))).unwrap();

        assert!(store.lead_exists("https://a.test/1", "").unwrap(), "same URL");
        assert!(store.lead_exists("https://b.test/other", "acme-backend-engineer").unwrap(), "same company and role");
        assert!(!store.lead_exists("https://b.test/other", "foo-sre").unwrap());
        assert!(!store.lead_exists("https://b.test/other", "").unwrap(), "an empty key never matches");
        assert!(store.add_lead(&new("https://a.test/1", "x", None)).is_err(), "URL is unique");
        assert_eq!(store.find_lead("https://b.test/other", "acme-backend-engineer").unwrap().unwrap().url, "https://a.test/1");
    }

    #[test]
    fn filtered_leads_are_stored_dismissed_and_promotion_links_the_application() {
        let store = Store::open_in_memory().unwrap();
        let src = store
            .add_source(&NewSource { kind: "lever".into(), purpose: SourcePurpose::Jobs, ..Default::default() })
            .unwrap();
        let filtered = store
            .add_lead(&NewLead {
                source_id: Some(src.id),
                state: LeadState::Dismissed,
                dismiss_reason: Some("excluded company".into()),
                ..new("https://a.test/9", "a-9", None)
            })
            .unwrap();
        assert_eq!(filtered.state, LeadState::Dismissed);
        assert!(store.lead_exists("https://a.test/9", "").unwrap(), "so it is not fetched again");

        let lead = store.add_lead(&new("https://a.test/10", "a-10", Some(75.0))).unwrap();
        store.set_lead_jd_path(lead.id, std::path::Path::new("/tmp/leads/10/jd.md")).unwrap();
        let app = store
            .create_application(&NewApplication { slug: "acme-be".into(), dir: "/tmp/acme-be".into(), ..Default::default() })
            .unwrap();
        store.promote_lead(lead.id, app.id).unwrap();
        let got = store.get_lead(lead.id).unwrap().unwrap();
        assert_eq!((got.state, got.app_id), (LeadState::Promoted, Some(app.id)));
        assert_eq!(got.jd_path.as_deref(), Some(std::path::Path::new("/tmp/leads/10/jd.md")));
        assert!(store.promote_lead(999, app.id).is_err());

        // Deleting the source or the application keeps the lead.
        store.delete_source(src.id).unwrap();
        assert_eq!(store.get_lead(filtered.id).unwrap().unwrap().source_id, None);
        store.delete_application(app.id).unwrap();
        assert_eq!(store.get_lead(lead.id).unwrap().unwrap().app_id, None);
    }
}
