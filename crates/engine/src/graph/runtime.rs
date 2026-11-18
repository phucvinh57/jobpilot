//! [`Graphs`]: runs graphs, resumes paused or failed threads, reports progress.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use adk_graph::{Checkpoint, ExecutionConfig, GraphError, Interrupt};
use anyhow::{bail, ensure, Context, Result};
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use super::def::GraphDef;
use super::state::{diff, from_map, from_object, to_map, GraphState};
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
    /// The reason cannot be read (saved by another version of the app). The thread can still be
    /// resumed or cancelled.
    #[serde(other)]
    Unknown,
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
    /// Free-text progress of a running node ("Tailoring the CV"), from [`Graphs::note`]. Labels
    /// only: never page, posting or CV text.
    Detail {
        node: String,
        text: String,
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

struct Inner {
    store: Store,
    tx: broadcast::Sender<Progress>,
    /// Threads running in this process.
    active: Mutex<HashMap<String, ActiveRun>>,
    next_run: AtomicU64,
    /// Whether the one-time prune of old threads was started.
    pruned: AtomicBool,
}

/// A run in progress: its id (a thread can be cancelled and started again while the old run is
/// still winding down) and the token that stops it.
struct ActiveRun {
    id: u64,
    token: CancellationToken,
}

tokio::task_local! {
    static CANCEL: CancellationToken;
}

/// The cancellation token of the graph thread the caller runs in: cancelled by
/// [`Graphs::cancel`]. A node that hands work to a task of its own (`runtime::spawn`, a child
/// process) clones it and stops that work when it fires:
/// `tokio::select! { _ = graph::cancel_token().cancelled() => .., r = work => .. }`.
/// Outside a graph run it returns a token that never fires.
///
/// The node's own future needs no help: it is dropped when the thread is cancelled.
pub fn cancel_token() -> CancellationToken {
    CANCEL.try_with(CancellationToken::clone).unwrap_or_default()
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
    id: u64,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        let mut active = self.graphs.active();
        // `cancel` may have removed the entry, and a new run of the thread may have taken its place.
        if active.get(&self.thread).is_some_and(|run| run.id == self.id) {
            active.remove(&self.thread);
        }
    }
}

impl Graphs {
    /// The runtime over `store`. Does no I/O: old threads are removed by [`prune`](Self::prune),
    /// which also runs in the background the first time a graph is run or resumed.
    pub fn new(store: Store) -> Self {
        let (tx, _) = broadcast::channel(PROGRESS_BUFFER);
        let inner = Inner {
            store,
            tx,
            active: Mutex::new(HashMap::new()),
            next_run: AtomicU64::new(0),
            pruned: AtomicBool::new(false),
        };
        Self { inner: Arc::new(inner) }
    }

    /// Delete finished (done or failed) threads that were last touched more than 30 days ago, and
    /// threads a crash left `running`; returns how many. Waiting threads never expire. A database
    /// call: run it off the UI thread.
    pub fn prune(&self) -> Result<usize> {
        let cutoff = chrono::Utc::now() - chrono::Duration::days(FINISHED_TTL_DAYS);
        self.inner.store.graph_prune(cutoff)
    }

    /// Start the one-time background prune (needs the tokio runtime, which `run` and `resume` have).
    fn prune_once(&self) {
        if self.inner.pruned.swap(true, Ordering::Relaxed) {
            return;
        }
        let graphs = self.clone();
        tokio::task::spawn_blocking(move || match graphs.prune() {
            Ok(0) => {}
            Ok(n) => tracing::info!(threads = n, "pruned old graph threads"),
            Err(e) => tracing::warn!(error = %e, "pruning graph threads failed"),
        });
    }

    fn active(&self) -> std::sync::MutexGuard<'_, HashMap<String, ActiveRun>> {
        self.inner.active.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin(&self, thread: &str) -> Result<(ActiveGuard, CancellationToken)> {
        self.prune_once();
        let mut active = self.active();
        ensure!(!active.contains_key(thread), "graph thread `{thread}` is already running");
        let id = self.inner.next_run.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        active.insert(thread.to_string(), ActiveRun { id, token: token.clone() });
        Ok((ActiveGuard { graphs: self.clone(), thread: thread.to_string(), id }, token))
    }

    fn emit(&self, thread: &str, graph: &str, event: ProgressEvent) {
        let _ = self.inner.tx.send(Progress { thread: thread.into(), graph: graph.into(), event });
    }

