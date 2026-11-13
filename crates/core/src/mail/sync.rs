//! Sync Gmail into the `emails` table, match each new message to an application, and apply the
//! result (status change / timeline event / review request).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::account::{KV_HISTORY_ID, KV_LAST_REPORT, KV_LAST_SYNC};
use super::classify::{self, MailKind};
use super::gmail::{self, GmailClient, MailMessage};
use crate::llm::{Llm, LlmFuture};
use crate::notify::desktop_notify;
use crate::store::{Application, Email, Status};
use crate::{runtime, Core};

/// At or above this confidence a classified email changes the status by itself; below it the
/// timeline gets a "needs review" entry instead.
pub const AUTO_APPLY_CONFIDENCE: f64 = 0.8;

/// Messages fetched in parallel.
const FETCH_CONCURRENCY: usize = 6;
/// Applications offered to the model when no deterministic match exists.
const MAX_LLM_CANDIDATES: usize = 40;

/// One application whose status an email moved.
#[derive(Clone, Debug)]
pub struct StatusChange {
    pub app_id: i64,
    pub company: String,
    pub role: String,
    pub from: Status,
    pub to: Status,
    pub subject: String,
}

#[derive(Clone, Debug, Default)]
pub struct SyncReport {
    /// New messages stored by this run.
    pub fetched: usize,
    /// Of those, how many were linked to an application.
    pub matched: usize,
    pub status_changes: Vec<StatusChange>,
    /// Emails left for the user to accept or dismiss.
    pub needs_review: usize,
    /// Problems that did not stop the run (the affected mail is retried next time).
    pub errors: Vec<String>,
    /// The run listed the last 60 days instead of continuing from the stored history id.
    pub full: bool,
}

impl SyncReport {
    pub fn summary(&self) -> ReportSummary {
        ReportSummary {
            fetched: self.fetched,
            updated: self.status_changes.len(),
            needs_review: self.needs_review,
            error: self.errors.first().cloned(),
        }
    }

    fn touched_data(&self) -> bool {
        self.fetched > 0 || !self.status_changes.is_empty()
    }
}

/// What the settings screen shows about the last sync (stored in kv as JSON).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportSummary {
    pub fetched: usize,
    pub updated: usize,
    pub needs_review: usize,
    #[serde(default)]
    pub error: Option<String>,
}

// ---------------------------------------------------------------- change channel

fn change_tx() -> &'static watch::Sender<u64> {
    static TX: OnceLock<watch::Sender<u64>> = OnceLock::new();
    TX.get_or_init(|| watch::channel(0).0)
}

/// Fires (`rx.changed().await`) whenever a sync stored mail or changed a status, so the UI can
/// reload. Works from any executor.
pub fn changes() -> watch::Receiver<u64> {
    change_tx().subscribe()
}

// ---------------------------------------------------------------- entry points

/// Only one sync at a time (poller and "Sync now" must not process the same mail twice).
static SYNC_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Fetch new mail, match and apply it. See the module docs for the rules.
pub async fn sync_once(core: &Core) -> Result<SyncReport> {
    let _one_at_a_time = SYNC_LOCK.lock().await;
    let result = run_sync(core).await;
    match &result {
        Ok(report) => {
            tracing::info!(
                "gmail: synced {} new message(s), {} matched, {} status change(s), {} need review{}",
                report.fetched,
                report.matched,
                report.status_changes.len(),
                report.needs_review,
                if report.full { " (full listing)" } else { "" }
            );
            let _ = core.store.kv_set(KV_LAST_SYNC, &chrono::Utc::now().to_rfc3339());
            save_summary(core, &report.summary());
            if report.touched_data() {
                change_tx().send_modify(|n| *n += 1);
            }
        }
        Err(e) => {
            tracing::warn!("gmail: sync failed: {e:#}");
            save_summary(core, &ReportSummary { error: Some(format!("{e:#}")), ..Default::default() })
        }
    }
    result
}

fn save_summary(core: &Core, summary: &ReportSummary) {
    if let Ok(json) = serde_json::to_string(summary) {
        let _ = core.store.kv_set(KV_LAST_REPORT, &json);
    }
}

/// Run [`sync_once`] now and then every `every` while the app lives (not connected: skipped).
/// `on_change` gets each report that stored mail or changed a status. Abort the handle to stop.
pub fn start_poller<F>(core: Arc<Core>, every: Duration, on_change: F) -> tokio::task::JoinHandle<()>
where
    F: Fn(SyncReport) + Send + Sync + 'static,
{
    runtime::handle().spawn(async move {
        loop {
            if core.gmail_connected() {
                match sync_once(&core).await {
                    Ok(report) if report.touched_data() => on_change(report),
                    Ok(_) => {}
                    Err(_) => {} // already logged by sync_once
                }
            }
            tokio::time::sleep(every).await;
        }
    })
}

