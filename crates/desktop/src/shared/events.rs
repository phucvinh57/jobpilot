//! Events the shell pushes to the web app. Names are `<context>:<event>`; payloads are DTOs.
//!
//! Two shared events exist: stored data changed (re-read it), and a graph run moved (discovery,
//! trends, prepare-CV and preference-chat runs all report through `core.graphs`).

use std::sync::Arc;

use jobpilot_core::graph::{Progress, ProgressEvent};
use jobpilot_core::{mail, runtime, Core};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

/// Stored data changed: payload [`DataChanged`].
pub const DATA_CHANGED: &str = "shared:data-changed";
/// A graph run moved: payload [`GraphProgressDto`].
pub const GRAPH_PROGRESS: &str = "shared:graph-progress";

/// Which data changed, so a screen re-reads only what it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Scope {
    Applications,
    Leads,
    Sources,
    Trends,
    Profile,
    Integrations,
    /// Prerequisites (provider, typst, Chrome) may have changed.
    Health,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataChanged {
    pub scope: Scope,
}

/// Send `payload` to the web app. A failure is logged, never fatal (the window may be closing).
pub fn emit<S: Serialize + Clone>(app: &AppHandle, event: &str, payload: S) {
    if let Err(e) = app.emit(event, payload) {
        tracing::warn!("emitting {event} failed: {e:#}");
    }
}

/// Tell the web app that `scope` changed.
pub fn data_changed(app: &AppHandle, scope: Scope) {
    emit(app, DATA_CHANGED, DataChanged { scope });
}

/// One step of a graph run. Labels only: never page, posting or CV text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphProgressDto {
    pub thread: String,
    pub graph: String,
    /// `nodeStarted` / `nodeFinished` / `nodeFailed` / `detail` / `waiting` / `done` / `failed` / `cancelled`.
    pub kind: &'static str,
    pub node: Option<String>,
    /// The detail text, or the failure message of a node.
    pub text: Option<String>,
}

impl From<Progress> for GraphProgressDto {
    fn from(p: Progress) -> Self {
        let (kind, node, text) = match p.event {
            ProgressEvent::NodeStarted { node } => ("nodeStarted", Some(node), None),
            ProgressEvent::NodeFinished { node, .. } => ("nodeFinished", Some(node), None),
            ProgressEvent::NodeFailed { node, message } => ("nodeFailed", Some(node), Some(message)),
            ProgressEvent::Detail { node, text } => ("detail", Some(node), Some(text)),
            ProgressEvent::Waiting(_) => ("waiting", None, None),
            ProgressEvent::Done => ("done", None, None),
            ProgressEvent::Failed => ("failed", None, None),
            ProgressEvent::Cancelled => ("cancelled", None, None),
        };
        Self { thread: p.thread, graph: p.graph, kind, node, text }
    }
}

/// Forward core's change signals to the web app for as long as the app runs: graph progress, and
/// "the Gmail sync touched the data".
pub fn bridge(app: &AppHandle, core: &Arc<Core>) {
    let mut progress = core.graphs.subscribe();
    let handle = app.clone();
    drop(runtime::spawn(async move {
        loop {
            match progress.recv().await {
                Ok(p) => emit(&handle, GRAPH_PROGRESS, GraphProgressDto::from(p)),
                // Fell behind: the next event still tells the screen to re-read.
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        }
    }));

    let mut mail_changes = mail::changes();
    let handle = app.clone();
    drop(runtime::spawn(async move {
        while mail_changes.changed().await.is_ok() {
            data_changed(&handle, Scope::Applications);
            data_changed(&handle, Scope::Leads);
        }
    }));
}

#[cfg(test)]
mod tests {
    use jobpilot_core::graph::{Progress, ProgressEvent};

    use super::GraphProgressDto;

    #[test]
    fn progress_keeps_the_thread_and_names_the_step() {
        let p = Progress {
            thread: "t1".into(),
            graph: "discovery".into(),
            event: ProgressEvent::Detail { node: "fetch".into(), text: "3 of 9".into() },
        };
        let dto = GraphProgressDto::from(p);
        assert_eq!((dto.thread.as_str(), dto.graph.as_str(), dto.kind), ("t1", "discovery", "detail"));
        assert_eq!(dto.text.as_deref(), Some("3 of 9"));

        let done = GraphProgressDto::from(Progress { thread: "t".into(), graph: "g".into(), event: ProgressEvent::Done });
        assert_eq!((done.kind, done.node), ("done", None));
    }
}
