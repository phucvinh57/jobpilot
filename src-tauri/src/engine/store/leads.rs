use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{applications, now, Application, NewApplication, Store};

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

impl Lead {
    /// Is the lead one the user can still act on: `new`, or `promoted` with its application gone
    /// (deleted since), which makes it promotable again.
    pub fn in_queue(&self) -> bool {
        self.state == LeadState::New || (self.state == LeadState::Promoted && self.app_id.is_none())
    }
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
    /// Job insight as JSON text.
    pub insight_json: Option<String>,
    pub state: LeadState,
    pub dismiss_reason: Option<String>,
    pub app_id: Option<i64>,
    pub found_at: DateTime<Utc>,
    /// Why the saved preferences hide this `new` lead from the queue (the hard filters' reason);
    /// `None` when it is shown. Unlike a dismissal it is undone when the preferences change back
    /// (see `discover::refilter`).
    pub filtered_reason: Option<String>,
    /// When it was dismissed (by the user or a filter); `None` for a lead that is not dismissed,
    /// or one dismissed before this was recorded (its `found_at` is the best guess then).
    pub dismissed_at: Option<DateTime<Utc>>,
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
                    insight_json, state, dismiss_reason, app_id, found_at, filtered_reason, dismissed_at";

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
            filtered_reason: r.get(15)?,
            dismissed_at: r.get(16)?,
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
                                        score_json, insight_json, state, dismiss_reason, found_at, dismissed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                             CASE WHEN ?11 = 'dismissed' THEN ?13 END)",
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

    /// The lead application `app_id` was prepared from, if it came from one.
    pub fn lead_of_application(&self, app_id: i64) -> Result<Option<Lead>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM leads WHERE app_id = ?1 ORDER BY id LIMIT 1"), [app_id], Lead::from_row)
                .optional()?)
        })
    }

    /// Is this posting already known, in any state? (see [`Store::find_lead`])
    pub fn lead_exists(&self, url: &str, dedup_key: &str) -> Result<bool> {
        Ok(self.find_lead(url, dedup_key)?.is_some())
    }

    /// The leads the queue shows: every `new` lead, plus promoted ones whose application was
    /// deleted (they can be prepared again), except those the preferences hide
    /// ([`Lead::filtered_reason`]). Best score first (unscored last), then newest.
    pub fn list_queue_leads(&self) -> Result<Vec<Lead>> {
        self.queue_leads(&format!("{IN_QUEUE} AND {SHOWN}"))
    }

    /// Every lead in the queue (see [`Lead::in_queue`]), also the ones the preferences hide: what
    /// a dismissal of a whole company must reach. Same order as [`Store::list_queue_leads`].
    pub fn list_queue_leads_with_hidden(&self) -> Result<Vec<Lead>> {
        self.queue_leads(IN_QUEUE)
    }

    fn queue_leads(&self, filter: &str) -> Result<Vec<Lead>> {
        self.with_conn(|c| {
            let mut stmt =
                c.prepare(&format!("SELECT {COLS} FROM leads WHERE {filter} ORDER BY score DESC, found_at DESC, id DESC"))?;
            let rows = stmt.query_map([], Lead::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// How many leads the queue shows (see [`Store::list_queue_leads`]), without reading them.
    pub fn count_queue_leads(&self) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 =
                c.query_row(&format!("SELECT COUNT(*) FROM leads WHERE {IN_QUEUE} AND {SHOWN}"), [], |r| r.get(0))?;
            Ok(usize::try_from(n).unwrap_or(0))
        })
    }

    /// Hide a `new` lead from the queue with the preferences' `reason`, or show it again (`None`).
    /// The state does not change, so this can always be undone. Returns whether the lead was
    /// changed: `false` when it already had that value, or is no longer `new`.
    pub fn set_lead_filtered(&self, id: i64, reason: Option<&str>) -> Result<bool> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE leads SET filtered_reason = ?1 WHERE id = ?2 AND state = 'new' AND filtered_reason IS NOT ?1",
                params![reason, id],
            )?;
            Ok(n == 1)
        })
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
        self.update_lead("jd_path = ?1", params![jd_path.to_string_lossy(), id], id, "")
    }

    /// The user (or a filter) rejected the lead; `reason` may be empty. Only a lead in the queue
    /// can be dismissed (see [`Lead::in_queue`]): states move forward only.
    pub fn dismiss_lead(&self, id: i64, reason: &str) -> Result<()> {
        self.update_lead(
            "state = 'dismissed', dismiss_reason = ?1, dismissed_at = ?2",
            params![reason, now(), id],
            id,
            &format!(" AND {IN_QUEUE}"),
        )
    }

    /// "Restore" a dismissed lead (by the user or a filter): `new` again, its reason and time
    /// forgotten, and shown (the caller hides it again when the saved preferences still filter it
    /// out, see [`Store::set_lead_filtered`]). Only a dismissed lead can be restored.
    pub fn restore_lead(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE leads SET state = 'new', dismiss_reason = NULL, dismissed_at = NULL, filtered_reason = NULL
                 WHERE id = ?1 AND state = 'dismissed'",
                [id],
            )?;
            anyhow::ensure!(n == 1, "lead {id} not found or not dismissed");
            Ok(())
        })
    }

    /// The dismissed leads (by the user or a filter) without an application, most recently
    /// dismissed first (a lead dismissed before that was recorded counts from when it was found).
    /// A `new` lead the preferences only hide ([`Lead::filtered_reason`]) is not dismissed.
    pub fn list_dismissed_leads(&self) -> Result<Vec<Lead>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM leads WHERE state = 'dismissed' AND app_id IS NULL
                 ORDER BY COALESCE(dismissed_at, found_at) DESC, id DESC"
            ))?;
            let rows = stmt.query_map([], Lead::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// How many leads [`Store::list_dismissed_leads`] lists, without reading them.
    pub fn count_dismissed_leads(&self) -> Result<usize> {
        self.with_conn(|c| {
            let n: i64 =
                c.query_row("SELECT COUNT(*) FROM leads WHERE state = 'dismissed' AND app_id IS NULL", [], |r| r.get(0))?;
            Ok(usize::try_from(n).unwrap_or(0))
        })
    }

    /// The lead became application `app_id`. Only a lead in the queue can be promoted.
    pub fn promote_lead(&self, id: i64, app_id: i64) -> Result<()> {
        self.update_lead(
            "state = 'promoted', app_id = ?1, filtered_reason = NULL",
            params![app_id, id],
            id,
            &format!(" AND {IN_QUEUE}"),
        )
    }

    /// Create the Draft application `new` for a `new` lead and mark the lead promoted, in one
    /// transaction: a crash leaves either both or neither, so a second try never makes a second
    /// application. `insight_json` is copied to the application. A lead that is already promoted
    /// returns its application and changes nothing, unless that application was deleted since: then
    /// a new one is created and linked (the lead stays `promoted`).
    pub fn promote_lead_to_application(&self, id: i64, new: &NewApplication, insight_json: Option<&str>) -> Result<Application> {
        let app_id = self.with_tx(|tx| {
            let (state, app_id): (String, Option<i64>) = tx
                .query_row("SELECT state, app_id FROM leads WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
                .with_context(|| format!("lead {id} not found"))?;
            match (state.as_str(), app_id) {
                ("promoted", Some(app_id)) => return Ok(app_id),
                ("promoted", None) | ("new", _) => {}
                (other, _) => anyhow::bail!("lead {id} is {other}: only a new lead can be promoted"),
            }
            let app_id = applications::insert_application(tx, new)?;
            if insight_json.is_some() {
                tx.execute("UPDATE applications SET insight_json = ?1 WHERE id = ?2", params![insight_json, app_id])?;
            }
            // The user chose it: a filter that hid it meanwhile no longer applies.
            tx.execute(
                "UPDATE leads SET state = 'promoted', app_id = ?1, filtered_reason = NULL WHERE id = ?2",
                params![app_id, id],
            )?;
            Ok(app_id)
        })?;
        self.get_application(app_id)?.context("application vanished after insert")
    }

    /// `SET <assignments> WHERE id = ?N<guard>`, where N is the last parameter.
    fn update_lead(&self, set: &str, args: &[&dyn rusqlite::ToSql], id: i64, guard: &str) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(&format!("UPDATE leads SET {set} WHERE id = ?{}{guard}", args.len()), args)?;
            anyhow::ensure!(n == 1, "lead {id} not found or no longer in the queue");
            Ok(())
        })
    }
}

