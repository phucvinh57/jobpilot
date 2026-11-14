//! Changing the saved preferences from the app, one change at a time.
//!
//! Several things edit the same document: a chip click, a Trends "add", an undo, and the model
//! reading the free-text notes (the `prefs_chat` graph merges its patch into the document). Each
//! is a read, a change and a write, so two at once would lose one of them. Every call here takes
//! the same process-wide lock for its whole read-change-write, so they queue instead:
//!
//! ```text
//! core.edit_preferences(|p| { p.locations.push("Hanoi".into()); true }).await?   // a chip click
//! core.read_preference_notes(notes).await?                                      // save + model reads
//! core.undo_preference_edit().await?
//! ```
//!
//! The lock is not part of [`Core::update_preferences`] (the chat graph's own `merge` calls it
//! while a notes read holds the lock), so code that writes preferences outside these calls must
//! not run at the same time as them.
//!
//! A notes read does not hold the lock for ever: the model gets [`READ_TIMEOUT`], then the read
//! gives up (the notes stay saved). A read that is still waiting for the lock when the user
//! undoes a change is skipped ([`NotesRead::Stale`]), so it cannot save the undone notes again.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::Mutex;

use super::chat::MAX_NOTES_CHARS;
use super::{ChatStatus, Preferences, PrefsChat, THREAD};
use crate::engine::Core;

/// Held for the whole of an edit, a notes read or an undo.
static LOCK: Mutex<()> = Mutex::const_new(());

/// The longest the model may take over the notes while the lock is held: edits wait behind it, so
/// it must not hang for ever. When it passes, the read is dropped and the notes stay saved.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// What the user is told when [`READ_TIMEOUT`] passes.
const TOO_SLOW: &str = "reading them took too long";

/// How many undos each database has had so far (keyed by its file, so two apps in one process, like
/// tests, do not see each other's). A read compares it with the count at the time it was
/// requested.
static UNDOS: std::sync::Mutex<BTreeMap<PathBuf, u64>> = std::sync::Mutex::new(BTreeMap::new());

fn undo_count(core: &Core) -> u64 {
    UNDOS.lock().unwrap_or_else(|e| e.into_inner()).get(&core.paths.db).copied().unwrap_or(0)
}

fn count_undo(core: &Core) {
    *UNDOS.lock().unwrap_or_else(|e| e.into_inner()).entry(core.paths.db.clone()).or_insert(0) += 1;
}

/// How far the model got with the notes ([`Core::read_preference_notes`]). The notes themselves are
/// saved in every case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotesRead {
    /// The model read the notes and what it understood is merged into the preferences (roles,
    /// seniority, salary, visa...). Also the answer for empty notes: there was nothing to read.
    Read,
    /// The model asked a question instead of summarising, which nobody can answer here. What it
    /// did understand before that is kept.
    PartlyRead,
    /// The model could not be asked (no provider, no network...), or took longer than
    /// [`READ_TIMEOUT`]. The text is the failure's, for [`classify`](crate::engine::health::classify).
    Failed(String),
    /// The user undid a change after this read was requested, so it was dropped before it saved
    /// or asked anything: it would have put the undone notes back.
    Stale,
}

/// Notes as they are saved: trimmed and cut to [`MAX_NOTES_CHARS`] characters.
pub fn clean_notes(raw: &str) -> String {
    raw.trim().chars().take(MAX_NOTES_CHARS).collect()
}

impl Core {
    /// Read the saved preferences, let `change` edit them and save the result (keeping the undo
    /// point), all under the lock. `change` returns whether it changed anything: when it says
    /// `false` nothing is saved, so a no-op never touches the undo point. Returns the preferences
    /// as they are after the call.
    pub async fn edit_preferences(&self, change: impl FnOnce(&mut Preferences) -> bool) -> Result<Preferences> {
        let _guard = LOCK.lock().await;
        let current = self.store.preferences()?;
        let mut draft = current.clone();
        if !change(&mut draft) {
            return Ok(current);
        }
        self.update_preferences(draft)
    }

    /// Put back the preferences from before the last change, under the lock; `None` when there is
    /// nothing to undo. Notes reads requested before this call that have not started yet are
    /// dropped ([`NotesRead::Stale`]), also the ones queued ahead of it.
    pub async fn undo_preference_edit(&self) -> Result<Option<Preferences>> {
        // Before waiting for the lock: a read that is queued now was asked for before the undo.
        count_undo(self);
        let _guard = LOCK.lock().await;
        self.undo_preferences()
    }