static POLLER: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>> = std::sync::Mutex::new(None);

/// App start (and after connecting): begin polling when Gmail is connected, interval from kv
/// `gmail_poll_minutes`. Does nothing if a poller already runs. Returns whether one was started.
/// The UI learns about changes through [`changes`], so no callback is needed here.
pub fn autostart(core: &Arc<Core>) -> bool {
    if !core.gmail_connected() {
        return false;
    }
    let mut slot = POLLER.lock().unwrap_or_else(|e| e.into_inner());
    if slot.as_ref().is_some_and(|h| !h.is_finished()) {
        return false;
    }
    *slot = Some(start_poller(core.clone(), core.gmail_poll_interval(), |_| {}));
    true
}

/// Stands in for the model when none is configured; fails only if a classification is needed.
struct Unavailable(String);

impl Llm for Unavailable {
    fn complete_json<'a>(&'a self, _: &'a str, _: &'a str, _: &'a serde_json::Value) -> LlmFuture<'a, serde_json::Value> {
        Box::pin(std::future::ready(Err(anyhow!("{}", self.0))))
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Err(anyhow!("{}", self.0))))
    }
}

async fn run_sync(core: &Core) -> Result<SyncReport> {
    anyhow::ensure!(core.gmail_connected(), "Gmail is not connected");
    let client = GmailClient::new(&core.gmail_access_token().await?);

    let (ids, cursor, full) = match list_new_ids(core, &client).await {
        Ok(found) => found,
        Err(e) => {
            if gmail::is_unauthorized(&e) {
                core.gmail_forget_access_token();
            }
            return Err(e);
        }
    };

    let mut report = SyncReport { full, ..Default::default() };
    let unseen: Vec<String> = ids.into_iter().filter(|id| !matches!(core.store.get_email(id), Ok(Some(_)))).collect();
    let fetched: Vec<Result<MailMessage>> = futures::stream::iter(unseen)
        .map(|id| {
            let client = &client;
            async move { client.get_message(&id).await }
        })
        .buffer_unordered(FETCH_CONCURRENCY)
        .collect()
        .await;
    let mut messages = Vec::new();
    for result in fetched {
        match result {
            Ok(m) => messages.push(m),
            // Deleted between listing and fetching.
            Err(e) if gmail::is_not_found(&e) => {}
            Err(e) => push_error(&mut report, format!("{e:#}")),
        }
    }
    // Oldest first: a confirmation must be applied before the rejection that follows it.
    messages.sort_by_key(|m| m.date);

    let llm: Box<dyn Llm> = core.default_llm().unwrap_or_else(|e| Box::new(Unavailable(format!("{e:#}"))));
    process_messages(core, llm.as_ref(), messages, &desktop_notify, &mut report).await;

    // Move the cursor only when everything was handled; otherwise the next run retries.
    if report.errors.is_empty() {
        if let Some(cursor) = cursor {
            core.store.kv_set(KV_HISTORY_ID, &cursor)?;
        }
    }
    Ok(report)
}

/// `(message ids, history id to store afterwards, was a full listing)`.
async fn list_new_ids(core: &Core, client: &GmailClient) -> Result<(Vec<String>, Option<String>, bool)> {
    let stored = core.store.kv_get(KV_HISTORY_ID)?.filter(|h| !h.is_empty());
    if let Some(start) = stored {
        match client.history_added(&start).await {
            Ok(delta) => return Ok((delta.message_ids, Some(delta.history_id.unwrap_or(start)), false)),
            // The stored history id expired (Gmail keeps about a week): start over.
            Err(e) if gmail::is_not_found(&e) => tracing::info!("Gmail history id expired; listing recent mail"),
            Err(e) => return Err(e),
        }
    }
    // Take the cursor BEFORE listing so mail arriving meanwhile is seen by the next run.
    let profile = client.profile().await?;
    let ids = client.list_messages(gmail::FIRST_SYNC_QUERY, gmail::FIRST_SYNC_CAP).await?;
    Ok((ids, Some(profile.history_id).filter(|h| !h.is_empty()), true))
}

fn push_error(report: &mut SyncReport, message: String) {
    if !report.errors.contains(&message) {
        report.errors.push(message);
    }
}