/// A lead the user can act on: `new`, or `promoted` whose application was deleted.
const IN_QUEUE: &str = "(state = 'new' OR (state = 'promoted' AND app_id IS NULL))";
/// A lead the preferences do not hide.
const SHOWN: &str = "filtered_reason IS NULL";



#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::store::{NewApplication, NewSource, SourcePurpose};

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

    #[test]
    fn a_promoted_lead_whose_application_was_deleted_is_back_in_the_queue() {
        let store = Store::open_in_memory().unwrap();
        let lead = store.add_lead(&new("https://a.test/1", "a-1", Some(70.0))).unwrap();
        let spec = |slug: &str| NewApplication { slug: slug.into(), dir: format!("/tmp/{slug}").into(), ..Default::default() };
        let first = store.promote_lead_to_application(lead.id, &spec("first"), None).unwrap();
        let ids = |leads: Vec<Lead>| leads.into_iter().map(|l| l.id).collect::<Vec<_>>();
        assert!(ids(store.list_queue_leads().unwrap()).is_empty(), "promoted with its application: not in the queue");
        assert!(store.dismiss_lead(lead.id, "x").is_err() && store.promote_lead(lead.id, first.id).is_err());

        store.delete_application(first.id).unwrap();
        let orphan = store.get_lead(lead.id).unwrap().unwrap();
        assert!(orphan.in_queue() && orphan.state == LeadState::Promoted);
        assert_eq!(ids(store.list_queue_leads().unwrap()), [lead.id]);
        assert!(ids(store.list_leads(LeadState::New).unwrap()).is_empty(), "list_leads keeps meaning the state");

        // Promoting again makes a new application; asking once more returns it.
        let second = store.promote_lead_to_application(lead.id, &spec("second"), Some("{}")).unwrap();
        assert_ne!(second.id, first.id);
        assert_eq!(second.insight_json.as_deref(), Some("{}"));
        assert_eq!(store.promote_lead_to_application(lead.id, &spec("third"), None).unwrap().id, second.id);
        assert_eq!(store.get_lead(lead.id).unwrap().unwrap().app_id, Some(second.id));

        // An orphan can be dismissed; a dismissed lead never comes back.
        store.delete_application(second.id).unwrap();
        store.dismiss_lead(lead.id, "gone").unwrap();
        assert!(store.list_queue_leads().unwrap().is_empty());
        assert!(store.promote_lead_to_application(lead.id, &spec("fourth"), None).is_err());
    }

    #[test]
    fn a_lead_the_preferences_filter_is_hidden_from_the_queue_until_shown_again() {
        let store = Store::open_in_memory().unwrap();
        let shown = store.add_lead(&new("https://a.test/1", "a-1", Some(50.0))).unwrap();
        let hidden = store.add_lead(&new("https://a.test/2", "a-2", Some(90.0))).unwrap();
        assert_eq!(hidden.filtered_reason, None, "a new lead is shown");
        let ids = |leads: Vec<Lead>| leads.into_iter().map(|l| l.id).collect::<Vec<_>>();

        assert!(store.set_lead_filtered(hidden.id, Some("location: hires in United States")).unwrap());
        assert!(!store.set_lead_filtered(hidden.id, Some("location: hires in United States")).unwrap(), "same value, no write");
        let got = store.get_lead(hidden.id).unwrap().unwrap();
        assert_eq!((got.state, got.filtered_reason.as_deref()), (LeadState::New, Some("location: hires in United States")));
        assert_eq!(ids(store.list_queue_leads().unwrap()), [shown.id]);
        assert_eq!(store.count_queue_leads().unwrap(), 1, "the badge does not count it");
        assert_eq!(ids(store.list_queue_leads_with_hidden().unwrap()), [hidden.id, shown.id]);
        assert_eq!(ids(store.list_leads(LeadState::New).unwrap()), [hidden.id, shown.id], "still new");

        assert!(store.set_lead_filtered(hidden.id, None).unwrap());
        assert!(!store.set_lead_filtered(hidden.id, None).unwrap());
        assert_eq!(ids(store.list_queue_leads().unwrap()), [hidden.id, shown.id]);
        assert_eq!(store.count_queue_leads().unwrap(), 2);

        // A hidden lead can still be dismissed or promoted; promoting it shows it again.
        store.set_lead_filtered(hidden.id, Some("x")).unwrap();
        let app = store
            .promote_lead_to_application(hidden.id, &NewApplication { slug: "s".into(), dir: "/tmp/s".into(), ..Default::default() }, None)
            .unwrap();
        assert_eq!(store.get_lead(hidden.id).unwrap().unwrap().filtered_reason, None);
        store.delete_application(app.id).unwrap();
        assert_eq!(ids(store.list_queue_leads().unwrap()), [hidden.id, shown.id], "an orphan is back in the queue");
        assert!(!store.set_lead_filtered(hidden.id, Some("x")).unwrap(), "only a new lead is hidden");
        store.set_lead_filtered(shown.id, Some("x")).unwrap();
        store.dismiss_lead(shown.id, "no").unwrap();
        assert!(!store.set_lead_filtered(shown.id, None).unwrap(), "a dismissed lead stays dismissed");
        assert!(!store.set_lead_filtered(999, None).unwrap());
    }

    #[test]
    fn dismissed_leads_are_listed_newest_first_and_restored_to_the_queue() {
        let store = Store::open_in_memory().unwrap();
        let filtered = store
            .add_lead(&NewLead {
                state: LeadState::Dismissed,
                dismiss_reason: Some("excluded company".into()),
                ..new("https://a.test/1", "a-1", None)
            })
            .unwrap();
        assert!(filtered.dismissed_at.is_some(), "a filtered lead is dismissed when stored");
        let lead = store.add_lead(&new("https://a.test/2", "a-2", Some(80.0))).unwrap();
        assert_eq!(lead.dismissed_at, None);
        // Hidden by the preferences first, then dismissed by the user.
        store.set_lead_filtered(lead.id, Some("location: hires in United States")).unwrap();
        store.dismiss_lead(lead.id, "too junior").unwrap();
        // A lead the preferences only hide is not dismissed: it is neither listed nor counted.
        let hidden = store.add_lead(&new("https://a.test/3", "a-3", Some(60.0))).unwrap();
        store.set_lead_filtered(hidden.id, Some("location: hires in United States")).unwrap();
        let got = store.get_lead(lead.id).unwrap().unwrap();
        assert!(got.dismissed_at.is_some_and(|at| at >= filtered.dismissed_at.unwrap()));

        let ids = |leads: Vec<Lead>| leads.into_iter().map(|l| l.id).collect::<Vec<_>>();
        assert_eq!(ids(store.list_dismissed_leads().unwrap()), [lead.id, filtered.id]);
        assert_eq!(store.count_dismissed_leads().unwrap(), 2);

        store.restore_lead(lead.id).unwrap();
        let back = store.get_lead(lead.id).unwrap().unwrap();
        assert_eq!((back.state, back.dismiss_reason, back.dismissed_at), (LeadState::New, None, None));
        assert_eq!(back.filtered_reason, None, "shown until the caller filters it again");
        assert_eq!(ids(store.list_queue_leads().unwrap()), [lead.id]);
        assert!(store.restore_lead(lead.id).is_err(), "only a dismissed lead is restored");
        assert!(store.restore_lead(999).is_err());

        // A filtered posting can be restored too.
        store.restore_lead(filtered.id).unwrap();
        assert_eq!(store.count_dismissed_leads().unwrap(), 0);
        assert_eq!(store.list_queue_leads().unwrap().len(), 2);
    }

    #[test]
    fn lead_state_moves_forward_only() {
        let store = Store::open_in_memory().unwrap();
        let lead = store.add_lead(&new("https://x/fwd", "acme-fwd", None)).unwrap();
        store.dismiss_lead(lead.id, "too far").unwrap();
        assert!(store.promote_lead(lead.id, 1).is_err());
        assert!(store.dismiss_lead(lead.id, "again").is_err());
        assert_eq!(store.get_lead(lead.id).unwrap().unwrap().state, LeadState::Dismissed);
    }
}