    /// Tell subscribers what a running node is doing now. A node that wraps a long operation with
    /// its own progress (the CV pipeline) forwards it here, so the UI follows one stream.
    pub fn note(&self, thread: &str, graph: &str, node: &str, text: &str) {
        self.emit(thread, graph, ProgressEvent::Detail { node: node.into(), text: redact(text).into_owned() });
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
        let (_guard, token) = self.begin(thread)?;
        let store = &self.inner.store;
        if let Some(existing) = store.graph_thread(thread)? {
            ensure!(
                existing.status == GRAPH_DONE,
                "graph thread `{thread}` is {}: resume or cancel it first",
                existing.status
            );
            store.graph_delete_thread(thread)?;
        }
        let compiled = graph.compile(store, &self.inner.tx, &token)?;
        // Saved before the first node runs, so a thread that fails in its first step can be resumed.
        let first = Checkpoint::new(thread, to_map(&input)?.into_iter().collect(), 0, compiled.get_entry_nodes());
        if let Some(checkpointer) = compiled.checkpointer() {
            if let Err(error) = checkpointer.save(&first).await {
                // Cancelled before the run started: the saver refused on purpose.
                if token.is_cancelled() {
                    return Ok(Outcome::Cancelled);
                }
                return Err(error).context("saving the first checkpoint");
            }
        }
        tracing::info!(graph = %graph.name, %thread, "graph run started");
        self.drive(graph, compiled, thread, adk_graph::State::new(), token).await
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
        let (_guard, token) = self.begin(thread)?;
        let store = &self.inner.store;
        let info = store.graph_thread(thread)?.with_context(|| format!("no graph thread `{thread}`"))?;
        ensure!(info.graph == graph.name, "graph thread `{thread}` belongs to `{}`, not `{}`", info.graph, graph.name);
        let row = store.graph_latest_checkpoint(thread)?.context("the thread has no checkpoint")?;
        let saved: Value = serde_json::from_str(&row.state_json).context("reading the saved state")?;
        let Value::Object(saved) = saved else { bail!("the saved state is not an object") };
        let before: S = from_object(saved)?;
        if info.status == GRAPH_DONE {
            return Ok(Outcome::Done(before));
        }
        let before_map = to_map(&before)?;
        let mut after = before;
        change(&mut after);
        let input = diff(&before_map, &to_map(&after)?, graph.appends())?;
        let compiled = graph.compile(store, &self.inner.tx, &token)?;
        store.graph_set_status(thread, GRAPH_RUNNING, None, || !token.is_cancelled())?;
        tracing::info!(graph = %graph.name, %thread, "graph run resumed");
        self.drive(graph, compiled, thread, input, token).await
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

    /// Stop `thread` and forget it, with its checkpoints. Its node futures are dropped and its
    /// [`cancel_token`] fires, so work a node handed to tasks of its own can stop too; the run's
    /// `run` / `resume` call returns [`Outcome::Cancelled`]. The thread id is free at once: it can
    /// be run again right away. Child threads a node started with [`run_each`](Self::run_each) are
    /// dropped with it but keep their rows (as threads the app closed on): cancel them by id.
    ///
    /// A run counts from the moment its `run` / `resume` future is first polled: a cancel that
    /// comes before that (a run handed to `runtime::spawn` that has not started yet) finds no such
    /// thread and does nothing, and the run then starts as usual. Cancel a run once it reports
    /// progress ([`subscribe`](Self::subscribe)) or from the task that awaits it.
    pub fn cancel(&self, thread: &str) -> Result<()> {
        let graph = self.inner.store.graph_thread(thread)?.map(|t| t.graph);
        // The token first: once it fires no checkpoint or status of that run is written, so the
        // delete below leaves nothing behind (see `Store::graph_save_checkpoint`).
        let run = self.active().remove(thread);
        if let Some(run) = &run {
            run.token.cancel();
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
        Ok(self.inner.store.graph_thread(thread)?.map(|t| self.info(t)))
    }

    /// Threads paused at an interrupt, oldest first: what the "needs you" banner lists. Survives a
    /// restart. A thread whose reason cannot be read is listed with [`WaitReason::Unknown`].
    pub fn waiting(&self) -> Result<Vec<Waiting>> {
        let rows = self.inner.store.graph_threads_with_status(GRAPH_WAITING)?;
        Ok(rows
            .into_iter()
            .map(|t| Waiting {
                reason: read_wait_reason(&t.thread, t.detail_json.as_deref()),
                thread: t.thread,
                graph: t.graph,
                since: t.updated_at,
            })
            .collect())
    }

    /// Threads running in this process right now, by thread id (an order that does not move while
    /// they run: `updated_at` changes at every checkpoint): with
    /// [`waiting`](Self::waiting), what the "background jobs" list shows. A thread the app closed
    /// on is stored as running but is not listed.
    pub fn running(&self) -> Result<Vec<ThreadInfo>> {
        let threads: Vec<String> = self.active().keys().cloned().collect();
        let mut running = Vec::with_capacity(threads.len());
        for thread in threads {
            // A run that has not saved its first checkpoint yet has no row: it is listed a moment later.
            if let Some(info) = self.status(&thread)?.filter(|i| i.status == ThreadStatus::Running) {
                running.push(info);
            }
        }
        running.sort_by(|a, b| a.thread.cmp(&b.thread));
        Ok(running)
    }

    /// The nodes `thread` is at: those running now (several after a fan-out), or the one a waiting
    /// or failed thread stopped at. Empty for a finished or unknown thread.
    pub fn current_nodes(&self, thread: &str) -> Result<Vec<String>> {
        match self.inner.store.graph_latest_rest(thread)? {
            Some(rest) => super::checkpoint::pending_nodes(&rest),
            None => Ok(Vec::new()),
        }
    }

    fn info(&self, t: crate::store::GraphThread) -> ThreadInfo {
        let detail = t.detail_json.as_deref();
        let status = match t.status.as_str() {
            GRAPH_DONE => ThreadStatus::Done,
            GRAPH_WAITING => ThreadStatus::Waiting(read_wait_reason(&t.thread, detail)),
            GRAPH_FAILED => ThreadStatus::Failed(read_failure(&t.thread, detail)),
            _ if self.active().contains_key(&t.thread) => ThreadStatus::Running,
            _ => ThreadStatus::Failed(stored_failure("", "the app closed while this was running")),
        };
        ThreadInfo {
            thread: t.thread,
            graph: t.graph,
            step: u64::try_from(t.step).unwrap_or(0),
            status,
            updated_at: t.updated_at,
        }
    }

    /// Run the compiled graph on the thread's saved checkpoint and record how it ended.
    async fn drive<S: GraphState>(
        &self,
        graph: &GraphDef<S>,
        compiled: adk_graph::CompiledGraph,
        thread: &str,
        input: adk_graph::State,
        token: CancellationToken,
    ) -> Result<Outcome<S>> {
        let store = &self.inner.store;
        let config = ExecutionConfig::new(thread).with_recursion_limit(graph.limits.max_steps);
        let started = Instant::now();
        let run = CANCEL.scope(token.clone(), compiled.invoke(input, config));
        // Cancelling drops the run, and with it every node future.
        let ended = tokio::select! {
            biased;
            () = token.cancelled() => None,
            ended = run => Some(ended),
        };
        let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let live = || !token.is_cancelled();
        // `cancel` deleted the thread, and a cancelled run writes nothing more.
        let Some(ended) = ended else { return Ok(Outcome::Cancelled) };
        let (outcome, event, status, detail) = match ended {
            Ok(state) => {
                tracing::info!(graph = %graph.name, %thread, ms, "graph run done");
                (Outcome::Done(from_map(&state)?), ProgressEvent::Done, GRAPH_DONE, None)
            }
            Err(GraphError::Interrupted(paused)) => {
                let reason = wait_reason(&paused.interrupt)?;
                tracing::info!(graph = %graph.name, %thread, ms, "graph run waiting");
                let detail = serde_json::to_string(&reason)?;
                (Outcome::Waiting(reason.clone()), ProgressEvent::Waiting(reason), GRAPH_WAITING, Some(detail))
            }
            Err(error) => {
                let failure = failure(&error);
                tracing::warn!(graph = %graph.name, %thread, ms, node = %failure.node, "graph run failed");
                let stored = StoredFailure { node: failure.node.clone(), message: failure.message.clone() };
                (Outcome::Failed(failure), ProgressEvent::Failed, GRAPH_FAILED, Some(serde_json::to_string(&stored)?))
            }
        };
        if !store.graph_set_status(thread, status, detail.as_deref(), live)? {
            return Ok(Outcome::Cancelled);
        }
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
    stored_failure(&node, &message)
}

fn stored_failure(node: &str, message: &str) -> Failure {
    let message = redact(message).into_owned();
    Failure { problem: classify(&message), node: node.into(), message }
}

/// The wait reason of a stored thread; one that cannot be read becomes [`WaitReason::Unknown`].
fn read_wait_reason(thread: &str, detail: Option<&str>) -> WaitReason {
    detail.and_then(|d| serde_json::from_str(d).ok()).unwrap_or_else(|| {
        tracing::warn!(%thread, "unreadable wait reason");
        WaitReason::Unknown
    })
}

/// The failure of a stored thread; one that cannot be read becomes a generic failure.
fn read_failure(thread: &str, detail: Option<&str>) -> Failure {
    match detail.and_then(|d| serde_json::from_str::<StoredFailure>(d).ok()) {
        Some(f) => stored_failure(&f.node, &f.message),
        None => {
            tracing::warn!(%thread, "unreadable failure record");
            stored_failure("", "the failure record cannot be read")
        }
    }
}