    /// Save `notes` as the user's notes (when they changed) and let the preference chat read them
    /// once, so the model can fill what a form hides. Nothing is shown: a summary it proposes is
    /// confirmed at once, a question or a failure ends the thread. The stored notes stay the
    /// user's own text whatever the model proposes.
    ///
    /// The lock is held from the first read to the last write, so edits and other reads wait
    /// their turn: the model's merge cannot overwrite a chip click, and no chat turn is running
    /// when a new one starts. The model gets [`READ_TIMEOUT`]; after that the answer is
    /// [`NotesRead::Failed`] and the notes stay saved. An `Err` means the notes could not be saved
    /// or put back.
    pub async fn read_preference_notes(self: &Arc<Self>, notes: String) -> Result<NotesRead> {
        read_notes_with(self, &PrefsChat::new(self.clone()), &notes).await
    }
}

/// [`Core::read_preference_notes`] with a given chat (tests use a scripted model).
pub(super) async fn read_notes_with(core: &Core, chat: &PrefsChat, notes: &str) -> Result<NotesRead> {
    read_notes_within(core, chat, notes, READ_TIMEOUT).await
}

/// [`read_notes_with`] with the model's time limit given.
async fn read_notes_within(core: &Core, chat: &PrefsChat, notes: &str, limit: Duration) -> Result<NotesRead> {
    let requested = undo_count(core);
    let notes = clean_notes(notes);
    let _guard = LOCK.lock().await;
    if undo_count(core) != requested {
        tracing::info!("preferences: reading the notes skipped, a change was undone since it was asked for");
        return Ok(NotesRead::Stale);
    }
    let mut prefs = core.store.preferences()?;
    if prefs.notes != notes {
        prefs.notes = notes.clone();
        core.update_preferences(prefs)?;
    }
    if notes.is_empty() {
        return Ok(NotesRead::Read);
    }
    tracing::info!(chars = notes.chars().count(), "preferences: reading the notes");
    let outcome = match tokio::time::timeout(limit, settle(chat, &notes)).await {
        Ok(outcome) => outcome,
        Err(_) => {
            tracing::warn!(seconds = limit.as_secs(), "preferences: reading the notes took too long");
            Ok(NotesRead::Failed(TOO_SLOW.to_string()))
        }
    };
    if !matches!(outcome, Ok(NotesRead::Read)) {
        // No question or failure may wait around: there is no chat on screen to answer it.
        if let Err(e) = core.graphs.cancel(THREAD) {
            tracing::warn!("dropping the preference chat thread after reading the notes: {e:#}");
        }
    }
    // The chat may have re-worded the notes: they stay the user's. No new undo point.
    let kept = keep_notes(core, &notes);
    if let (Err(e), Err(_)) = (&kept, &outcome) {
        tracing::warn!("putting the notes back after a failed reading: {e:#}");
    }
    let read = outcome?;
    kept.context("putting your notes back after reading them")?;
    Ok(read)
}

/// Send `notes` to the chat and take it to the end of its thread.
async fn settle(chat: &PrefsChat, notes: &str) -> Result<NotesRead> {
    Ok(match chat.send(notes).await? {
        ChatStatus::Accepted => NotesRead::Read,
        // The user asked for exactly this by saving the notes: confirm the summary.
        ChatStatus::AwaitConfirm => match chat.accept().await? {
            ChatStatus::Accepted => NotesRead::Read,
            _ => NotesRead::PartlyRead,
        },
        ChatStatus::Failed(failure) => NotesRead::Failed(failure.message),
        ChatStatus::AwaitUser | ChatStatus::Working | ChatStatus::Idle => NotesRead::PartlyRead,
    })
}

