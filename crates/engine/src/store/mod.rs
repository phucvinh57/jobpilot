//! SQLite store. One `Store` (cheap to clone) shared by the whole app.
//!
//! Each table lives in its own file as `impl Store { .. }`, so a module extends its table without
//! touching the others: `applications`/`events`/`status`, `providers`, `sessions`, `emails`, `kv`
//! (shared settings), `preferences`, `chat` (preference chat), `sources`, `leads` (discovery),
//! `trends` (trend items + reports), `graph` (graph checkpoints), `profile` (the user's documents
//! and the profile read from them), `answers` (remembered application answers) and
//! `source_suggestions` (job sources the "Suggest sources" search found).
//!
//! Schema changes: append a new entry to [`MIGRATIONS`] (never edit an existing one, nor [`SCHEMA`]).

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
pub use providers::{NewProvider, Protocol, Provider, ProviderKind};
pub use sessions::{NewSession, Session};
pub use source_suggestions::{NewSuggestion, SourceSuggestion, SuggestionState};
pub use sources::{NewSource, Source, SourcePurpose};
pub use status::Status;
pub use trends::{NewReport, NewTrendItem, Report, TrendItem};

/// The schema version the squashed [`SCHEMA`] stands for: a database at an older version was made
/// by a release whose migrations are gone, and is refused.
const BASELINE: usize = 10;

