use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{events, now, Status, Store};

const COLS: &str =
    "id, slug, company, role, source, apply_url, status, dir, analysis_json, gaps_json, created_at, updated_at, insight_json";

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
    /// JD analysis as JSON text (P2).
    pub analysis_json: Option<String>,
    /// CV gaps as JSON text (P2).
    pub gaps_json: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Job insight (domain, company, skills, interview process) as JSON text (P11).
    pub insight_json: Option<String>,
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

impl Store {
    /// Insert a `Draft` application and a `created` event. Fails if the slug exists.
    pub fn create_application(&self, new: &NewApplication) -> Result<Application> {
        let id = self.with_tx(|tx| {
            let at = now();
            tx.execute(
                "INSERT INTO applications (slug, company, role, source, apply_url, status, dir, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    new.slug,
                    new.company,
                    new.role,
                    new.source,
                    new.apply_url,
                    Status::Draft,
                    new.dir.to_string_lossy(),
                    at
                ],
            )
            .with_context(|| format!("creating application `{}`", new.slug))?;
            let id = tx.last_insert_rowid();
            events::insert_event(tx, id, "created", "application created")?;
            Ok(id)
        })?;
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
