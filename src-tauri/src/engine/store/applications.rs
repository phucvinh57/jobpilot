use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{events, now, Status, Store};

const COLS: &str =
    "id, slug, company, role, source, apply_url, status, dir, analysis_json, gaps_json, created_at, updated_at, insight_json, interview_round";

#[derive(Clone, Debug)]
pub struct Application {
    pub id: i64,
    /// kebab-case `company-role`, unique; also the folder name under `applications/`.
    pub slug: String,
    pub company: String,
    pub role: String,
    /// JD URL or file path the user gave.
    pub source: Option<String>,
    pub apply_url: Option<String>,
    pub status: Status,
    /// `applications/<slug>/` (jd.md, cv.yaml, cv.pdf, page-*.png).
    pub dir: PathBuf,
    /// JD analysis as JSON text.
    pub analysis_json: Option<String>,
    /// CV gaps as JSON text.
    pub gaps_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Job insight (domain, company, skills, interview process) as JSON text.
    pub insight_json: Option<String>,
    /// The interview round the user marked while the status is `Interview` (`Technical interview`).
    pub interview_round: Option<String>,
}

impl Application {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: r.get(0)?,
            slug: r.get(1)?,
            company: r.get(2)?,
            role: r.get(3)?,
            source: r.get(4)?,
            apply_url: r.get(5)?,
            status: r.get(6)?,
            dir: PathBuf::from(r.get::<_, String>(7)?),
            analysis_json: r.get(8)?,
            gaps_json: r.get(9)?,
            created_at: r.get(10)?,
            updated_at: r.get(11)?,
            insight_json: r.get(12)?,
            interview_round: r.get(13)?,
        })
    }

    pub fn jd_path(&self) -> PathBuf {
        self.dir.join("jd.md")
    }
    pub fn cv_yaml_path(&self) -> PathBuf {
        self.dir.join("cv.yaml")
    }
    pub fn cv_pdf_path(&self) -> PathBuf {
        self.dir.join("cv.pdf")
    }
}

/// Longest interview round stored, in characters.
pub const ROUND_MAX_CHARS: usize = 60;

/// A round as stored: trimmed, at most [`ROUND_MAX_CHARS`] characters; blank is no round.
pub fn clean_round(round: Option<&str>) -> Option<String> {
    let round = round?.trim();
    let cut: String = round.chars().take(ROUND_MAX_CHARS).collect();
    let cut = cut.trim_end();
    (!cut.is_empty()).then(|| cut.to_string())
}

/// Fields needed to create a `Draft` application.
#[derive(Clone, Debug, Default)]
pub struct NewApplication {
    pub slug: String,
    pub company: String,
    pub role: String,
    pub source: Option<String>,
    pub apply_url: Option<String>,
    pub dir: PathBuf,
}

/// The row and its `created` event, inside the caller's transaction.
pub(super) fn insert_application(tx: &rusqlite::Transaction, new: &NewApplication) -> Result<i64> {
    let at = now();
    tx.execute(
        "INSERT INTO applications (slug, company, role, source, apply_url, status, dir, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![new.slug, new.company, new.role, new.source, new.apply_url, Status::Draft, new.dir.to_string_lossy(), at],
    )
    .with_context(|| format!("creating application `{}`", new.slug))?;
    let id = tx.last_insert_rowid();
    events::insert_event(tx, id, "created", "application created")?;
    Ok(id)
}

impl Store {
    /// Insert a `Draft` application and a `created` event. Fails if the slug exists.
    pub fn create_application(&self, new: &NewApplication) -> Result<Application> {
        let id = self.with_tx(|tx| insert_application(tx, new))?;
        self.get_application(id)?.context("application vanished after insert")
    }

