//! SQLite store. One `Store` (cheap to clone) shared by the whole app.
//!
//! Each table lives in its own file as `impl Store { .. }`, so a module extends its table without
//! touching the others: `applications`/`events`/`status`, `providers`, `sessions`, `emails`, `kv`
//! (shared settings), `preferences`, `chat` (preference chat), `sources`, `leads` (discovery),
//! `trends` (trend items + reports), `graph` (graph checkpoints), `profile` (the user's documents
//! and the profile read from them), `answers` (remembered application answers) and
//! `source_suggestions` (job sources the "Suggest sources" search found).
//!
//! Schema changes: append a new entry to [`MIGRATIONS`] (never edit an existing one).

mod answers;
mod applications;
mod chat;
mod emails;
mod events;
mod graph;
mod kv;
mod leads;
mod preferences;
mod profile;
mod providers;
mod sessions;
mod source_suggestions;
mod sources;
mod status;
mod trends;

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use anyhow::{Context, Result};
use rusqlite::{Connection, Transaction};

pub use applications::{clean_round, Application, NewApplication, ROUND_MAX_CHARS};
pub use chat::ChatMessage;
pub use emails::Email;
pub use events::Event;
pub use leads::{Lead, LeadState, NewLead};
pub use profile::{sources_hash, NewProfileSource, ProfileSource, StoredProfile};
pub use graph::{
    GraphCheckpointRow, GraphThread, GRAPH_DONE, GRAPH_FAILED, GRAPH_RUNNING, GRAPH_WAITING,
};
pub use providers::{NewProvider, Provider, ProviderKind};
pub use sessions::{NewSession, Session};
pub use source_suggestions::{NewSuggestion, SourceSuggestion, SuggestionState};
pub use sources::{NewSource, Source, SourcePurpose};
pub use status::Status;
pub use trends::{NewReport, NewTrendItem, Report, TrendItem};

