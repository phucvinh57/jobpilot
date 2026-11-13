//! [`Graphs`]: runs graphs, resumes paused or failed threads, reports progress.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use adk_graph::{Checkpoint, ExecutionConfig, GraphError, Interrupt};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use futures::future::{AbortHandle, AbortRegistration, Abortable};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use super::def::GraphDef;
use super::state::{diff, from_map, to_map, GraphState};
use crate::health::{classify, Problem};
use crate::redact::redact;
use crate::store::{GRAPH_DONE, GRAPH_FAILED, GRAPH_RUNNING, GRAPH_WAITING};
use crate::Store;

/// Finished (done or failed) threads are deleted after this many days.
const FINISHED_TTL_DAYS: i64 = 30;
/// Progress events buffered per subscriber; a slow one skips the oldest.
const PROGRESS_BUFFER: usize = 256;

/// Why a thread is paused: what the user has to do before it can go on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaitReason {
    /// The preference chat waits for the next message.
    UserMessage,
    /// The preference chat waits for the user to accept the summary or correct it.
    ConfirmPreferences,
    /// A source wants a login at `url`.
    Login { source_id: i64, url: String },
    /// A source shows a captcha at `url`.
    Captcha { source_id: i64, url: String },
    /// A source blocks the app at `url`.
    Blocked { source_id: i64, url: String },
}

/// A thread that ended with an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// What the user can do about it (and whether Retry is offered).
    pub problem: Problem,
    /// The node that failed; empty when the run failed outside a node.
    pub node: String,
    /// The error text, with secrets masked.
    pub message: String,
}

/// What [`Graphs::run`] and [`Graphs::resume`] return.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<S> {
    /// The graph reached its end; the final state.
    Done(S),
    /// An interrupt node paused the thread; resume it with [`Graphs::resume`].
    Waiting(WaitReason),
    /// A node failed; [`Graphs::resume`] re-runs the failed step.
    Failed(Failure),
    /// [`Graphs::cancel`] ended the thread while it ran.
    Cancelled,
}

/// A thread's status, as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadStatus {
    /// Running in this process.
    Running,
    Waiting(WaitReason),
    /// Also the status of a thread that was running when the app closed.
    Failed(Failure),
    Done,
}

/// A thread's [`ThreadStatus`] with its identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadInfo {
    pub thread: String,
    pub graph: String,
    /// Steps completed so far.
    pub step: u64,
    pub status: ThreadStatus,
    pub updated_at: DateTime<Utc>,
}

/// A paused thread: one entry of the "needs you" list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Waiting {
    pub thread: String,
    pub graph: String,
    pub reason: WaitReason,
    /// When it paused.
    pub since: DateTime<Utc>,
}

/// One progress event, from [`Graphs::subscribe`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub thread: String,
    pub graph: String,
    pub event: ProgressEvent,
}

/// What happened in a thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgressEvent {
    NodeStarted {
        node: String,
    },
    NodeFinished {
        node: String,
        ms: u64,
    },
    NodeFailed {
        node: String,
        message: String,
    },
    /// The thread paused.
    Waiting(WaitReason),
    Done,
    Failed,
    Cancelled,
}

/// The failure as stored in `detail_json`.
#[derive(Serialize, Deserialize)]
struct StoredFailure {
    node: String,
    message: String,
}

impl StoredFailure {
    fn into_failure(self) -> Failure {
        Failure { problem: classify(&self.message), node: self.node, message: self.message }
    }
}

struct Inner {
    store: Store,
    tx: broadcast::Sender<Progress>,
    /// Threads running in this process, with the handle that stops them.
    active: Mutex<HashMap<String, AbortHandle>>,
}

/// The graph runtime. Cheap to clone; one per app (see `Core::graphs`).
///
/// Calls are `async` and meant for the app's global runtime: from the UI use
/// [`runtime::spawn`](crate::runtime::spawn) and await its receiver.
#[derive(Clone)]
pub struct Graphs {
    inner: Arc<Inner>,
}

/// Marks a thread as running for as long as it lives.
struct ActiveGuard {
    graphs: Graphs,
    thread: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.graphs.active().remove(&self.thread);
    }
}