    pub fn get_application(&self, id: i64) -> Result<Option<Application>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM applications WHERE id = ?1"), [id], Application::from_row)
                .optional()?)
        })
    }

    pub fn get_application_by_slug(&self, slug: &str) -> Result<Option<Application>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM applications WHERE slug = ?1"), [slug], Application::from_row)
                .optional()?)
        })
    }

    /// Most recently updated first.
    pub fn list_applications(&self) -> Result<Vec<Application>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLS} FROM applications ORDER BY updated_at DESC, id DESC"))?;
            let rows = stmt.query_map([], Application::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Save everything except `slug`, `status` (use `set_status`), `insight_json` (use `set_insight_json`)
    /// and timestamps; bumps `updated_at`.
    pub fn save_application(&self, app: &Application) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE applications SET company = ?1, role = ?2, source = ?3, apply_url = ?4, dir = ?5,
                        analysis_json = ?6, gaps_json = ?7, updated_at = ?8 WHERE id = ?9",
                params![
                    app.company,
                    app.role,
                    app.source,
                    app.apply_url,
                    app.dir.to_string_lossy(),
                    app.analysis_json,
                    app.gaps_json,
                    now(),
                    app.id
                ],
            )?;
            anyhow::ensure!(n == 1, "application {} not found", app.id);
            Ok(())
        })
    }

    /// Store (or clear) the job insight JSON; bumps `updated_at`.
    pub fn set_insight_json(&self, id: i64, insight_json: Option<&str>) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE applications SET insight_json = ?1, updated_at = ?2 WHERE id = ?3",
                params![insight_json, now(), id],
            )?;
            anyhow::ensure!(n == 1, "application {id} not found");
            Ok(())
        })
    }

    /// Mark (or clear) the interview round the application is in, e.g. `Technical interview`.
    /// The text is trimmed and cut to [`ROUND_MAX_CHARS`]; an empty one clears the round. Only an
    /// application in `Interview` has a round. A change bumps `updated_at` and appends a `round`
    /// event (`Round: Technical interview` / `Round cleared`); setting the same round again is a no-op.
    pub fn set_interview_round(&self, id: i64, round: Option<&str>) -> Result<()> {
        let round = clean_round(round);
        self.with_tx(|tx| {
            let (status, current): (Status, Option<String>) = tx
                .query_row("SELECT status, interview_round FROM applications WHERE id = ?1", [id], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?
                .with_context(|| format!("application {id} not found"))?;
            anyhow::ensure!(
                status == Status::Interview,
                "only an application in Interview has a round (application {id} is {})",
                status.label()
            );
            if current == round {
                return Ok(());
            }
            tx.execute(
                "UPDATE applications SET interview_round = ?1, updated_at = ?2 WHERE id = ?3",
                params![round, now(), id],
            )?;
            let text = match &round {
                Some(r) => format!("Round: {r}"),
                None => "Round cleared".to_string(),
            };
            events::insert_event(tx, id, "round", &text)?;
            Ok(())
        })
    }

    /// Deletes the row and its events (files on disk are the caller's job).
    pub fn delete_application(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM applications WHERE id = ?1", [id])?;
            Ok(())
        })
    }

    /// `base`, or `base-2`, `base-3`, ... whichever is not taken yet.
    pub fn unique_slug(&self, base: &str) -> Result<String> {
        let mut candidate = base.to_string();
        let mut n = 1;
        while self.get_application_by_slug(&candidate)?.is_some() {
            n += 1;
            candidate = format!("{base}-{n}");
        }
        Ok(candidate)
    }
}

#[cfg(test)]
mod tests {
    use super::{clean_round, ROUND_MAX_CHARS};
    use crate::engine::store::{NewApplication, Status, Store};

    fn store_with_app() -> (tempfile::TempDir, Store, i64) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        let app = store
            .create_application(&NewApplication {
                slug: "acme".into(),
                company: "Acme".into(),
                role: "Engineer".into(),
                dir: "/tmp/acme".into(),
                ..Default::default()
            })
            .unwrap();
        (dir, store, app.id)
    }

    #[test]
    fn a_round_is_trimmed_capped_and_blank_means_none() {
        assert_eq!(clean_round(Some("  Technical interview ")).as_deref(), Some("Technical interview"));
        assert_eq!(clean_round(Some("   ")), None);
        assert_eq!(clean_round(None), None);
        let long = "x".repeat(ROUND_MAX_CHARS + 10);
        assert_eq!(clean_round(Some(&long)).map(|r| r.chars().count()), Some(ROUND_MAX_CHARS));
        // Multi-byte text is cut on characters, not bytes.
        let wide = "\u{e9}".repeat(ROUND_MAX_CHARS + 1);
        assert_eq!(clean_round(Some(&wide)).map(|r| r.chars().count()), Some(ROUND_MAX_CHARS));
    }

    #[test]
    fn the_round_is_stored_with_a_timeline_event_only_in_interview() {
        let (_dir, store, id) = store_with_app();
        assert!(store.get_application(id).unwrap().unwrap().interview_round.is_none());

        let err = store.set_interview_round(id, Some("HR screen")).unwrap_err();
        assert!(format!("{err:#}").contains("Interview"), "{err:#}");
        assert!(store.set_interview_round(999, Some("HR screen")).is_err());

        store.set_status(id, Status::Applied, "").unwrap();
        store.set_status(id, Status::Interview, "").unwrap();
        store.set_interview_round(id, Some("  Technical interview ")).unwrap();
        let app = store.get_application(id).unwrap().unwrap();
        assert_eq!(app.interview_round.as_deref(), Some("Technical interview"));
        let last = store.list_events(id).unwrap().pop().unwrap();
        assert_eq!((last.kind.as_str(), last.message.as_str()), ("round", "Round: Technical interview"));

        // The same round again records nothing.
        let before = store.list_events(id).unwrap().len();
        store.set_interview_round(id, Some("Technical interview")).unwrap();
        assert_eq!(store.list_events(id).unwrap().len(), before);

        store.set_interview_round(id, Some(" ")).unwrap();
        assert_eq!(store.get_application(id).unwrap().unwrap().interview_round, None);
        let last = store.list_events(id).unwrap().pop().unwrap();
        assert_eq!((last.kind.as_str(), last.message.as_str()), ("round", "Round cleared"));

        // The round is kept when the application moves on, but cannot be changed any more.
        store.set_interview_round(id, Some("Final interview")).unwrap();
        store.set_status(id, Status::Offer, "").unwrap();
        assert_eq!(store.get_application(id).unwrap().unwrap().interview_round.as_deref(), Some("Final interview"));
        assert!(store.set_interview_round(id, None).is_err());
    }
}