// ---------------------------------------------------------------- matching + applying

/// Applications an email can belong to: submitted or later (not drafts, not failed).
fn in_pool(status: Status) -> bool {
    status.rank() >= Status::Submitting.rank()
}

/// Match, classify and apply each message (oldest first). Failures are collected in
/// `report.errors`; a failed message is not stored, so a later run retries it.
pub(crate) async fn process_messages(
    core: &Core,
    llm: &dyn Llm,
    messages: Vec<MailMessage>,
    notify: &(dyn Fn(&str, &str) + Sync),
    report: &mut SyncReport,
) {
    for msg in messages {
        if let Err(e) = process_one(core, llm, &msg, notify, report).await {
            push_error(report, format!("{e:#}"));
        }
    }
}

async fn process_one(
    core: &Core,
    llm: &dyn Llm,
    msg: &MailMessage,
    notify: &(dyn Fn(&str, &str) + Sync),
    report: &mut SyncReport,
) -> Result<()> {
    if core.store.get_email(&msg.id)?.is_some() {
        return Ok(());
    }
    let pool: Vec<Application> = core.store.list_applications()?.into_iter().filter(|a| in_pool(a.status)).collect();
    let mut email = Email {
        id: msg.id.clone(),
        app_id: None,
        from_addr: msg.from.clone(),
        subject: msg.subject.clone(),
        snippet: msg.snippet.clone(),
        received_at: msg.date,
        kind: None,
        confidence: None,
        summary: None,
        applied: false,
    };

    let mut candidates: Vec<&Application> =
        classify::deterministic_candidates(&msg.from, &msg.subject, &pool).into_iter().map(|(a, _)| a).collect();
    if candidates.is_empty() && classify::looks_job_related(&msg.subject, &msg.snippet) {
        candidates = pool.iter().take(MAX_LLM_CANDIDATES).collect();
    }
    if candidates.is_empty() {
        core.store.upsert_email(&email)?;
        report.fetched += 1;
        return Ok(());
    }

    let cls = classify::classify(llm, &candidates, msg).await?;
    // A generic "other" mail is only linked when the model is sure.
    let linked = cls.application_id.filter(|_| cls.kind != MailKind::Other || cls.confidence >= AUTO_APPLY_CONFIDENCE);
    email.app_id = linked;
    email.kind = Some(cls.kind.as_str().to_string());
    email.confidence = Some(cls.confidence);
    email.summary = Some(cls.summary.clone());

    if let Some(app) = linked.and_then(|id| candidates.iter().find(|a| a.id == id)) {
        report.matched += 1;
        if cls.kind != MailKind::Other {
            if cls.confidence >= AUTO_APPLY_CONFIDENCE {
                let change = record_outcome(core, &email, app, cls.kind)?;
                email.applied = change.is_some();
                if let Some(change) = change {
                    notify(
                        &format!("{}: {}", change.to.label(), change.company),
                        &format!("{} - {}", change.role, change.subject),
                    );
                    report.status_changes.push(change);
                }
            } else {
                core.store.add_event(app.id, "email_review", &review_message(&email, cls.kind))?;
                report.needs_review += 1;
            }
        }
    }
    core.store.upsert_email(&email)?;
    report.fetched += 1;
    Ok(())
}

fn review_message(email: &Email, kind: MailKind) -> String {
    let pct = (email.confidence.unwrap_or(0.0) * 100.0).round() as i64;
    format!(
        "needs review: {} \u{2014} looks like {} ({pct}%): {}",
        email.subject,
        kind.as_str(),
        email.summary.as_deref().unwrap_or("")
    )
}

/// Move the application forward if the rule allows and add the `email` timeline event.
/// `app` must be the current row. Returns the status change, if any.
fn record_outcome(core: &Core, email: &Email, app: &Application, kind: MailKind) -> Result<Option<StatusChange>> {
    let summary = email.summary.as_deref().unwrap_or("");
    let mut text = format!("email: {} \u{2014} {summary}", email.subject);
    let mut change = None;
    if let Some(target) = kind.target_status() {
        if app.status == target {
            // Already there (e.g. a second confirmation).
        } else if app.status.can_advance_to(target) {
            core.store.set_status(app.id, target, summary)?;
            change = Some(StatusChange {
                app_id: app.id,
                company: app.company.clone(),
                role: app.role.clone(),
                from: app.status,
                to: target,
                subject: email.subject.clone(),
            });
        } else {
            // Forward-only: never move back (e.g. Offer -> Rejected); the timeline still shows it.
            text.push_str(&format!(" (status stays {})", app.status.label()));
        }
    }
    core.store.add_event(app.id, "email", &text)?;
    Ok(change)
}

