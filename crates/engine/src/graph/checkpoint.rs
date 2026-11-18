//! [`SqliteCheckpointer`]: the library's `Checkpointer` over the app's `rusqlite` [`Store`].

use std::collections::HashMap;

use adk_graph::{Checkpoint, Checkpointer, GraphError};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::store::{GraphCheckpointRow, Store};

/// How many checkpoints of a thread are kept (the newest is the one a resume loads).
const KEEP: usize = 3;

/// Saves graph checkpoints in the `graph_checkpoints` table, tagged with the graph's name.
///
/// Store calls run inline, not through `spawn_blocking`: they are single short statements on a
/// local WAL database, like every other store call made from async code.
///
/// Once its [cancellation token](Self::with_cancel) is set, `save` is refused (inside the
/// transaction, see [`Store::graph_save_checkpoint`]), so a cancelled thread cannot be written
/// back after it was deleted.
#[derive(Clone)]
pub struct SqliteCheckpointer {
    store: Store,
    graph: String,
    cancel: CancellationToken,
}

/// The parts of a library checkpoint besides the state, kept as one JSON column.
#[derive(Serialize, Deserialize)]
struct Rest {
    pending_nodes: Vec<String>,
    #[serde(default)]
    metadata: HashMap<String, Value>,
    #[serde(default)]
    cleared_interrupt: Option<String>,
    #[serde(default)]
    attempts: HashMap<String, u32>,
    #[serde(default)]
    child_ledger: HashMap<String, Value>,
}

impl SqliteCheckpointer {
    /// A checkpointer for the graph called `graph`.
    pub fn new(store: Store, graph: impl Into<String>) -> Self {
        Self { store, graph: graph.into(), cancel: CancellationToken::new() }
    }

    /// Refuse saves once `token` is cancelled.
    pub fn with_cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = token;
        self
    }

    fn to_row(&self, cp: &Checkpoint) -> Result<GraphCheckpointRow> {
        let rest = Rest {
            pending_nodes: cp.pending_nodes.clone(),
            metadata: cp.metadata.clone(),
            cleared_interrupt: cp.cleared_interrupt.clone(),
            attempts: cp.attempts.clone(),
            child_ledger: cp.child_ledger.clone(),
        };
        Ok(GraphCheckpointRow {
            id: cp.checkpoint_id.clone(),
            thread: cp.thread_id.clone(),
            graph: self.graph.clone(),
            step: i64::try_from(cp.step).context("step out of range")?,
            state_json: serde_json::to_string(&cp.state)?,
            rest_json: serde_json::to_string(&rest)?,
            created_at: cp.created_at,
        })
    }
}

fn from_row(row: GraphCheckpointRow) -> Result<Checkpoint> {
    let rest: Rest = serde_json::from_str(&row.rest_json)?;
    Ok(Checkpoint {
        thread_id: row.thread,
        checkpoint_id: row.id,
        state: serde_json::from_str(&row.state_json)?,
        step: usize::try_from(row.step).context("step out of range")?,
        pending_nodes: rest.pending_nodes,
        metadata: rest.metadata,
        created_at: row.created_at,
        cleared_interrupt: rest.cleared_interrupt,
        attempts: rest.attempts,
        child_ledger: rest.child_ledger,
    })
}

/// The nodes a saved checkpoint goes on with (the ones running after it was saved), from its
/// `rest_json` alone: the state is not needed, so it is not read.
pub(crate) fn pending_nodes(rest_json: &str) -> Result<Vec<String>> {
    /// Only the field asked for: the other parts of [`Rest`] are skipped, not built.
    #[derive(Deserialize)]
    struct Pending {
        pending_nodes: Vec<String>,
    }
    let rest: Pending = serde_json::from_str(rest_json).context("reading the saved checkpoint")?;
    Ok(rest.pending_nodes)
}

fn failed(e: anyhow::Error) -> GraphError {
    GraphError::CheckpointError(format!("{e:#}"))
}

#[async_trait]
impl Checkpointer for SqliteCheckpointer {
    async fn save(&self, checkpoint: &Checkpoint) -> adk_graph::Result<String> {
        let row = self.to_row(checkpoint).map_err(failed)?;
        if !self.store.graph_save_checkpoint(&row, KEEP, || !self.cancel.is_cancelled()).map_err(failed)? {
            return Err(GraphError::CheckpointError("the thread was cancelled".into()));
        }
        Ok(row.id)
    }

    async fn load(&self, thread_id: &str) -> adk_graph::Result<Option<Checkpoint>> {
        let row = self.store.graph_latest_checkpoint(thread_id).map_err(failed)?;
        row.map(from_row).transpose().map_err(failed)
    }

    async fn load_by_id(&self, checkpoint_id: &str) -> adk_graph::Result<Option<Checkpoint>> {
        let row = self.store.graph_checkpoint_by_id(checkpoint_id).map_err(failed)?;
        row.map(from_row).transpose().map_err(failed)
    }

    async fn list(&self, thread_id: &str) -> adk_graph::Result<Vec<Checkpoint>> {
        let rows = self.store.graph_checkpoints(thread_id).map_err(failed)?;
        rows.into_iter().map(from_row).collect::<Result<_>>().map_err(failed)
    }

    async fn delete(&self, thread_id: &str) -> adk_graph::Result<()> {
        self.store.graph_delete_thread(thread_id).map_err(failed)
    }
}