impl Graphs {
    /// The runtime over `store`. Deletes finished threads older than 30 days.
    pub fn new(store: Store) -> Self {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(FINISHED_TTL_DAYS);
        match store.graph_prune(cutoff) {
            Ok(0) => {}
            Ok(n) => tracing::info!(threads = n, "pruned finished graph threads"),
            Err(e) => tracing::warn!(error = %e, "pruning graph threads failed"),
        }
        let (tx, _) = broadcast::channel(PROGRESS_BUFFER);
        Self { inner: Arc::new(Inner { store, tx, active: Mutex::new(HashMap::new()) }) }
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, AbortHandle>> {
        self.inner.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin(&self, thread: &str) -> Result<(ActiveGuard, AbortRegistration)> {
        let mut active = self.active();
        ensure!(!active.contains_key(thread), "graph thread `{thread}` is already running");
        let (handle, registration) = AbortHandle::new_pair();
        active.insert(thread.to_string(), handle);
        Ok((ActiveGuard { graphs: self.clone(), thread: thread.to_string() }, registration))
    }

    fn emit(&self, thread: &str, graph: &str, event: ProgressEvent) {
        let _ = self.inner.tx.send(Progress { thread: thread.into(), graph: graph.into(), event });
    }

    /// Progress events of every thread (nodes started, finished, failed; threads done, waiting,
    /// failed, cancelled). Receivers that fall behind miss the oldest events.
    pub fn subscribe(&self) -> broadcast::Receiver<Progress> {
        self.inner.tx.subscribe()
    }

    /// Start `graph` on a new `thread` with `input` as its state, and run until it ends or pauses.
    ///
    /// A finished (done) thread of the same id is replaced. A thread that is running, waiting or
    /// failed is not: [`resume`](Self::resume) or [`cancel`](Self::cancel) it first.
    pub async fn run<S: GraphState>(&self, graph: &GraphDef<S>, thread: &str, input: S) -> Result<Outcome<S>> {
        let (_guard, registration) = self.begin(thread)?;
        let store = &self.inner.store;
        if let Some(existing) = store.graph_thread(thread)? {
            ensure!(
                existing.status == GRAPH_DONE,
                "graph thread `{thread}` is {}: resume or cancel it first",
                existing.status
            );
            store.graph_delete_thread(thread)?;
        }
        let compiled = graph.compile(store, &self.inner.tx)?;
        // Saved before the first node runs, so a thread that fails in its first step can be resumed.
        let first = Checkpoint::new(thread, to_map(&input)?.into_iter().collect(), 0, compiled.get_entry_nodes());
        if let Some(checkpointer) = compiled.checkpointer() {
            checkpointer.save(&first).await.context("saving the first checkpoint")?;
        }
        tracing::info!(graph = %graph.name, %thread, "graph run started");
        self.drive(graph, compiled, thread, adk_graph::State::new(), registration).await
    }

    /// Continue a waiting or failed `thread`.
    ///
    /// `change` edits the saved state first: the user's answer for a waiting thread (so that its
    /// interrupt node lets it through), a fix for a failed one, or nothing (`|_| {}`) for "Retry" and
    /// "Continue". The thread then goes on at the node it stopped at. A finished thread returns its
    /// final state.
    pub async fn resume<S: GraphState>(
        &self,
        graph: &GraphDef<S>,
        thread: &str,
        change: impl FnOnce(&mut S),
    ) -> Result<Outcome<S>> {
        let (_guard, registration) = self.begin(thread)?;
        let store = &self.inner.store;
        let info = store.graph_thread(thread)?.with_context(|| format!("no graph thread `{thread}`"))?;
        ensure!(info.graph == graph.name, "graph thread `{thread}` belongs to `{}`, not `{}`", info.graph, graph.name);
        let row = store.graph_latest_checkpoint(thread)?.context("the thread has no checkpoint")?;
        let saved: Value = serde_json::from_str(&row.state_json).context("reading the saved state")?;
        let before: S = serde_json::from_value(saved)
            .map_err(|e| anyhow::anyhow!("saved state does not fit the graph's type ({:?})", e.classify()))?;
        if info.status == GRAPH_DONE {
            return Ok(Outcome::Done(before));
        }
        let before_map = to_map(&before)?;
        let mut after = before;
        change(&mut after);
        let input = diff(&before_map, &to_map(&after)?, &graph.appends())?;
        let compiled = graph.compile(store, &self.inner.tx)?;
        store.graph_set_status(thread, GRAPH_RUNNING, None)?;
        tracing::info!(graph = %graph.name, %thread, "graph run resumed");
        self.drive(graph, compiled, thread, input, registration).await
    }

    /// Run `graph` once per `(thread, input)`, each as its own thread, at most `concurrency` at a
    /// time. One thread waiting or failing does not stop the others. For a node that fans work
    /// out to independent runs (see the module docs).
    pub async fn run_each<S: GraphState>(
        &self,
        graph: &GraphDef<S>,
        jobs: Vec<(String, S)>,
        concurrency: usize,
    ) -> Vec<(String, Result<Outcome<S>>)> {
        futures::stream::iter(jobs)
            .map(|(thread, input)| async move {
                let outcome = self.run(graph, &thread, input).await;
                (thread, outcome)
            })
            .buffer_unordered(concurrency.max(1))
            .collect()
            .await
    }

    /// Stop `thread` (if it runs in this process) and forget it, with its checkpoints.
    pub fn cancel(&self, thread: &str) -> Result<()> {
        let graph = self.inner.store.graph_thread(thread)?.map(|t| t.graph);
        if let Some(handle) = self.active().get(thread) {
            handle.abort();
        }
        self.inner.store.graph_delete_thread(thread)?;
        tracing::info!(%thread, "graph thread cancelled");
        if let Some(graph) = graph {
            self.emit(thread, &graph, ProgressEvent::Cancelled);
        }
        Ok(())
    }

    /// The status of `thread`, or `None` if there is no such thread.
    pub fn status(&self, thread: &str) -> Result<Option<ThreadInfo>> {
        self.inner.store.graph_thread(thread)?.map(|t| self.info(t)).transpose()
    }

    /// Threads paused at an interrupt, oldest first: what the "needs you" banner lists. Survives a
    /// restart.
    pub fn waiting(&self) -> Result<Vec<Waiting>> {
        let rows = self.inner.store.graph_threads_with_status(GRAPH_WAITING)?;
        Ok(rows
            .into_iter()
            .filter_map(|t| {
                let reason = serde_json::from_str(t.detail_json.as_deref()?).ok()?;
                Some(Waiting { thread: t.thread, graph: t.graph, reason, since: t.updated_at })
            })
            .collect())
    }

    fn info(&self, t: crate::store::GraphThread) -> Result<ThreadInfo> {
        let detail = t.detail_json.as_deref().unwrap_or("null");
        let status = match t.status.as_str() {
            GRAPH_DONE => ThreadStatus::Done,
            GRAPH_WAITING => ThreadStatus::Waiting(serde_json::from_str(detail).context("reading the wait reason")?),
            GRAPH_FAILED => ThreadStatus::Failed(
                serde_json::from_str::<StoredFailure>(detail).context("reading the failure")?.into_failure(),
            ),
            _ if self.active().contains_key(&t.thread) => ThreadStatus::Running,
            _ => ThreadStatus::Failed(
                StoredFailure { node: String::new(), message: "the app closed while this was running".into() }
                    .into_failure(),
            ),
        };
        Ok(ThreadInfo {
            thread: t.thread,
            graph: t.graph,
            step: u64::try_from(t.step).unwrap_or(0),
            status,
            updated_at: t.updated_at,
        })
    }

    /// Run the compiled graph on the thread's saved checkpoint and record how it ended.
    async fn drive<S: GraphState>(
        &self,
        graph: &GraphDef<S>,
        compiled: adk_graph::CompiledGraph,
        thread: &str,
        input: adk_graph::State,
        registration: AbortRegistration,
    ) -> Result<Outcome<S>> {
        let store = &self.inner.store;
        let config = ExecutionConfig::new(thread).with_recursion_limit(graph.limits.max_steps);
        let started = Instant::now();
        let ended = Abortable::new(compiled.invoke(input, config), registration).await;
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let (outcome, event) = match ended {
            Err(_aborted) => {
                // `cancel` deleted the thread; a step that was mid-save may have written again.
                store.graph_delete_thread(thread)?;
                return Ok(Outcome::Cancelled);
            }
            Ok(Ok(state)) => {
                store.graph_set_status(thread, GRAPH_DONE, None)?;
                tracing::info!(graph = %graph.name, %thread, ms, "graph run done");
                (Outcome::Done(from_map(&state)?), ProgressEvent::Done)
            }
            Ok(Err(GraphError::Interrupted(paused))) => {
                let reason = wait_reason(&paused.interrupt)?;
                store.graph_set_status(thread, GRAPH_WAITING, Some(&serde_json::to_string(&reason)?))?;
                tracing::info!(graph = %graph.name, %thread, ms, "graph run waiting");
                (Outcome::Waiting(reason.clone()), ProgressEvent::Waiting(reason))
            }
            Ok(Err(error)) => {
                let failure = failure(&error);
                let stored = StoredFailure { node: failure.node.clone(), message: failure.message.clone() };
                store.graph_set_status(thread, GRAPH_FAILED, Some(&serde_json::to_string(&stored)?))?;
                tracing::warn!(graph = %graph.name, %thread, ms, node = %failure.node, "graph run failed");
                (Outcome::Failed(failure), ProgressEvent::Failed)
            }
        };
        self.emit(thread, &graph.name, event);
        Ok(outcome)
    }
}

/// The reason carried by the interrupt a wait node raised.
fn wait_reason(interrupt: &Interrupt) -> Result<WaitReason> {
    match interrupt {
        Interrupt::Dynamic { data: Some(data), .. } => {
            serde_json::from_value(data.clone()).context("reading the wait reason")
        }
        _ => bail!("the graph paused without a wait reason"),
    }
}

/// A library error as a [`Failure`].
fn failure(error: &GraphError) -> Failure {
    let (node, message) = match error {
        GraphError::NodeExecutionFailed { node, message } => (node.clone(), message.clone()),
        other => (String::new(), other.to_string()),
    };
    let message = redact(&message).into_owned();
    Failure { problem: classify(&message), node, message }
}