/// The whole schema as of [`BASELINE`] (migrations 1 to 10 of earlier releases, squashed).
const SCHEMA: &str = "
    CREATE TABLE applications (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        slug            TEXT NOT NULL UNIQUE,
        company         TEXT NOT NULL DEFAULT '',
        role            TEXT NOT NULL DEFAULT '',
        source          TEXT,
        apply_url       TEXT,
        status          TEXT NOT NULL DEFAULT 'draft',
        dir             TEXT NOT NULL,
        analysis_json   TEXT,
        gaps_json       TEXT,
        created_at      TEXT NOT NULL,
        updated_at      TEXT NOT NULL,
        insight_json    TEXT,
        interview_round TEXT,
        dismissed_from  TEXT,
        dismiss_reason  TEXT,
        dismissed_at    TEXT
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
    -- A `new` lead the saved preferences filter out is hidden (`filtered_reason`), not dismissed.
    CREATE TABLE leads (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        source_id       INTEGER REFERENCES sources(id) ON DELETE SET NULL,
        url             TEXT NOT NULL UNIQUE,
        dedup_key       TEXT NOT NULL DEFAULT '',
        company         TEXT NOT NULL DEFAULT '',
        role            TEXT NOT NULL DEFAULT '',
        location        TEXT NOT NULL DEFAULT '',
        jd_path         TEXT,
        score           REAL,
        score_json      TEXT,
        insight_json    TEXT,
        state           TEXT NOT NULL DEFAULT 'new' CHECK (state IN ('new', 'dismissed', 'promoted')),
        dismiss_reason  TEXT,
        app_id          INTEGER REFERENCES applications(id) ON DELETE SET NULL,
        found_at        TEXT NOT NULL,
        filtered_reason TEXT,
        dismissed_at    TEXT
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
    -- One report per period: a rerun replaces it.
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
    CREATE UNIQUE INDEX reports_period ON reports(kind, period) WHERE period != '';
    -- Graph runtime checkpoints (`graph`): `status`/`detail_json` of the newest row are the thread's.
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
    -- The user's documents and the one profile read from them (`profile.id = 1`).
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
        id            INTEGER PRIMARY KEY CHECK (id = 1),
        yaml          TEXT NOT NULL,
        sources_hash  TEXT NOT NULL,
        left_out_json TEXT NOT NULL DEFAULT '[]',
        built_at      TEXT NOT NULL
    );
    CREATE TABLE application_answers (
        key        TEXT PRIMARY KEY,
        question   TEXT NOT NULL,
        answer     TEXT NOT NULL,
        updated_at TEXT NOT NULL
    );
    -- Job sources the \"Suggest sources\" search found; an added or dismissed one is never suggested again.
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
";

/// Migrations after [`BASELINE`], in order: entry `i` takes the database to version
/// `BASELINE + i + 1`. `PRAGMA user_version` holds the version.
const MIGRATIONS: &[&str] = &[
    // 11: provider kinds split into official and compatible: an `openai` / `anthropic` row stays
    // official only when it points at exactly the official API (an empty base URL meant the
    // official default); any other address, look-alike hosts included, is a compatible endpoint
    "
    UPDATE providers SET kind = 'openai_compatible'
        WHERE kind = 'openai' AND base_url NOT IN
            ('', 'https://api.openai.com', 'https://api.openai.com/', 'https://api.openai.com/v1', 'https://api.openai.com/v1/');
    UPDATE providers SET kind = 'anthropic_compatible'
        WHERE kind = 'anthropic' AND base_url NOT IN
            ('', 'https://api.anthropic.com', 'https://api.anthropic.com/', 'https://api.anthropic.com/v1', 'https://api.anthropic.com/v1/');
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
        Self::init(conn).with_context(|| format!("opening {}", path.display()))
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut conn)?;
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
    let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let mut version =
        usize::try_from(version).with_context(|| format!("the database schema version {version} is invalid"))?;
    if version == 0 {
        let tx = conn.transaction()?;
        tx.execute_batch(SCHEMA).context("creating the schema")?;
        tx.pragma_update(None, "user_version", BASELINE as i64)?;
        tx.commit()?;
        version = BASELINE;
    }
    anyhow::ensure!(
        version >= BASELINE,
        "the database (schema version {version}) is too old for this release: delete it to start over"
    );
    let latest = BASELINE + MIGRATIONS.len();
    anyhow::ensure!(
        version <= latest,
        "the database (schema version {version}) was made by a newer release (this one knows {latest})"
    );
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version - BASELINE) {
        let to = BASELINE + i + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql).with_context(|| format!("migration {to}"))?;
        tx.pragma_update(None, "user_version", to as i64)?;
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
    fn a_new_database_is_at_the_latest_version_and_reopens() {
        let (dir, store) = store();
        let version = |s: &Store| -> usize {
            s.with_conn(|c| Ok(c.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?)).unwrap() as usize
        };
        assert_eq!(version(&store), BASELINE + MIGRATIONS.len());
        store.kv_set("keep", "me").unwrap();
        drop(store);
        let again = Store::open(&dir.path().join("t.db")).unwrap();
        assert_eq!(version(&again), BASELINE + MIGRATIONS.len());
        assert_eq!(again.kv_get("keep").unwrap().as_deref(), Some("me"));
    }

    #[test]
    fn a_database_older_than_the_baseline_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", 3).unwrap();
        let error = Store::open(&path).err().unwrap();
        assert!(format!("{error:#}").contains("too old"), "{error:#}");
        assert!(format!("{error:#}").contains("old.db"), "names the file: {error:#}");
    }

    #[test]
    fn a_database_at_the_baseline_is_migrated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("v10.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute_batch(
            "INSERT INTO providers (name, kind, base_url) VALUES
                ('official', 'openai', 'https://api.openai.com/v1'),
                ('local', 'openai', 'http://localhost:11434/v1'),
                ('proxy', 'anthropic', 'https://api.anthropic.com.evil.test');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", BASELINE as i64).unwrap();
        drop(conn);

        let store = Store::open(&path).unwrap();
        let kinds = store
            .with_conn(|c| {
                let mut stmt = c.prepare("SELECT kind FROM providers ORDER BY id")?;
                let rows = stmt.query_map([], |r| r.get(0))?;
                Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
            })
            .unwrap();
        assert_eq!(kinds, ["openai", "openai_compatible", "anthropic_compatible"]);
    }

    #[test]
    fn a_database_from_a_newer_release_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("new.db");
        Connection::open(&path).unwrap().pragma_update(None, "user_version", (BASELINE + MIGRATIONS.len() + 1) as i64).unwrap();
        let error = Store::open(&path).err().unwrap();
        assert!(format!("{error:#}").contains("newer release"), "{error:#}");
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
                name: "Local".into(),
                kind: ProviderKind::OpenAiCompatible,
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
