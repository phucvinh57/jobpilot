//! The `prefs_chat` graph (thread `prefs`) and [`PrefsChat`], the calls the UI makes on it.
//!
//! ```text
//! START -> load_context -> ask -> merge -+-> wait_user (interrupt: UserMessage) -> ask
//!                                        +-> confirm   (interrupt: ConfirmPreferences) -+-> END
//!                                                                                       +-> ask
//! ```
//!
//! - `load_context`: the stored conversation, fresh flags.
//! - `ask`: takes the user's message from the inbox and runs one model turn
//!   ([`chat::turn_schema`](super::chat)): reply, questions, patch, complete. The saved preferences
//!   and the profile are read again every turn, so a hand edit or an undo made while the chat is
//!   open counts. The patch is only checked here.
//! - `merge`: applies the patch in code ([`merge_patch`]), saves the preferences (keeping the
//!   previous ones for undo), stores the new messages in `chat_messages` and the questions in kv.
//! - `wait_user` / `confirm`: pause until [`PrefsChat::send`] / [`PrefsChat::accept`].
//!
//! The thread never expires (waiting threads are not pruned): reopening the chat later resumes it
//! with a new message. A finished (accepted) thread starts again from the stored conversation.
//! `chat_messages` is the source of truth for what was said; nodes may be re-run after a failure,
//! so `merge` stores only the messages the table does not have yet.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use super::chat::{
    clean_user_text, merge_patch, parse_turn, system_prompt, turn_history, turn_schema, PrefsPatch, Question,
};
use super::Preferences;
use crate::engine::graph::{Failure, GraphDef, Outcome, ThreadStatus, WaitReason, END, START};
use crate::engine::llm::{Llm, Message};
use crate::engine::store::ChatMessage;
use crate::engine::Core;

/// The graph's name (stored with its checkpoints).
pub const GRAPH: &str = "prefs_chat";
/// The one thread of the preference chat (also the `chat_messages` thread).
pub const THREAD: &str = "prefs";
/// A user message is at most this many characters; longer text goes in as a file.
pub const MAX_USER_CHARS: usize = 4_000;
/// kv key of the questions (quick replies) of the last turn.
pub(super) const QUESTIONS_KEY: &str = "prefs_chat_questions";

/// What the user answered at the confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confirmation {
    /// The summary is right: the thread ends.
    Accept,
    /// Something is wrong: the message in the inbox goes to the model.
    Correct,
}

/// The state of the `prefs_chat` thread.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ChatState {
    /// The conversation so far (append-only).
    #[serde(default)]
    pub messages: Vec<Message>,
    /// The saved preferences as of the last turn.
    #[serde(default)]
    pub preferences: Preferences,
    /// Questions of the last turn (quick replies).
    #[serde(default)]
    pub questions: Vec<Question>,
    /// The model showed a summary and wants the user's confirmation.
    #[serde(default)]
    pub complete: bool,
    /// The user's message, until `ask` has taken it.
    #[serde(default)]
    pub inbox: Option<String>,
    /// The patch of the last turn, until `merge` has applied it.
    #[serde(default)]
    pub patch: Option<PrefsPatch>,
    /// The answer at `confirm`.
    #[serde(default)]
    pub confirmation: Option<Confirmation>,
}

/// Where the model comes from: the default provider (looked up when a turn runs, so a missing
/// provider fails that turn with the "no AI provider" problem) or a fixed client (tests).
#[derive(Clone)]
enum LlmSource {
    Default,
    Fixed(Arc<dyn Llm>),
}

impl LlmSource {
    fn get(&self, core: &Core) -> Result<Arc<dyn Llm>> {
        match self {
            LlmSource::Default => Ok(Arc::from(core.default_llm()?)),
            LlmSource::Fixed(llm) => Ok(llm.clone()),
        }
    }
}