// ---------------------------------------------------------------- review

/// A classified email below the auto-apply confidence that the user has not resolved yet.
pub fn needs_review(email: &Email) -> bool {
    email.app_id.is_some()
        && !email.applied
        && email.confidence.is_some_and(|c| c < AUTO_APPLY_CONFIDENCE)
        && email.kind.as_deref().and_then(MailKind::parse).is_some_and(|k| k != MailKind::Other)
}

impl Core {
    /// Emails waiting for a one-click accept/dismiss, newest first (`Some(app_id)`: one application).
    pub fn emails_needing_review(&self, app_id: Option<i64>) -> Result<Vec<Email>> {
        Ok(self.store.list_emails(app_id)?.into_iter().filter(needs_review).collect())
    }

    pub fn accept_email(&self, email_id: &str) -> Result<Option<StatusChange>> {
        accept_email(self, email_id)
    }

    pub fn dismiss_email(&self, email_id: &str) -> Result<()> {
        dismiss_email(self, email_id)
    }
}

/// The user agrees with the classification of a "needs review" email: apply it like a confident
/// match (forward-only) and send the notification. Returns the status change, if any.
pub fn accept_email(core: &Core, email_id: &str) -> Result<Option<StatusChange>> {
    let mut email = core.store.get_email(email_id)?.with_context(|| format!("email {email_id} not found"))?;
    let app_id = email.app_id.context("this email is not linked to an application")?;
    let kind = email
        .kind
        .as_deref()
        .and_then(MailKind::parse)
        .filter(|k| *k != MailKind::Other)
        .context("this email has nothing to apply")?;
    let app = core.store.get_application(app_id)?.with_context(|| format!("application {app_id} no longer exists"))?;

    let change = record_outcome(core, &email, &app, kind)?;
    // Confirmed by the user: no longer "low confidence".
    email.confidence = Some(1.0);
    email.applied = change.is_some();
    core.store.upsert_email(&email)?;
    if let Some(c) = &change {
        desktop_notify(&format!("{}: {}", c.to.label(), c.company), &format!("{} - {}", c.role, c.subject));
    }
    Ok(change)
}