fn keep_notes(core: &Core, notes: &str) -> Result<()> {
    let mut prefs = core.store.preferences()?;
    if prefs.notes != notes {
        prefs.notes = notes.to_string();
        core.save_preferences(prefs)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use serde_json::{json, Value};
    use tokio::sync::Notify;
    use tokio::task::JoinSet;

    use super::*;
    use crate::engine::llm::fake::FakeLlm;
    use crate::engine::llm::{Llm, LlmFuture};
    use crate::engine::prefs::Remote;
    use crate::engine::{runtime, Paths, Secrets};

    fn open_core(dir: &Path) -> Arc<Core> {
        Arc::new(Core::open_at(Paths::new(dir.join("jp")), Secrets::memory()).unwrap())
    }

    fn turn(complete: bool, patch: Value) -> Value {
        json!({"reply": "Noted.", "questions": [], "patch": patch, "complete": complete})
    }

    fn chat(core: &Arc<Core>, replies: Vec<Value>) -> (Arc<FakeLlm>, PrefsChat) {
        let llm = Arc::new(FakeLlm::with_replies(replies));
        (llm.clone(), PrefsChat::with_llm(core.clone(), llm))
    }

    fn read(core: &Arc<Core>, chat: &PrefsChat, notes: &str) -> NotesRead {
        runtime::block_on(read_notes_with(core, chat, notes)).unwrap()
    }

    /// A model that answers only when told to, so a test decides what happens meanwhile.
    struct GatedLlm {
        inner: FakeLlm,
        /// Questions that reached the model so far (answered or not).
        asked: AtomicUsize,
        entered: Notify,
        release: Notify,
    }

    impl GatedLlm {
        fn new(replies: Vec<Value>) -> Arc<Self> {
            Arc::new(Self { inner: FakeLlm::with_replies(replies), asked: AtomicUsize::new(0), entered: Notify::new(), release: Notify::new() })
        }
    }

    impl Llm for GatedLlm {
        fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
            Box::pin(async move {
                self.asked.fetch_add(1, Ordering::SeqCst);
                self.entered.notify_one();
                self.release.notified().await;
                self.inner.complete_json(system, user, schema).await
            })
        }

        fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
            self.inner.list_models()
        }
    }

    /// Wait for `fut`, but not for ever: a test that fails must not leave the process-wide lock held.
    async fn soon<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(10), fut).await.expect("timed out")
    }

    #[test]
    fn a_no_op_edit_saves_nothing_and_keeps_the_undo_point() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let a = core.update_preferences(Preferences { roles: vec!["SRE".into()], ..Default::default() }).unwrap();
        let b = runtime::block_on(core.edit_preferences(|p| {
            p.remote = Remote::Hybrid;
            true
        }))
        .unwrap();
        assert_eq!(b.remote, Remote::Hybrid);

        // `false`: whatever the closure did to its copy is dropped, nothing is written.
        let same = runtime::block_on(core.edit_preferences(|p| {
            p.roles.push("ignored".into());
            false
        }))
        .unwrap();
        assert_eq!(same, b, "the stored document comes back");
        assert_eq!(core.preferences().unwrap(), b);
        // `true` with nothing really changed keeps the undo point too.
        runtime::block_on(core.edit_preferences(|_| true)).unwrap();
        assert_eq!(core.undo_preferences().unwrap(), Some(a), "the undo point is still the document before the one real change");
    }

    #[test]
    fn edits_are_tidied_and_the_first_one_undoes_to_the_default() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let saved = runtime::block_on(core.edit_preferences(|p| {
            p.locations = vec![" Hanoi ".into(), "hanoi".into()];
            true
        }))
        .unwrap();
        assert_eq!(saved.locations, ["Hanoi"]);
        assert_eq!(runtime::block_on(core.undo_preference_edit()).unwrap(), Some(Preferences::default()));
        assert_eq!(runtime::block_on(core.undo_preference_edit()).unwrap(), None, "one level");
    }

    #[test]
    fn notes_are_trimmed_and_capped() {
        assert_eq!(clean_notes("  hello\n"), "hello");
        assert_eq!(clean_notes(&"x".repeat(MAX_NOTES_CHARS + 50)).chars().count(), MAX_NOTES_CHARS);
        assert_eq!(clean_notes("   "), "");
    }

    #[test]
    fn a_summary_is_confirmed_a_question_is_dropped_and_a_failure_keeps_the_notes() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());

        // A summary: confirmed at once, so the patch is kept and no thread waits for an answer.
        let (_, c) = chat(&core, vec![turn(true, json!({"roles": ["Backend Engineer"]}))]);
        assert_eq!(read(&core, &c, " Remote, senior, Rust "), NotesRead::Read);
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.roles.as_slice(), prefs.notes.as_str()), (&["Backend Engineer".to_string()][..], "Remote, senior, Rust"));
        assert_eq!(c.status().unwrap(), ChatStatus::Accepted);

        // A question: nobody answers it here, so no thread is left waiting.
        let (_, c) = chat(&core, vec![turn(false, json!({"salary_min": 90000}))]);
        assert_eq!(read(&core, &c, "More notes"), NotesRead::PartlyRead);
        assert_eq!(c.status().unwrap(), ChatStatus::Idle);
        assert!(core.graphs.waiting().unwrap().is_empty(), "no \"needs you\" left behind");
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.salary_min, prefs.notes.as_str()), (Some(90_000), "More notes"), "what it understood stays");

        // A failure: its text, the failed thread is dropped, the notes are saved.
        let c = PrefsChat::with_llm(core.clone(), Arc::new(FakeLlm::with_error("asking the AI: connection refused")));
        let NotesRead::Failed(message) = read(&core, &c, "Even more notes") else { panic!("expected a failure") };
        assert!(message.contains("connection refused"), "{message}");
        assert!(core.graphs.status(THREAD).unwrap().is_none());
        assert_eq!(core.preferences().unwrap().notes, "Even more notes");
    }

    #[test]
    fn empty_notes_are_saved_without_asking_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        core.update_preferences(Preferences { notes: "old".into(), ..Default::default() }).unwrap();
        let (llm, c) = chat(&core, vec![]);
        assert_eq!(read(&core, &c, "  \n "), NotesRead::Read);
        assert_eq!(core.preferences().unwrap().notes, "");
        assert!(llm.calls().is_empty());
    }

    #[test]
    fn the_notes_stay_the_users_text_when_the_model_rewrites_them() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let (_, c) = chat(&core, vec![turn(true, json!({"notes": "model words", "roles": ["SRE"]}))]);
        assert_eq!(read(&core, &c, "my own words"), NotesRead::Read);
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.roles.as_slice(), prefs.notes.as_str()), (&["SRE".to_string()][..], "my own words"));

        // The undo point is the document before the model's patch: it has the user's notes too.
        let undone = core.undo_preferences().unwrap().unwrap();
        assert_eq!((undone.roles.is_empty(), undone.notes.as_str()), (true, "my own words"));
    }

    #[test]
    fn saving_the_same_notes_again_after_a_failure_reads_them_again() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let broken = PrefsChat::with_llm(core.clone(), Arc::new(FakeLlm::with_error("error sending request for url (https://api.example.com)")));
        assert!(matches!(read(&core, &broken, "Remote, senior"), NotesRead::Failed(_)));
        assert_eq!(core.preferences().unwrap().notes, "Remote, senior");
        assert!(core.graphs.status(THREAD).unwrap().is_none(), "nothing left to block the next read");

        let (llm, c) = chat(&core, vec![turn(true, json!({"seniority": ["senior"], "remote": "remote"}))]);
        assert_eq!(read(&core, &c, "Remote, senior"), NotesRead::Read, "unchanged text, read again");
        assert_eq!(llm.calls().len(), 1);
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.seniority.as_slice(), prefs.remote, prefs.notes.as_str()), (&["senior".to_string()][..], Remote::Remote, "Remote, senior"));
    }

    #[test]
    fn a_chip_edit_waits_for_a_notes_read_and_neither_overwrites_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let llm = GatedLlm::new(vec![turn(true, json!({"roles": ["SRE"]}))]);
        let chat = Arc::new(PrefsChat::with_llm(core.clone(), llm.clone()));

        runtime::block_on(async {
            // A `JoinSet` aborts its tasks when the test fails, so the lock is never left held.
            let mut reader = JoinSet::new();
            reader.spawn({
                let (core, chat) = (core.clone(), chat.clone());
                async move { read_notes_with(&core, &chat, "Remote, senior").await }
            });
            soon(llm.entered.notified()).await; // the read holds the lock and the model is being asked
            let mut editor = JoinSet::new();
            editor.spawn({
                let core = core.clone();
                async move {
                    core.edit_preferences(|p| {
                        p.locations.push("Hanoi".into());
                        true
                    })
                    .await
                }
            });
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(editor.try_join_next().is_none(), "the chip edit waits for the read");
            assert!(core.preferences().unwrap().locations.is_empty());

            llm.release.notify_one();
            assert_eq!(soon(reader.join_next()).await.unwrap().unwrap().unwrap(), NotesRead::Read);
            let after = soon(editor.join_next()).await.unwrap().unwrap().unwrap();
            assert_eq!(after.roles, ["SRE"], "the edit started from the document with the model's merge");
            assert_eq!(after.locations, ["Hanoi"], "and the merge did not erase it");
            assert_eq!(after.notes, "Remote, senior");
            assert_eq!(core.preferences().unwrap(), after);
        });
    }

    #[test]
    fn two_notes_reads_run_one_after_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let llm = GatedLlm::new(vec![turn(true, json!({})), turn(true, json!({}))]);
        let chat = Arc::new(PrefsChat::with_llm(core.clone(), llm.clone()));

        runtime::block_on(async {
            let mut reads = JoinSet::new();
            for notes in ["first notes", "second notes"] {
                let (core, chat) = (core.clone(), chat.clone());
                reads.spawn(async move { read_notes_with(&core, &chat, notes).await });
            }
            soon(llm.entered.notified()).await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert_eq!(llm.asked.load(Ordering::SeqCst), 1, "the second read has not reached the model");
            llm.release.notify_one();
            soon(llm.entered.notified()).await;
            assert_eq!(llm.asked.load(Ordering::SeqCst), 2);
            llm.release.notify_one();
            while let Some(done) = soon(reads.join_next()).await {
                assert_eq!(done.unwrap().unwrap(), NotesRead::Read, "no \"still answering\" error for the one that waited");
            }
        });
        let last = llm.inner.calls().pop().unwrap();
        assert!(last.user.contains(&core.preferences().unwrap().notes), "the stored notes are the ones read last");
    }

    #[test]
    fn a_model_that_never_answers_ends_the_read_and_frees_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        // Never released: the model hangs.
        let llm = GatedLlm::new(vec![turn(true, json!({"roles": ["SRE"]}))]);
        let hanging = PrefsChat::with_llm(core.clone(), llm.clone());

        let timed_out = runtime::block_on(soon(read_notes_within(&core, &hanging, "Remote, senior", Duration::from_millis(200)))).unwrap();
        assert_eq!(timed_out, NotesRead::Failed(TOO_SLOW.to_string()));
        assert_eq!(llm.asked.load(Ordering::SeqCst), 1);
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.notes.as_str(), prefs.roles.is_empty()), ("Remote, senior", true), "the notes stay saved");
        assert!(core.graphs.status(THREAD).unwrap().is_none(), "the chat thread is dropped");
        assert!(core.graphs.waiting().unwrap().is_empty());

        // The lock is free again: an edit and the next read go through.
        runtime::block_on(soon(core.edit_preferences(|p| {
            p.locations.push("Hanoi".into());
            true
        })))
        .unwrap();
        let (_, c) = chat(&core, vec![turn(true, json!({"roles": ["SRE"]}))]);
        assert_eq!(read(&core, &c, "Remote, senior"), NotesRead::Read);
        assert_eq!(core.preferences().unwrap().roles, ["SRE"]);
    }

    #[test]
    fn a_read_waiting_for_the_lock_is_dropped_when_a_change_is_undone_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let llm = GatedLlm::new(vec![turn(true, json!({"roles": ["SRE"]})), turn(true, json!({"roles": ["Data"]}))]);
        let gated = Arc::new(PrefsChat::with_llm(core.clone(), llm.clone()));

        runtime::block_on(async {
            let mut tasks = JoinSet::new();
            tasks.spawn({
                let (core, gated) = (core.clone(), gated.clone());
                async move { read_notes_with(&core, &gated, "first notes").await.map(Some) }
            });
            soon(llm.entered.notified()).await; // the first read holds the lock
            let second = tasks.spawn({
                let (core, gated) = (core.clone(), gated.clone());
                async move { read_notes_with(&core, &gated, "second notes").await.map(Some) }
            });
            tokio::time::sleep(Duration::from_millis(100)).await; // the second read is queued
            let undo = tokio::spawn({
                let core = core.clone();
                async move { core.undo_preference_edit().await }
            });
            tokio::time::sleep(Duration::from_millis(100)).await;
            llm.release.notify_one();

            let mut reads = Vec::new();
            while let Some(done) = soon(tasks.join_next_with_id()).await {
                let (id, read) = done.unwrap();
                reads.push((id == second.id(), read.unwrap().unwrap()));
            }
            reads.sort_by_key(|(is_second, _)| *is_second);
            assert_eq!(reads, [(false, NotesRead::Read), (true, NotesRead::Stale)]);
            let undone = soon(undo).await.unwrap().unwrap().unwrap();
            assert_eq!(undone.notes, "first notes");
        });
        assert_eq!(llm.asked.load(Ordering::SeqCst), 1, "the dropped read never reached the model");
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.notes.as_str(), prefs.roles.is_empty()), ("first notes", true), "the undone state stays, and the second notes were never saved");

        // A read asked for after the undo is a normal one.
        let (_, c) = chat(&core, vec![turn(true, json!({"roles": ["Data"]}))]);
        assert_eq!(read(&core, &c, "third notes"), NotesRead::Read);
        assert_eq!(core.preferences().unwrap().roles, ["Data"]);
    }
}
