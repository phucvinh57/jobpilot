//! [`GraphDef`]: the builder for a graph over a typed state, and its translation to the library.

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use adk_graph::{CompiledGraph, NodeContext, NodeOutput, StateGraph, StateSchema};
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use futures::FutureExt;
use tokio::sync::broadcast;

use super::runtime::{Progress, ProgressEvent, WaitReason};
use super::state::{diff, from_map, to_map, GraphState};
use super::SqliteCheckpointer;
use crate::redact::redact;
use crate::Store;

/// Limits applied to every run of a graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// A node running longer fails with a "timed out" error (classified as a network problem, so
    /// the UI offers Retry). Default 5 minutes.
    pub node_timeout: Duration,
    /// Maximum steps of a thread over its whole life, cycles included; resumes continue the
    /// count. Default 500.
    pub max_steps: usize,
    /// Maximum nodes of one step running at once. Default 4.
    pub max_concurrency: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self { node_timeout: Duration::from_secs(300), max_steps: 500, max_concurrency: 4 }
    }
}

/// What a node does: pass the (new) state on, or pause the thread.
pub(crate) enum Step<S> {
    Next(S),
    Wait(WaitReason),
}

type NodeFn<S> = Arc<dyn Fn(S) -> BoxFuture<'static, Result<Step<S>>> + Send + Sync>;
type RouteFn<S> = Arc<dyn Fn(&S) -> &'static str + Send + Sync>;

#[derive(Clone)]
struct NodeDef<S> {
    name: String,
    run: NodeFn<S>,
}

#[derive(Clone)]
struct RouteDef<S> {
    from: String,
    pick: RouteFn<S>,
    targets: Vec<&'static str>,
}

/// A graph: named nodes over the typed state `S`, connected by edges. Built with the methods
/// below, run by [`Graphs`](super::Graphs). Cheap to clone.
#[derive(Clone)]
pub struct GraphDef<S> {
    pub(crate) name: String,
    nodes: Vec<NodeDef<S>>,
    edges: Vec<(String, String)>,
    routes: Vec<RouteDef<S>>,
    appends: Vec<&'static str>,
    timeouts: Vec<(String, Duration)>,
    pub(crate) limits: Limits,
}