/// The user says the suggestion is wrong: the email stays linked but counts as `other`.
pub fn dismiss_email(core: &Core, email_id: &str) -> Result<()> {
    let mut email = core.store.get_email(email_id)?.with_context(|| format!("email {email_id} not found"))?;
    email.kind = Some(MailKind::Other.as_str().to_string());
    email.applied = false;
    core.store.upsert_email(&email)?;
    if let Some(app_id) = email.app_id {
        core.store.add_event(app_id, "email_review", &format!("review dismissed: {}", email.subject))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;
    use crate::llm::fake::FakeLlm;
    use crate::store::NewApplication;
    use crate::{Paths, Secrets};

    struct Fixture {
        _dir: tempfile::TempDir,
        core: Core,
        notes: Mutex<Vec<(String, String)>>,
    }

    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
            Self { _dir: dir, core, notes: Mutex::default() }
        }

        /// Application at `status` whose company domain is `<slug>.com`.
        fn app(&self, slug: &str, company: &str, status: Status) -> i64 {
            let app = self
                .core
                .store
                .create_application(&NewApplication {
                    slug: slug.into(),
                    company: company.into(),
                    role: "Backend Engineer".into(),
                    source: Some(format!("https://careers.{slug}.com/jobs/1")),
                    dir: self.core.paths.application_dir(slug),
                    ..Default::default()
                })
                .unwrap();
            self.core.store.force_status(app.id, status, "fixture").unwrap();
            app.id
        }

        async fn run(&self, llm: &FakeLlm, msgs: Vec<MailMessage>) -> SyncReport {
            let mut report = SyncReport::default();
            let notes = &self.notes;
            let notify = move |t: &str, b: &str| notes.lock().unwrap().push((t.to_string(), b.to_string()));
            process_messages(&self.core, llm, msgs, &notify, &mut report).await;
            report
        }
    }

    fn mail(id: &str, from: &str, subject: &str, day: u32) -> MailMessage {
        MailMessage {
            id: id.into(),
            from: from.into(),
            subject: subject.into(),
            date: Utc.with_ymd_and_hms(2024, 11, day, 9, 0, 0).unwrap(),
            snippet: "snippet".into(),
            body: "body".into(),
        }
    }

    fn reply(app: Option<i64>, kind: &str, confidence: f64) -> serde_json::Value {
        json!({"application_id": app, "kind": kind, "confidence": confidence, "summary": format!("{kind} summary")})
    }

    fn kinds(core: &Core, app: i64) -> Vec<String> {
        core.store.list_events(app).unwrap().into_iter().map(|e| e.kind).collect()
    }

    #[tokio::test]
    async fn confident_rejection_moves_status_notifies_and_logs() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);
        let llm = FakeLlm::with_replies([reply(Some(acme), "rejection", 0.95)]);

        let report = fx.run(&llm, vec![mail("m1", "Acme HR <jobs@acme.com>", "Your application", 10)]).await;

        assert_eq!((report.fetched, report.matched, report.needs_review), (1, 1, 0));
        assert_eq!(report.status_changes.len(), 1);
        assert_eq!((report.status_changes[0].from, report.status_changes[0].to), (Status::Applied, Status::Rejected));
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Rejected);
        assert_eq!(kinds(&fx.core, acme), ["created", "status", "status", "email"]);
        let events = fx.core.store.list_events(acme).unwrap();
        assert_eq!(events.last().unwrap().message, "email: Your application \u{2014} rejection summary");

        let email = fx.core.store.get_email("m1").unwrap().unwrap();
        assert_eq!((email.app_id, email.applied, email.kind.as_deref()), (Some(acme), true, Some("rejection")));
        assert!(!needs_review(&email));
        assert_eq!(fx.notes.lock().unwrap().len(), 1);
        assert!(fx.notes.lock().unwrap()[0].0.contains("Rejected"));
    }

    #[tokio::test]
    async fn status_never_moves_backwards() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Offer);
        let llm = FakeLlm::with_replies([reply(Some(acme), "rejection", 0.99), reply(Some(acme), "confirmation", 0.99)]);

        let report = fx
            .run(
                &llm,
                vec![mail("m1", "jobs@acme.com", "Sorry", 10), mail("m2", "jobs@acme.com", "We got your application", 11)],
            )
            .await;

        assert!(report.status_changes.is_empty());
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Offer);
        let events = fx.core.store.list_events(acme).unwrap();
        let email_events: Vec<_> = events.iter().filter(|e| e.kind == "email").collect();
        assert_eq!(email_events.len(), 2);
        assert!(email_events[0].message.ends_with("(status stays Offer)"), "{}", email_events[0].message);
        assert!(!fx.core.store.get_email("m1").unwrap().unwrap().applied);
        assert!(fx.notes.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn low_confidence_asks_for_review_and_accept_applies() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);
        let llm = FakeLlm::with_replies([reply(Some(acme), "interview", 0.55)]);

        let report = fx.run(&llm, vec![mail("m1", "jobs@acme.com", "Let's talk", 10)]).await;
        assert_eq!((report.matched, report.needs_review, report.status_changes.len()), (1, 1, 0));
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Applied);
        let events = fx.core.store.list_events(acme).unwrap();
        let review = events.iter().find(|e| e.kind == "email_review").expect("review event");
        assert!(review.message.starts_with("needs review: Let's talk"), "{}", review.message);
        assert!(review.message.contains("55%"), "{}", review.message);

        let pending = fx.core.emails_needing_review(Some(acme)).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(fx.core.emails_needing_review(Some(acme + 1)).unwrap().is_empty());

        let change = fx.core.accept_email("m1").unwrap().expect("status change");
        assert_eq!((change.from, change.to), (Status::Applied, Status::Interview));
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Interview);
        assert!(fx.core.emails_needing_review(None).unwrap().is_empty());
        assert!(fx.core.store.get_email("m1").unwrap().unwrap().applied);
    }

    #[tokio::test]
    async fn dismissing_a_review_keeps_the_status() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);
        let llm = FakeLlm::with_replies([reply(Some(acme), "offer", 0.4)]);
        fx.run(&llm, vec![mail("m1", "jobs@acme.com", "Maybe an offer", 10)]).await;
        assert_eq!(fx.core.emails_needing_review(None).unwrap().len(), 1);

        fx.core.dismiss_email("m1").unwrap();
        assert!(fx.core.emails_needing_review(None).unwrap().is_empty());
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Applied);
        assert!(fx.core.accept_email("m1").is_err());
    }

    #[tokio::test]
    async fn ids_outside_the_candidates_are_ignored() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);
        let other = fx.app("zeta", "Zeta", Status::Applied);
        let llm = FakeLlm::with_replies([reply(Some(other + 1000), "rejection", 0.99)]);

        let report = fx.run(&llm, vec![mail("m1", "jobs@acme.com", "Update", 10)]).await;
        assert_eq!((report.matched, report.status_changes.len()), (0, 0));
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Applied);
        assert_eq!(fx.core.store.get_email("m1").unwrap().unwrap().app_id, None);

        // The model was only offered the deterministic candidate (Acme), not every application.
        let prompt = &llm.calls()[0];
        assert!(prompt.user.contains(&format!("- id {acme}: Acme")));
        assert!(!prompt.user.contains("Zeta"));
        assert!(prompt.system.contains("UNTRUSTED DATA"));
    }

    #[tokio::test]
    async fn unrelated_mail_is_stored_without_calling_the_model() {
        let fx = Fixture::new();
        fx.app("acme", "Acme", Status::Applied);
        let llm = FakeLlm::with_replies([]);

        let report = fx
            .run(&llm, vec![mail("n1", "news@medium.com", "Your weekly digest", 10)])
            .await;
        assert_eq!((report.fetched, report.matched), (1, 0));
        assert!(llm.calls().is_empty());
        assert!(fx.core.store.get_email("n1").unwrap().is_some());
    }

    #[tokio::test]
    async fn job_mail_from_unknown_sender_goes_to_the_model_with_all_candidates() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);
        fx.app("draft-co", "Draft Co", Status::Draft);
        let llm = FakeLlm::with_replies([reply(Some(acme), "interview", 0.9)]);

        // Agency domain, but the subject looks like a job email.
        let report = fx.run(&llm, vec![mail("m1", "recruiter@staffing.io", "Interview for the backend position", 10)]).await;
        assert_eq!(report.status_changes.len(), 1);
        let user = &llm.calls()[0].user;
        assert!(user.contains("Acme"));
        assert!(!user.contains("Draft Co"), "drafts are not candidates");
    }

    #[tokio::test]
    async fn model_failure_is_reported_and_the_mail_is_retried_later() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Applied);

        let report = fx.run(&FakeLlm::with_error("no AI provider is set up"), vec![mail("m1", "jobs@acme.com", "Hi", 10)]).await;
        assert_eq!(report.fetched, 0);
        assert_eq!(report.errors, ["no AI provider is set up"]);
        assert!(fx.core.store.get_email("m1").unwrap().is_none());

        let llm = FakeLlm::with_replies([reply(Some(acme), "confirmation", 0.9)]);
        let report = fx.run(&llm, vec![mail("m1", "jobs@acme.com", "Hi", 10)]).await;
        assert_eq!((report.fetched, report.errors.len()), (1, 0));
        // Already Applied: the confirmation is logged but changes nothing.
        assert!(report.status_changes.is_empty());
        assert!(kinds(&fx.core, acme).contains(&"email".to_string()));
    }

    #[tokio::test]
    async fn seen_messages_are_skipped_and_order_is_oldest_first() {
        let fx = Fixture::new();
        let acme = fx.app("acme", "Acme", Status::Submitting);
        let llm = FakeLlm::with_replies([reply(Some(acme), "confirmation", 0.9), reply(Some(acme), "rejection", 0.9)]);

        let msgs = vec![mail("m1", "jobs@acme.com", "Confirmation", 10), mail("m2", "jobs@acme.com", "Rejection", 12)];
        let report = fx.run(&llm, msgs.clone()).await;
        assert_eq!(report.status_changes.len(), 2);
        assert_eq!(fx.core.store.get_application(acme).unwrap().unwrap().status, Status::Rejected);

        let again = fx.run(&llm, msgs).await;
        assert_eq!((again.fetched, llm.calls().len()), (0, 2));
    }

    #[tokio::test]
    async fn sync_requires_a_connection() {
        let fx = Fixture::new();
        let err = sync_once(&fx.core).await.unwrap_err();
        assert!(err.to_string().contains("not connected"), "{err}");
        assert!(fx.core.gmail_state().last_report.unwrap().error.unwrap().contains("not connected"));
    }

    #[test]
    fn pool_statuses() {
        assert!(!in_pool(Status::Draft) && !in_pool(Status::CvReady) && !in_pool(Status::Failed));
        assert!(in_pool(Status::Submitting) && in_pool(Status::Applied) && in_pool(Status::Offer));
        assert!(in_pool(Status::Interview) && in_pool(Status::Rejected));
    }
}