/// Where the chat stands, for the UI.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatStatus {
    /// No thread yet (or it was cancelled): a message starts one.
    Idle,
    /// A turn is running in this process.
    Working,
    /// Waiting for the user's next message.
    AwaitUser,
    /// Waiting for the user to confirm the summary or correct it.
    AwaitConfirm,
    /// The user confirmed; a new message reopens the conversation.
    Accepted,
    /// A node failed; [`PrefsChat::retry`] runs it again.
    Failed(Failure),
}

impl ChatStatus {
    fn from_outcome(outcome: Outcome<ChatState>) -> Self {
        match outcome {
            Outcome::Done(_) => ChatStatus::Accepted,
            Outcome::Waiting(WaitReason::ConfirmPreferences) => ChatStatus::AwaitConfirm,
            Outcome::Waiting(_) => ChatStatus::AwaitUser,
            Outcome::Failed(failure) => ChatStatus::Failed(failure),
            Outcome::Cancelled => ChatStatus::Idle,
        }
    }
}

/// Everything the Preferences screen shows, read in one go.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatView {
    pub status: ChatStatus,
    /// The stored conversation, oldest first.
    pub messages: Vec<ChatMessage>,
    /// Quick replies of the last turn.
    pub questions: Vec<Question>,
    pub preferences: Preferences,
    /// Preferences were saved at least once.
    pub saved: bool,
    pub can_undo: bool,
    /// The message of a failed turn that was never stored (so the UI can show it as "not sent").
    pub unsent: Option<String>,
}

impl Core {
    /// The state of the preference chat (database reads: run it off the UI thread).
    pub fn prefs_chat_view(&self) -> Result<ChatView> {
        let status = chat_status(self)?;
        let unsent = match &status {
            ChatStatus::Failed(_) => unsent_message(self)?,
            _ => None,
        };
        let questions = match self.store.kv_get(QUESTIONS_KEY)? {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => Vec::new(),
        };
        Ok(ChatView {
            status,
            messages: self.store.chat_messages(THREAD)?,
            questions,
            preferences: self.store.preferences()?,
            saved: self.store.has_preferences()?,
            can_undo: self.can_undo_preferences()?,
            unsent,
        })
    }
}

fn chat_status(core: &Core) -> Result<ChatStatus> {
    Ok(match core.graphs.status(THREAD)? {
        None => ChatStatus::Idle,
        Some(info) => match info.status {
            ThreadStatus::Running => ChatStatus::Working,
            ThreadStatus::Waiting(WaitReason::ConfirmPreferences) => ChatStatus::AwaitConfirm,
            ThreadStatus::Waiting(_) => ChatStatus::AwaitUser,
            ThreadStatus::Failed(failure) => ChatStatus::Failed(failure),
            ThreadStatus::Done => ChatStatus::Accepted,
        },
    })
}

/// The inbox of the thread's saved state: what the user said in the turn that failed.
fn unsent_message(core: &Core) -> Result<Option<String>> {
    let Some(row) = core.store.graph_latest_checkpoint(THREAD)? else { return Ok(None) };
    let state: serde_json::Value = serde_json::from_str(&row.state_json).context("reading the saved state")?;
    Ok(state.get("inbox").and_then(|v| v.as_str()).map(str::to_string))
}

/// The stored profile's compact YAML; empty when there is none (the chat works without).
fn read_profile(core: &Core) -> String {
    match core.profile() {
        Ok(Some(profile)) => profile.yaml,
        Ok(None) => String::new(),
        Err(e) => {
            tracing::warn!("preference chat: the profile cannot be read: {e:#}");
            String::new()
        }
    }
}

// ---------------------------------------------------------------- nodes

fn load_context(core: &Core, mut s: ChatState) -> Result<ChatState> {
    if s.messages.is_empty() {
        s.messages = core.store.chat_messages(THREAD)?.iter().map(ChatMessage::message).collect();
    }
    s.preferences = core.preferences()?;
    s.complete = false;
    s.confirmation = None;
    Ok(s)
}