/// Ordered migrations; `PRAGMA user_version` holds how many have been applied.
const MIGRATIONS: &[&str] = &[
    // 1: initial schema
    "
    CREATE TABLE applications (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        slug          TEXT NOT NULL UNIQUE,
        company       TEXT NOT NULL DEFAULT '',
        role          TEXT NOT NULL DEFAULT '',
        source        TEXT,
        apply_url     TEXT,
        status        TEXT NOT NULL DEFAULT 'draft',
        dir           TEXT NOT NULL,
        analysis_json TEXT,
        gaps_json     TEXT,
        created_at    TEXT NOT NULL,
        updated_at    TEXT NOT NULL
    );
    CREATE TABLE events (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        app_id  INTEGER NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
        at      TEXT NOT NULL,
        kind    TEXT NOT NULL,
        message TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX events_app ON events(app_id, id);
    CREATE TABLE providers (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        name       TEXT NOT NULL,
        kind       TEXT NOT NULL,
        base_url   TEXT NOT NULL,
        model      TEXT NOT NULL DEFAULT '',
        json_mode  TEXT NOT NULL DEFAULT 'json_schema',
        is_default INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE sessions (
        id               INTEGER PRIMARY KEY AUTOINCREMENT,
        name             TEXT NOT NULL,
        start_url        TEXT NOT NULL,
        domains_json     TEXT NOT NULL DEFAULT '[]',
        status           TEXT NOT NULL DEFAULT 'new',
        last_verified_at TEXT,
        created_at       TEXT NOT NULL
    );
    CREATE TABLE emails (
        id          TEXT PRIMARY KEY,
        app_id      INTEGER REFERENCES applications(id) ON DELETE SET NULL,
        from_addr   TEXT NOT NULL DEFAULT '',
        subject     TEXT NOT NULL DEFAULT '',
        snippet     TEXT NOT NULL DEFAULT '',
        received_at TEXT NOT NULL,
        kind        TEXT,
        confidence  REAL,
        summary     TEXT,
        applied     INTEGER NOT NULL DEFAULT 0
    );
    CREATE INDEX emails_app ON emails(app_id);
    CREATE TABLE kv (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    ",
    // 2: preferences, discovery leads, market trends, per-application insight
    "
    CREATE TABLE preferences (
        id         INTEGER PRIMARY KEY CHECK (id = 1),
        json       TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    CREATE TABLE chat_messages (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        thread  TEXT NOT NULL,
        role    TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
        content TEXT NOT NULL,
        at      TEXT NOT NULL
    );
    CREATE INDEX chat_messages_thread ON chat_messages(thread, id);
    CREATE TABLE sources (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        kind        TEXT NOT NULL,
        name        TEXT NOT NULL DEFAULT '',
        url         TEXT NOT NULL DEFAULT '',
        purpose     TEXT NOT NULL CHECK (purpose IN ('jobs', 'trends')),
        enabled     INTEGER NOT NULL DEFAULT 1,
        session_id  INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
        last_run_at TEXT,
        last_status TEXT
    );
    CREATE TABLE leads (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        source_id      INTEGER REFERENCES sources(id) ON DELETE SET NULL,
        url            TEXT NOT NULL UNIQUE,
        dedup_key      TEXT NOT NULL DEFAULT '',
        company        TEXT NOT NULL DEFAULT '',
        role           TEXT NOT NULL DEFAULT '',
        location       TEXT NOT NULL DEFAULT '',
        jd_path        TEXT,
        score          REAL,
        score_json     TEXT,
        insight_json   TEXT,
        state          TEXT NOT NULL DEFAULT 'new' CHECK (state IN ('new', 'dismissed', 'promoted')),
        dismiss_reason TEXT,
        app_id         INTEGER REFERENCES applications(id) ON DELETE SET NULL,
        found_at       TEXT NOT NULL
    );
    CREATE INDEX leads_dedup ON leads(dedup_key);
    CREATE INDEX leads_state_score ON leads(state, score DESC);
    CREATE TABLE trend_items (
        id           INTEGER PRIMARY KEY AUTOINCREMENT,
        source_id    INTEGER REFERENCES sources(id) ON DELETE SET NULL,
        url          TEXT NOT NULL UNIQUE,
        title        TEXT NOT NULL DEFAULT '',
        published_at TEXT,
        text_path    TEXT,
        fetched_at   TEXT NOT NULL
    );
    CREATE TABLE reports (
        id             INTEGER PRIMARY KEY AUTOINCREMENT,
        kind           TEXT NOT NULL,
        period         TEXT NOT NULL DEFAULT '',
        title          TEXT NOT NULL DEFAULT '',
        body_md        TEXT NOT NULL DEFAULT '',
        citations_json TEXT NOT NULL DEFAULT '[]',
        created_at     TEXT NOT NULL
    );
    CREATE INDEX reports_kind ON reports(kind, id);
    ALTER TABLE applications ADD COLUMN insight_json TEXT;
    ",
    // 3: graph runtime checkpoints (see `graph`); `status`/`detail_json` of the newest row are the
    // thread's status
    "
    CREATE TABLE graph_checkpoints (
        seq         INTEGER PRIMARY KEY AUTOINCREMENT,
        id          TEXT NOT NULL UNIQUE,
        thread      TEXT NOT NULL,
        graph       TEXT NOT NULL,
        step        INTEGER NOT NULL,
        state_json  TEXT NOT NULL,
        rest_json   TEXT NOT NULL DEFAULT '{}',
        status      TEXT NOT NULL DEFAULT 'running',
        detail_json TEXT,
        created_at  TEXT NOT NULL,
        updated_at  TEXT NOT NULL
    );
    CREATE INDEX graph_checkpoints_thread ON graph_checkpoints(thread, seq);
    CREATE INDEX graph_checkpoints_status ON graph_checkpoints(status, updated_at);
    ",
    // 4: the interview round the user marked (Jobs page, `Store::set_interview_round`)
    "
    ALTER TABLE applications ADD COLUMN interview_round TEXT;
    ",
    // 5: the profile comes from the user's documents (`profile_sources`), read into one stored
    // profile (`profile.id = 1`); remembered application answers move out of `profile.yaml`
    "
    CREATE TABLE profile_sources (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        name       TEXT NOT NULL,
        kind       TEXT NOT NULL CHECK (kind IN ('file', 'text')),
        text       TEXT NOT NULL,
        chars      INTEGER NOT NULL,
        truncated  INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL
    );
    CREATE TABLE profile (
        id             INTEGER PRIMARY KEY CHECK (id = 1),
        yaml           TEXT NOT NULL,
        sources_hash   TEXT NOT NULL,
        left_out_json  TEXT NOT NULL DEFAULT '[]',
        built_at       TEXT NOT NULL
    );
    CREATE TABLE application_answers (
        key        TEXT PRIMARY KEY,
        question   TEXT NOT NULL,
        answer     TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    DELETE FROM kv WHERE key IN ('profile_path', 'prefs_cv_name');
    ",
    // 6: a `new` lead the saved preferences filter out is hidden from the queue, not dismissed, so a
    // later change can show it again (`Store::set_lead_filtered`, `discover::refilter`)
    "
    ALTER TABLE leads ADD COLUMN filtered_reason TEXT;
    ",
    // 7: dismissed jobs can be listed and restored: when a lead was dismissed (older rows fall back
    // to `found_at`), and for a dismissed application the status it had, why and when
    "
    ALTER TABLE leads ADD COLUMN dismissed_at TEXT;
    ALTER TABLE applications ADD COLUMN dismissed_from TEXT;
    ALTER TABLE applications ADD COLUMN dismiss_reason TEXT;
    ALTER TABLE applications ADD COLUMN dismissed_at TEXT;
    ",
    // 8: sessions are sites the user is signed in to in their own Chrome; the logins of the old
    // app-owned profiles went with those profiles, so every session starts over
    "
    UPDATE sessions SET status = 'new', domains_json = '[]', last_verified_at = NULL;
    ",
    // 9: job sources the "Suggest sources" search found and checked (`discover::suggest`); an
    // added or dismissed one is never suggested again
    "
    CREATE TABLE source_suggestions (
        id       INTEGER PRIMARY KEY AUTOINCREMENT,
        kind     TEXT NOT NULL,
        input    TEXT NOT NULL,
        name     TEXT NOT NULL DEFAULT '',
        jobs     INTEGER NOT NULL DEFAULT 0,
        matching INTEGER NOT NULL DEFAULT 0,
        query    TEXT NOT NULL DEFAULT '',
        state    TEXT NOT NULL DEFAULT 'new' CHECK (state IN ('new', 'added', 'dismissed')),
        found_at TEXT NOT NULL,
        UNIQUE(kind, input)
    );
    ",
];

#[derive(Clone)]
pub struct Store {
    conn: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open (creating if needed) the database file and run migrations.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
        // Best effort: events are masked on read as well, so a failure here never stops the app.
        match events::scrub_old_events(&mut conn) {
            Ok(0) => {}
            Ok(count) => tracing::info!(count, "masked secrets in old events"),
            Err(error) => tracing::warn!(error = %error, "masking secrets in old events failed"),
        }
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Run `f` with the connection (for table-specific code in other modules).
    pub fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        f(&self.lock())
    }

    /// Run `f` in a transaction; commits if `f` returns `Ok`.
    pub fn with_tx<T>(&self, f: impl FnOnce(&Transaction) -> Result<T>) -> Result<T> {
        let mut guard = self.lock();
        let tx = guard.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let applied: usize = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(applied) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", i + 1)?;
        tx.commit()?;
    }
    Ok(())
}

pub(crate) fn now() -> chrono::DateTime<chrono::Utc> {
    chrono::Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.db")).unwrap();
        (dir, store)
    }

    fn new_app(slug: &str) -> NewApplication {
        NewApplication {
            slug: slug.into(),
            company: "Acme".into(),
            role: "Backend Engineer".into(),
            source: Some("https://acme.test/jobs/1".into()),
            dir: format!("/tmp/{slug}").into(),
            ..Default::default()
        }
    }

    #[test]
    fn application_and_status_event_round_trip() {
        let (_dir, store) = store();
        let app = store.create_application(&new_app("acme-backend")).unwrap();
        assert_eq!(app.status, Status::Draft);

        let got = store.get_application(app.id).unwrap().unwrap();
        assert_eq!(got.slug, "acme-backend");
        assert_eq!(got.company, "Acme");
        assert_eq!(store.get_application_by_slug("acme-backend").unwrap().unwrap().id, app.id);

        store.set_status(app.id, Status::CvReady, "approved").unwrap();
        let got = store.get_application(app.id).unwrap().unwrap();
        assert_eq!(got.status, Status::CvReady);

        let events = store.list_events(app.id).unwrap();
        assert_eq!(events.len(), 2, "created + status change");
        assert_eq!(events[0].kind, "created");
        assert_eq!(events[1].kind, "status");
        assert!(events[1].message.contains("approved"));

        assert_eq!(store.list_applications().unwrap().len(), 1);
        assert!(store.create_application(&new_app("acme-backend")).is_err(), "slug is unique");
        assert_eq!(store.unique_slug("acme-backend").unwrap(), "acme-backend-2");
    }

    #[test]
    fn an_existing_database_is_migrated_and_keeps_its_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            // A database as the first release left it: only migration 1 applied.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(MIGRATIONS[0]).unwrap();
            conn.pragma_update(None, "user_version", 1).unwrap();
            conn.execute(
                "INSERT INTO applications (slug, company, role, status, dir, created_at, updated_at)
                 VALUES ('old-app', 'Acme', 'Dev', 'applied', '/tmp/old-app', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
            conn.execute("INSERT INTO kv (key, value) VALUES ('profile_path', '/x/profile.yaml')", []).unwrap();
            conn.execute(
                "INSERT INTO sessions (name, start_url, domains_json, status, last_verified_at, created_at)
                 VALUES ('LinkedIn', 'https://www.linkedin.com/', '[\"linkedin.com\"]', 'ok', '2026-01-02T00:00:00+00:00', '2026-01-01T00:00:00+00:00')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();
        let version: usize = store.with_conn(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get(0))?)).unwrap();
        assert_eq!(version, MIGRATIONS.len());

        let app = store.get_application_by_slug("old-app").unwrap().unwrap();
        assert_eq!((app.company.as_str(), app.status, app.insight_json.as_deref()), ("Acme", Status::Applied, None));
        assert_eq!(app.interview_round, None);
        // The profile is no longer a file: its old path setting is forgotten.
        assert_eq!(store.kv_get("profile_path").unwrap(), None);
        // The logins of the old app-owned browser profiles are gone: every session starts over.
        let session = &store.list_sessions().unwrap()[0];
        assert_eq!((session.name.as_str(), session.status.as_str(), session.domains_json.as_str()), ("LinkedIn", "new", "[]"));
        assert!(session.last_verified_at.is_none());

        // The new column and tables are usable.
        store.set_insight_json(app.id, Some(r#"{"domain": "payments"}"#)).unwrap();
        assert_eq!(store.get_application(app.id).unwrap().unwrap().insight_json.as_deref(), Some(r#"{"domain": "payments"}"#));
        store.save_preferences(&Default::default()).unwrap();
        store.add_chat_message("prefs", crate::engine::llm::Role::User, "hi").unwrap();
        store.add_source(&NewSource { kind: "lever".into(), ..Default::default() }).unwrap();
        let lead = store.add_lead(&NewLead { url: "https://a.test/1".into(), ..Default::default() }).unwrap();
        assert_eq!(lead.filtered_reason, None);
        assert!(store.set_lead_filtered(lead.id, Some("location")).unwrap());
        store.add_report(&NewReport { kind: "weekly".into(), ..Default::default() }).unwrap();
        store.add_profile_source(&NewProfileSource::text("Note", "I build APIs."), usize::MAX).unwrap();
        store.save_profile("basics: {name: A}\n", "hash", &[]).unwrap();
        store.save_answer("why", "Why us?", "Because.").unwrap();
        store.upsert_suggestion(&NewSuggestion { kind: "lever".into(), input: "jobs.lever.co/acme".into(), ..Default::default() }).unwrap();

        // Reopening applies nothing twice.
        drop(store);
        assert!(Store::open(&path).unwrap().get_application_by_slug("old-app").unwrap().is_some());
    }

    #[test]
    fn status_moves_forward_only() {
        let (_dir, store) = store();
        let app = store.create_application(&new_app("a")).unwrap();
        store.set_status(app.id, Status::Offer, "").unwrap();
        assert!(store.set_status(app.id, Status::Rejected, "").is_err());
        assert!(store.set_status(app.id, Status::Draft, "").is_err());
        store.force_status(app.id, Status::Rejected, "manual").unwrap();
        assert_eq!(store.get_application(app.id).unwrap().unwrap().status, Status::Rejected);

        assert!(Status::Failed.can_advance_to(Status::CvReady));
        assert!(Status::Failed.can_advance_to(Status::Submitting));
        assert!(!Status::Failed.can_advance_to(Status::Applied));
        assert!(Status::Submitting.can_advance_to(Status::Failed));
        assert!(!Status::Applied.can_advance_to(Status::Failed));
        assert_eq!(Status::parse("cv_ready"), Some(Status::CvReady));
    }

    #[test]
    fn kv_and_reopen_keep_data() {
        let (dir, store) = store();
        assert_eq!(store.kv_get("some_setting").unwrap(), None);
        store.kv_set("some_setting", "/x").unwrap();
        store.kv_set("some_setting", "/y").unwrap();
        assert_eq!(store.kv_get("some_setting").unwrap().as_deref(), Some("/y"));
        drop(store);
        let again = Store::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(again.kv_get("some_setting").unwrap().as_deref(), Some("/y"));
        again.kv_delete("some_setting").unwrap();
        assert_eq!(again.kv_get("some_setting").unwrap(), None);
    }

    #[test]
    fn providers_sessions_emails_crud() {
        let (_dir, store) = store();
        let a = store
            .add_provider(&NewProvider {
                name: "OpenAI".into(),
                kind: ProviderKind::OpenAi,
                base_url: "https://api.openai.com/v1".into(),
                ..Default::default()
            })
            .unwrap();
        let b = store
            .add_provider(&NewProvider { name: "Claude".into(), kind: ProviderKind::Anthropic, ..Default::default() })
            .unwrap();
        store.set_default_provider(b.id).unwrap();
        assert_eq!(store.default_provider().unwrap().unwrap().id, b.id);
        store.set_default_provider(a.id).unwrap();
        assert_eq!(store.default_provider().unwrap().unwrap().id, a.id);
        assert_eq!(store.list_providers().unwrap().len(), 2);
        store.delete_provider(a.id).unwrap();
        assert!(store.default_provider().unwrap().is_none());

        let s = store
            .add_session(&NewSession { name: "linkedin".into(), start_url: "https://linkedin.com".into() })
            .unwrap();
        assert_eq!(s.status, "new");
        store.update_session_status(s.id, "ok", Some(now())).unwrap();
        assert!(store.get_session(s.id).unwrap().unwrap().last_verified_at.is_some());

        let app = store.create_application(&new_app("m")).unwrap();
        let mut email = Email {
            id: "gmail-1".into(),
            app_id: None,
            from_addr: "jobs@acme.test".into(),
            subject: "Thanks for applying".into(),
            snippet: "We received".into(),
            received_at: now(),
            kind: None,
            confidence: None,
            summary: None,
            applied: false,
        };
        store.upsert_email(&email).unwrap();
        email.app_id = Some(app.id);
        email.kind = Some("confirmation".into());
        email.confidence = Some(0.9);
        store.upsert_email(&email).unwrap();
        let got = store.get_email("gmail-1").unwrap().unwrap();
        assert_eq!(got.app_id, Some(app.id));
        assert_eq!(store.list_emails(Some(app.id)).unwrap().len(), 1);
        assert_eq!(store.list_emails(None).unwrap().len(), 1);
    }
}
