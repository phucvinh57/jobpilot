//! Graph runtime: flows as graphs of small async steps over a typed state, with interrupts and
//! checkpoints (design: `docs/plans/graph-libraries.md`, `docs/plans/activity-diagram.md`).
//!
//! This module is a facade over the `adk-graph` crate and the only code in the workspace that
//! names it; everything else uses the types below, so the library can be swapped without touching
//! the flows.
//!
//! ```ignore
//! let graph = GraphDef::<Chat>::new("prefs_chat")
//!     .node("ask", |mut s| async move { s.reply = Some("...".into()); Ok(s) })
//!     .wait("wait_user", |s| s.message.is_none().then_some(WaitReason::UserMessage))
//!     .edge(START, "ask")
//!     .edge("ask", "wait_user")
//!     .edge("wait_user", END);
//! let outcome = core.graphs.run(&graph, "prefs", Chat::default()).await?; // Waiting(UserMessage)
//! let outcome = core.graphs.resume(&graph, "prefs", |s| s.message = Some(text)).await?;
//! ```
//!
//! # Model
//!
//! - **State**: one `serde` struct per graph ([`GraphState`]). Nodes get a copy and return the new
//!   value; the facade turns the changed fields into the library's channel updates, so nodes never
//!   see JSON. Fields start as overwrite channels; two parallel nodes must not write the same
//!   field, except fields declared with [`GraphDef::append`], which are lists that nodes may only
//!   grow (a node that edits or shrinks one fails).
//! - **Nodes** run once per step; steps run in order, nodes of one step in parallel. A node is
//!   re-run from the start when a thread is resumed after a failure or a pause, so a node should
//!   be idempotent (a failed step re-runs all of its nodes, also those that had succeeded).
//! - **Interrupts**: a [`GraphDef::wait`] node is a predicate over the state: `Some(reason)`
//!   pauses the thread ([`Outcome::Waiting`]), `None` lets it through. [`Graphs::resume`] applies
//!   the caller's change to the saved state, then runs the waiting node again; it passes once the
//!   change made the predicate `None`. A waiting node cannot change the state itself.
//! - **Separate threads** (P10's one run per source): the library stops a whole run at an
//!   interrupt, so a paused branch holds back the downstream nodes of its siblings in the same
//!   run. Work that must go on independently runs as its own thread: a parent node calls
//!   [`Graphs::run_each`] with one thread id per item.
//! - **Checkpoints** are written after every step to `graph_checkpoints` through
//!   [`SqliteCheckpointer`], so paused and failed threads survive a restart. A failed node keeps
//!   the checkpoint before its step, so [`Graphs::resume`] re-runs that step only.
//! - **Limits** ([`Limits`]): a timeout per node, a maximum number of steps per thread (cycles
//!   count), and a maximum number of nodes running at once.
//!
//! Progress (node started / finished / failed, thread done / waiting / failed) is broadcast by
//! [`Graphs::subscribe`]. Logs carry node, thread and duration only, never state.

mod checkpoint;
mod def;
pub mod nodes;
mod runtime;
mod state;
#[cfg(test)]
mod tests;

pub use checkpoint::SqliteCheckpointer;
pub use def::{GraphDef, Limits};
pub use runtime::{Failure, Graphs, Outcome, Progress, ProgressEvent, ThreadInfo, ThreadStatus, WaitReason, Waiting};
pub use state::GraphState;

/// Entry point of a graph: `edge(START, "first")`.
pub const START: &str = adk_graph::START;
/// Exit of a graph: `edge("last", END)`, or a [`GraphDef::route`] target.
pub const END: &str = adk_graph::END;