async fn ask(core: &Core, llm: &LlmSource, mut s: ChatState) -> Result<ChatState> {
    if let Some(text) = s.inbox.take() {
        s.messages.push(Message::user(text));
    }
    s.confirmation = None;
    s.complete = false;
    s.preferences = core.preferences()?;
    let llm = llm.get(core)?;
    // Read each turn (not kept in the state, so it is not copied into every checkpoint and a
    // profile read again is seen from the next turn on).
    let system = system_prompt(&s.preferences, &read_profile(core));
    let history = turn_history(&s.messages);
    let value = llm.chat_json(&system, &history, &turn_schema()).await.context("asking the AI")?;
    let turn = parse_turn(value)?;
    // Check the patch now: a bad one fails this turn (and a retry asks again), not `merge`.
    merge_patch(&s.preferences, &turn.patch)?;
    s.messages.push(Message::assistant(turn.message_text()));
    s.complete = turn.complete;
    s.questions = turn.questions;
    s.patch = Some(turn.patch);
    Ok(s)
}

fn merge(core: &Core, mut s: ChatState) -> Result<ChatState> {
    let had = core.store.has_preferences()?;
    let current = core.store.preferences()?;
    let patch = s.patch.take().unwrap_or_default();
    // Nothing to save, except the first time a finished conversation leaves a row behind.
    s.preferences = if !patch.is_empty() || (s.complete && !had) {
        core.update_preferences(merge_patch(&current, &patch)?)?
    } else {
        current
    };
    let stored = core.store.chat_messages(THREAD)?.len();
    for m in s.messages.iter().skip(stored) {
        core.store.add_chat_message(THREAD, m.role, &m.content)?;
    }
    core.store.kv_set(QUESTIONS_KEY, &serde_json::to_string(&s.questions)?)?;
    tracing::info!(complete = s.complete, questions = s.questions.len(), "preference chat: turn merged");
    Ok(s)
}

/// The `prefs_chat` graph over `core`, with the model from `llm`.
fn build(core: Arc<Core>, llm: LlmSource) -> GraphDef<ChatState> {
    GraphDef::new(GRAPH)
        .append("messages")
        .node("load_context", {
            let core = core.clone();
            move |s| {
                let core = core.clone();
                async move { load_context(&core, s) }
            }
        })
        .node("ask", {
            let (core, llm) = (core.clone(), llm);
            move |s| {
                let (core, llm) = (core.clone(), llm.clone());
                async move { ask(&core, &llm, s).await }
            }
        })
        .node("merge", {
            let core = core.clone();
            move |s| {
                let core = core.clone();
                async move { merge(&core, s) }
            }
        })
        .wait("wait_user", |s| s.inbox.is_none().then_some(WaitReason::UserMessage))
        .wait("confirm", |s| s.confirmation.is_none().then_some(WaitReason::ConfirmPreferences))
        .edge(START, "load_context")
        .edge("load_context", "ask")
        .edge("ask", "merge")
        .route("merge", |s| if s.complete { "confirm" } else { "wait_user" }, &["confirm", "wait_user"])
        // A route, not an edge: a node with several incoming edges waits for all of them, which
        // `ask` (entered from `load_context` and from here) must not.
        .route("wait_user", |_| "ask", &["ask"])
        .route(
            "confirm",
            |s| if s.confirmation == Some(Confirmation::Accept) { END } else { "ask" },
            &[END, "ask"],
        )
}

// ---------------------------------------------------------------- calls

/// The calls the UI makes on the preference chat. Every method is `async` and meant for the app's
/// runtime (`runtime::spawn`); each runs the graph until it pauses again and says where it stands.
///
/// ```text
/// let chat = PrefsChat::new(core.clone());
/// let status = chat.open().await?;          // the model's first turn, when nothing was said yet
/// let status = chat.send("Remote, Rust").await?;
/// let status = chat.accept().await?;        // at AwaitConfirm
/// ```
pub struct PrefsChat {
    core: Arc<Core>,
    llm: LlmSource,
}