impl<S: GraphState> GraphDef<S> {
    /// An empty graph called `name` (stored with its checkpoints; keep it stable).
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            nodes: Vec::new(),
            edges: Vec::new(),
            routes: Vec::new(),
            appends: Vec::new(),
            timeouts: Vec::new(),
            limits: Limits::default(),
        }
    }

    /// The graph's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn appends(&self) -> Vec<&'static str> {
        self.appends.clone()
    }

    /// A node: an async function from the state to the new state. An `Err` fails the thread at
    /// this node; the error text is classified into a [`Problem`](crate::health::Problem).
    pub fn node<F, Fut>(mut self, name: &str, f: F) -> Self
    where
        F: Fn(S) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<S>> + Send + 'static,
    {
        let run: NodeFn<S> = Arc::new(move |s| f(s).map(|r| r.map(Step::Next)).boxed());
        self.nodes.push(NodeDef { name: name.into(), run });
        self
    }

    /// An interrupt node: `Some(reason)` pauses the thread until [`Graphs::resume`] changes the
    /// state so that the predicate returns `None`, which lets the thread through unchanged.
    ///
    /// [`Graphs::resume`]: super::Graphs::resume
    pub fn wait<F>(mut self, name: &str, ask: F) -> Self
    where
        F: Fn(&S) -> Option<WaitReason> + Send + Sync + 'static,
    {
        let run: NodeFn<S> = Arc::new(move |s| {
            let step = match ask(&s) {
                Some(reason) => Step::Wait(reason),
                None => Step::Next(s),
            };
            futures::future::ready(Ok(step)).boxed()
        });
        self.nodes.push(NodeDef { name: name.into(), run });
        self
    }

    /// An edge `from -> to`. `from` may be [`START`](super::START) and `to` [`END`](super::END).
    /// Several edges out of one node run their targets in parallel; a node with several incoming
    /// edges waits for all of them.
    pub fn edge(mut self, from: &str, to: &str) -> Self {
        self.edges.push((from.into(), to.into()));
        self
    }

    /// A conditional edge: after `from`, `pick` looks at the state and names the next node, one of
    /// `targets` (which may include [`END`](super::END)). An answer outside `targets` fails the
    /// thread.
    pub fn route<F>(mut self, from: &str, pick: F, targets: &[&'static str]) -> Self
    where
        F: Fn(&S) -> &'static str + Send + Sync + 'static,
    {
        self.routes.push(RouteDef { from: from.into(), pick: Arc::new(pick), targets: targets.to_vec() });
        self
    }

    /// `from` fans out to every node in `branches` (one step, in parallel); they join at `join`,
    /// which runs once all of them are done. Shorthand for the edges.
    pub fn parallel(mut self, from: &str, branches: &[&str], join: &str) -> Self {
        for branch in branches {
            self.edges.push((from.into(), (*branch).into()));
            self.edges.push(((*branch).into(), join.into()));
        }
        self
    }

    /// Declare `field` of the state (a `Vec`) as an append list: parallel nodes can each push to it
    /// without overwriting one another. Nodes may only add items at its end.
    pub fn append(mut self, field: &'static str) -> Self {
        self.appends.push(field);
        self
    }

    /// Replace the default [`Limits`].
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// A timeout for one node, instead of [`Limits::node_timeout`].
    pub fn node_timeout(mut self, node: &str, timeout: Duration) -> Self {
        self.timeouts.push((node.into(), timeout));
        self
    }

    /// The library graph, with checkpoints in `store` and progress sent on `tx`.
    pub(crate) fn compile(&self, store: &Store, tx: &broadcast::Sender<Progress>) -> Result<CompiledGraph> {
        let mut schema = StateSchema::builder();
        for field in &self.appends {
            schema = schema.list_channel(field);
        }
        let mut graph = StateGraph::new(schema.build());
        let env = Arc::new(NodeEnv { graph: self.name.clone(), tx: tx.clone(), appends: self.appends.clone() });
        for node in &self.nodes {
            let (env, name, run) = (env.clone(), node.name.clone(), node.run.clone());
            let timeout = (self.timeouts.iter().rev())
                .find(|(n, _)| *n == node.name)
                .map_or(self.limits.node_timeout, |(_, t)| *t);
            graph = graph
                .add_node_fn(&node.name, move |ctx| run_node(env.clone(), name.clone(), run.clone(), timeout, ctx));
        }
        for (from, to) in &self.edges {
            graph = graph.add_edge(from, to);
        }
        for route in &self.routes {
            let pick = route.pick.clone();
            // A state that no longer fits its type routes to a name no graph declares, which the
            // library reports as an error.
            let router = move |state: &adk_graph::State| {
                from_map::<S>(state).map_or_else(|_| "<invalid state>".to_string(), |s| pick(&s).to_string())
            };
            graph = graph.add_conditional_edges(&route.from, router, route.targets.iter().map(|t| (*t, *t)));
        }
        let compiled = graph
            .compile()
            .with_context(|| format!("graph `{}` is invalid", self.name))?
            .with_checkpointer(SqliteCheckpointer::new(store.clone(), &self.name))
            .with_max_concurrency(self.limits.max_concurrency);
        Ok(compiled)
    }
}

/// What every node of one run needs besides its function.
struct NodeEnv {
    graph: String,
    tx: broadcast::Sender<Progress>,
    appends: Vec<&'static str>,
}

impl NodeEnv {
    fn emit(&self, thread: &str, event: ProgressEvent) {
        let _ = self.tx.send(Progress { thread: thread.into(), graph: self.graph.clone(), event });
    }
}

/// One execution of a node: typed state in, channel updates (or an interrupt) out, with progress,
/// a timeout and a log line.
async fn run_node<S: GraphState>(
    env: Arc<NodeEnv>,
    node: String,
    run: NodeFn<S>,
    timeout: Duration,
    ctx: NodeContext,
) -> adk_graph::Result<NodeOutput> {
    let thread = ctx.config.thread_id.clone();
    env.emit(&thread, ProgressEvent::NodeStarted { node: node.clone() });
    let started = Instant::now();
    let result: Result<NodeOutput> = async {
        let state: S = from_map(&ctx.state)?;
        let before = to_map(&state)?;
        let step = tokio::time::timeout(timeout, run(state))
            .await
            .map_err(|_| anyhow::anyhow!("node `{node}` timed out after {}s", timeout.as_secs()))??;
        Ok(match step {
            Step::Next(after) => NodeOutput::new().with_updates(diff(&before, &to_map(&after)?, &env.appends)?),
            Step::Wait(reason) => NodeOutput::interrupt_with_data("waiting", serde_json::to_value(reason)?),
        })
    }
    .await;
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(output) => {
            tracing::info!(graph = %env.graph, %thread, %node, ms, "node finished");
            env.emit(&thread, ProgressEvent::NodeFinished { node, ms });
            Ok(output)
        }
        Err(e) => {
            let message = redact(&format!("{e:#}")).into_owned();
            tracing::warn!(graph = %env.graph, %thread, %node, ms, error = %message, "node failed");
            env.emit(&thread, ProgressEvent::NodeFailed { node, message: message.clone() });
            Err(adk_graph::GraphError::Other(message))
        }
    }
}