impl PrefsChat {
    /// The chat with the default AI provider (looked up when a turn runs).
    pub fn new(core: Arc<Core>) -> Self {
        Self { core, llm: LlmSource::Default }
    }

    /// The chat with a given model client (tests).
    pub fn with_llm(core: Arc<Core>, llm: Arc<dyn Llm>) -> Self {
        Self { core, llm: LlmSource::Fixed(llm) }
    }

    fn graph(&self) -> GraphDef<ChatState> {
        build(self.core.clone(), self.llm.clone())
    }

    /// Where the chat stands.
    pub fn status(&self) -> Result<ChatStatus> {
        chat_status(&self.core)
    }

    /// Start the conversation if nothing was said yet: the model speaks first. Otherwise only
    /// reports the status (no model call), so reopening an old chat is free.
    pub async fn open(&self) -> Result<ChatStatus> {
        let status = self.status()?;
        let fresh = matches!(status, ChatStatus::Idle | ChatStatus::Accepted) && self.core.store.chat_messages(THREAD)?.is_empty();
        if !fresh {
            return Ok(status);
        }
        tracing::info!("preference chat: opening turn");
        let outcome = self.core.graphs.run(&self.graph(), THREAD, ChatState::default()).await?;
        Ok(ChatStatus::from_outcome(outcome))
    }

    /// The user's message. Resumes the waiting thread; at the confirmation it counts as a
    /// correction; with no live thread (never started, accepted earlier, failed) it starts one.
    pub async fn send(&self, text: &str) -> Result<ChatStatus> {
        let text = clean_user_text(text, MAX_USER_CHARS)?;
        self.deliver(text).await
    }

    async fn deliver(&self, text: String) -> Result<ChatStatus> {
        let graph = self.graph();
        let graphs = &self.core.graphs;
        let fresh = |text: String| ChatState { inbox: Some(text), ..Default::default() };
        let outcome = match self.status()? {
            ChatStatus::Working => bail!("the assistant is still answering"),
            ChatStatus::AwaitUser => {
                graphs
                    .resume(&graph, THREAD, |s| {
                        s.inbox = Some(text);
                    })
                    .await?
            }
            ChatStatus::AwaitConfirm => {
                graphs
                    .resume(&graph, THREAD, |s| {
                        s.confirmation = Some(Confirmation::Correct);
                        s.inbox = Some(text);
                    })
                    .await?
            }
            // A failed turn is replaced by the new message; the old one was never stored.
            ChatStatus::Failed(_) => {
                graphs.cancel(THREAD)?;
                graphs.run(&graph, THREAD, fresh(text)).await?
            }
            ChatStatus::Idle | ChatStatus::Accepted => graphs.run(&graph, THREAD, fresh(text)).await?,
        };
        Ok(ChatStatus::from_outcome(outcome))
    }

    /// The user confirms the summary: the thread ends.
    pub async fn accept(&self) -> Result<ChatStatus> {
        if self.status()? != ChatStatus::AwaitConfirm {
            bail!("there is nothing to confirm");
        }
        let outcome = self.core.graphs.resume(&self.graph(), THREAD, |s| s.confirmation = Some(Confirmation::Accept)).await?;
        let status = ChatStatus::from_outcome(outcome);
        if status == ChatStatus::Accepted {
            self.core.store.kv_delete(QUESTIONS_KEY)?;
            tracing::info!("preference chat: preferences confirmed");
        }
        Ok(status)
    }

    /// Run the failed step again.
    pub async fn retry(&self) -> Result<ChatStatus> {
        if !matches!(self.status()?, ChatStatus::Failed(_)) {
            bail!("there is nothing to retry");
        }
        tracing::info!("preference chat: retry");
        let outcome = self.core.graphs.resume(&self.graph(), THREAD, |_| {}).await?;
        Ok(ChatStatus::from_outcome(outcome))
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::{json, Value};

    use super::*;
    use crate::engine::health::Problem;
    use crate::engine::llm::fake::FakeLlm;
    use crate::engine::llm::Role;
    use crate::engine::prefs::Remote;
    use crate::engine::{runtime, Paths, Secrets};

    fn open_core(dir: &Path) -> Arc<Core> {
        Arc::new(Core::open_at(Paths::new(dir.join("jp")), Secrets::memory()).unwrap())
    }

    fn turn(reply: &str, questions: Value, patch: Value, complete: bool) -> Value {
        json!({"reply": reply, "questions": questions, "patch": patch, "complete": complete})
    }

    fn texts(core: &Core) -> Vec<(Role, String)> {
        core.store.chat_messages(THREAD).unwrap().into_iter().map(|m| (m.role, m.content)).collect()
    }

    /// Opening turn, then three user messages, each resumed from `wait_user`; the last turn is
    /// `complete`, so the thread ends at `confirm` with the preferences saved.
    #[test]
    fn three_turns_end_at_confirm_with_saved_preferences_and_the_thread_survives_a_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(FakeLlm::with_replies([
            turn("Hi! Let's set up your search.", json!([{"text": "Which roles?", "options": ["Backend", "SRE"]}]), json!({}), false),
            turn("Backend it is.", json!([{"text": "Remote or on-site?", "options": ["Remote", "Hybrid"]}]), json!({"roles": ["Backend Engineer"], "must_skills": ["Rust"]}), false),
            turn("Remote, noted.", json!([{"text": "Salary floor?", "options": []}]), json!({"remote": "remote", "locations": ["Vietnam"]}), false),
            turn("Summary: backend, Rust, remote, USD 90k. Correct?", json!([]), json!({"salary_min": 90000, "currency": "usd", "weights": {"skills": 60}}), true),
        ]));
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core.clone(), llm.clone());

        // Nothing is saved before the first turn changes something.
        assert!(!core.store.has_preferences().unwrap());
        assert_eq!(chat.status().unwrap(), ChatStatus::Idle);
        assert_eq!(runtime::block_on(chat.open()).unwrap(), ChatStatus::AwaitUser);
        assert!(!core.store.has_preferences().unwrap(), "an empty patch saves nothing");
        let view = core.prefs_chat_view().unwrap();
        assert_eq!(view.questions[0].options, ["Backend", "SRE"], "quick replies are kept for the UI");
        assert_eq!(view.messages.len(), 1);
        assert!(view.messages[0].content.contains("- Which roles?"), "the questions are part of the stored message");

        assert_eq!(runtime::block_on(chat.send("Backend please")).unwrap(), ChatStatus::AwaitUser);
        let saved = core.preferences().unwrap();
        assert_eq!(saved.roles, ["Backend Engineer"]);
        assert!(core.store.has_preferences().unwrap());

        // The app is closed and opened again: a new store and runtime over the same file.
        drop(chat);
        drop(core);
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core.clone(), llm.clone());
        assert_eq!(chat.status().unwrap(), ChatStatus::AwaitUser, "the paused thread is still there");
        assert_eq!(core.graphs.waiting().unwrap()[0].reason, WaitReason::UserMessage);

        assert_eq!(runtime::block_on(chat.send("Remote")).unwrap(), ChatStatus::AwaitUser);
        assert_eq!(runtime::block_on(chat.send("90k USD")).unwrap(), ChatStatus::AwaitConfirm);
        assert_eq!(core.graphs.waiting().unwrap()[0].reason, WaitReason::ConfirmPreferences);

        let prefs = core.preferences().unwrap();
        assert_eq!(prefs.roles, ["Backend Engineer"]);
        assert_eq!(prefs.must_skills, ["Rust"]);
        assert_eq!(prefs.remote, Remote::Remote);
        assert_eq!(prefs.locations, ["Vietnam"]);
        assert_eq!((prefs.salary_min, prefs.currency.as_deref()), (Some(90_000), Some("USD")));
        assert_eq!(prefs.weights.sum(), 100);
        assert!(prefs.weights.skills > 35);

        // Four model turns, in order, with a growing history; stored once each.
        let said = texts(&core);
        assert_eq!(said.len(), 7);
        assert_eq!(said.iter().filter(|(r, _)| *r == Role::User).count(), 3);
        assert_eq!(said[1], (Role::User, "Backend please".into()));
        assert_eq!(said[6].0, Role::Assistant);
        let calls = llm.calls();
        assert_eq!(calls.len(), 4);
        assert!(calls[0].user.contains("just opened the preference chat"));
        assert!(calls[3].user.contains("[user]\nBackend please") && calls[3].user.contains("[user]\n90k USD"));
        assert!(calls[3].system.contains("\"Backend Engineer\""), "the saved preferences are in the context");

        // The user confirms: the thread ends and the row stays.
        assert_eq!(runtime::block_on(chat.accept()).unwrap(), ChatStatus::Accepted);
        assert_eq!(core.preferences().unwrap(), prefs);
        assert!(core.prefs_chat_view().unwrap().questions.is_empty());
        assert!(runtime::block_on(chat.accept()).is_err(), "nothing left to confirm");
    }

    #[test]
    fn a_correction_at_the_confirmation_goes_back_to_the_model() {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(FakeLlm::with_replies([
            turn("Summary: SRE. Correct?", json!([]), json!({"roles": ["SRE"]}), true),
            turn("Changed. Summary: SRE and platform. Correct?", json!([]), json!({"roles": ["SRE", "Platform Engineer"]}), true),
            turn("Welcome back. Added Singapore.", json!([]), json!({"locations": ["Singapore"], "remote": "hybrid"}), false),
        ]));
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core.clone(), llm.clone());

        assert_eq!(runtime::block_on(chat.open()).unwrap(), ChatStatus::AwaitConfirm);
        assert_eq!(core.preferences().unwrap().roles, ["SRE"]);
        let sent = runtime::block_on(chat.send("Also platform roles")).unwrap();
        assert_eq!(sent, ChatStatus::AwaitConfirm, "the corrected summary is shown for confirmation again");
        assert_eq!(core.preferences().unwrap().roles, ["SRE", "Platform Engineer"]);
        assert_eq!(runtime::block_on(chat.accept()).unwrap(), ChatStatus::Accepted);

        // Later: "I now also accept hybrid in Singapore" reopens the conversation from the store.
        assert_eq!(runtime::block_on(chat.send("I now also accept hybrid in Singapore")).unwrap(), ChatStatus::AwaitUser);
        let prefs = core.preferences().unwrap();
        assert_eq!((prefs.remote, prefs.locations.as_slice()), (Remote::Hybrid, &["Singapore".to_string()][..]));
        let last = llm.calls().pop().unwrap();
        assert!(last.user.contains("[user]\nAlso platform roles"), "the earlier conversation is loaded again: {}", last.user);
        assert_eq!(texts(&core).len(), 5, "opening, correction, summary, new message, answer");

        // And the last change can be undone.
        assert!(core.can_undo_preferences().unwrap());
        let undone = core.undo_preferences().unwrap().unwrap();
        assert_eq!(undone.roles, ["SRE", "Platform Engineer"]);
        assert!(undone.locations.is_empty());
    }

    #[test]
    fn a_failed_turn_keeps_the_message_and_retry_runs_it_again_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(FakeLlm::with_replies([
            turn("Hi.", json!([]), json!({}), false),
            json!({"reply": 5}),                                // not a turn
            json!({"reply": "x", "questions": [], "complete": false, "patch": {"salary_min": 5_000_000_000u64}}), // implausible
            turn("Got it.", json!([]), json!({"roles": ["SRE"]}), false),
        ]));
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core.clone(), llm);
        runtime::block_on(chat.open()).unwrap();

        let status = runtime::block_on(chat.send("SRE roles")).unwrap();
        let ChatStatus::Failed(failure) = status else { panic!("expected a failure, got {status:?}") };
        assert_eq!(failure.node, "ask");
        let view = core.prefs_chat_view().unwrap();
        assert_eq!(view.unsent.as_deref(), Some("SRE roles"), "the message of the failed turn is not lost");
        assert_eq!(view.messages.len(), 1, "and not stored yet");

        let again = runtime::block_on(chat.retry()).unwrap();
        assert!(matches!(again, ChatStatus::Failed(_)), "a patch that fails the checks fails the turn: {again:?}");
        assert_eq!(runtime::block_on(chat.retry()).unwrap(), ChatStatus::AwaitUser);
        assert_eq!(core.preferences().unwrap().roles, ["SRE"]);
        let said = texts(&core);
        assert_eq!(said.iter().filter(|(r, c)| *r == Role::User && c == "SRE roles").count(), 1, "stored once: {said:?}");
        assert_eq!(said.len(), 3);
        assert!(runtime::block_on(chat.retry()).is_err());
    }

    #[test]
    fn a_missing_provider_fails_the_turn_with_the_right_problem() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let chat = PrefsChat::new(core.clone());
        let status = runtime::block_on(chat.open()).unwrap();
        let ChatStatus::Failed(failure) = status else { panic!("expected a failure, got {status:?}") };
        assert_eq!(failure.problem, Problem::NoProvider);

        let network = FakeLlm::with_error("error sending request for url (https://api.example.com)");
        let dir2 = tempfile::tempdir().unwrap();
        let core2 = open_core(dir2.path());
        let status = runtime::block_on(PrefsChat::with_llm(core2, Arc::new(network)).open()).unwrap();
        let ChatStatus::Failed(failure) = status else { panic!("expected a failure") };
        assert_eq!(failure.problem, Problem::Network);
    }

    #[test]
    fn hand_edits_and_the_profile_reach_the_next_turn() {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(FakeLlm::with_replies([
            turn("Hi.", json!([]), json!({}), false),
            turn("Noted.", json!([]), json!({}), false),
            turn("And now?", json!([]), json!({}), false),
        ]));
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core.clone(), llm.clone());
        runtime::block_on(chat.open()).unwrap();
        assert!(llm.calls()[0].system.contains("(no profile yet)"), "the chat works without a profile");

        // The user edits the list by hand while the chat waits.
        let mut prefs = core.preferences().unwrap();
        prefs.domains = vec!["fintech".into()];
        core.update_preferences(prefs).unwrap();

        runtime::block_on(chat.send("SRE roles")).unwrap();
        assert!(llm.calls()[1].system.contains("\"fintech\""), "the hand edit is in the context");

        // A profile read while the chat waits reaches the next turn, wrapped as data.
        crate::engine::profile::set_test_profile(&core, "basics:\n  name: Jane\n");
        runtime::block_on(chat.send("Anything else?")).unwrap();
        let third = &llm.calls()[2].system;
        assert!(third.contains("<profile>") && third.contains("name: Jane"), "{third}");
        assert!(!third.contains("uploaded_cv"));
    }

    #[test]
    fn state_round_trips_through_a_checkpoint() {
        let state = ChatState {
            messages: vec![Message::user("hi")],
            patch: Some(PrefsPatch { salary_min: Some(None), roles: Some(vec!["SRE".into()]), ..Default::default() }),
            confirmation: Some(Confirmation::Correct),
            ..Default::default()
        };
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(serde_json::from_str::<ChatState>(&json).unwrap(), state);
        assert_eq!(serde_json::from_str::<ChatState>("{}").unwrap(), ChatState::default(), "old threads stay loadable");
    }

    #[test]
    fn sending_needs_text_and_a_free_assistant() {
        let dir = tempfile::tempdir().unwrap();
        let core = open_core(dir.path());
        let chat = PrefsChat::with_llm(core, Arc::new(FakeLlm::with_replies([])));
        assert!(runtime::block_on(chat.send("   ")).is_err());
        assert!(runtime::block_on(chat.send(&"x".repeat(MAX_USER_CHARS + 1))).is_err());
    }
}
