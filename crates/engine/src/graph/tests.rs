//! Graph runtime tests: no network, no LLM; checkpoints in a real SQLite file.

use std::future::ready;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast::error::RecvError;

use super::nodes::{self, Challenge, ChallengeKind, Challenged};
use super::*;
use crate::health::Problem;
use crate::{runtime, Store};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Flow {
    n: i64,
    log: Vec<String>,
    approved: bool,
    joined: bool,
    challenge: Option<Challenge>,
}

impl Challenged for Flow {
    fn challenge(&self) -> Option<&Challenge> {
        self.challenge.as_ref()
    }
}

/// Which nodes ran, in order.
#[derive(Clone, Default)]
struct Runs(Arc<Mutex<Vec<String>>>);

impl Runs {
    fn count(&self, node: &str) -> usize {
        self.0.lock().unwrap().iter().filter(|n| *n == node).count()
    }
}

/// A node that records its run and pushes its name on the `log` list.
fn step(runs: &Runs, name: &'static str) -> impl Fn(Flow) -> std::future::Ready<Result<Flow>> + Send + Sync {
    let runs = runs.clone();
    move |mut s| {
        runs.0.lock().unwrap().push(name.into());
        s.log.push(name.into());
        ready(Ok(s))
    }
}

/// prepare -> (n > 1 ? fan -> {left, right} -> join -> approve -> finish : short)
fn demo(runs: &Runs) -> GraphDef<Flow> {
    GraphDef::new("demo")
        .append("log")
        .node("prepare", {
            let record = step(runs, "prepare");
            move |mut s: Flow| {
                s.n += 1;
                record(s)
            }
        })
        .route("prepare", |s| if s.n > 1 { "fan" } else { "short" }, &["fan", "short"])
        .node("short", step(runs, "short"))
        .node("fan", step(runs, "fan"))
        .node("left", step(runs, "left"))
        .node("right", step(runs, "right"))
        .node("join", {
            let record = step(runs, "join");
            move |mut s: Flow| {
                s.joined = true;
                record(s)
            }
        })
        .wait("approve", |s| (!s.approved).then_some(WaitReason::ConfirmPreferences))
        .node("finish", step(runs, "finish"))
        .edge(START, "prepare")
        .edge("short", END)
        .parallel("fan", &["left", "right"], "join")
        .edge("join", "approve")
        .edge("approve", "finish")
        .edge("finish", END)
}

fn db(dir: &Path) -> PathBuf {
    dir.join("graph.db")
}

fn store_at(path: &Path) -> Store {
    Store::open(path).unwrap()
}

fn graphs_at(path: &Path) -> Graphs {
    Graphs::new(store_at(path))
}

fn logged(outcome: Outcome<Flow>) -> Vec<String> {
    match outcome {
        Outcome::Done(s) => s.log,
        other => panic!("expected Done, got {other:?}"),
    }
}

#[test]
fn conditional_edge_picks_the_branch() {
    let dir = tempfile::tempdir().unwrap();
    let (graphs, runs) = (graphs_at(&db(dir.path())), Runs::default());
    let graph = demo(&runs);
    runtime::block_on(async {
        let short = graphs.run(&graph, "short", Flow::default()).await.unwrap();
        assert_eq!(logged(short), ["prepare", "short"]);
        let fan = graphs.run(&graph, "fan", Flow { n: 1, ..Flow::default() }).await.unwrap();
        assert_eq!(fan, Outcome::Waiting(WaitReason::ConfirmPreferences));
    });
    assert_eq!(runs.count("short"), 1);
    assert_eq!(runs.count("fan"), 1);
}

#[test]
fn paused_thread_survives_a_restart_and_resumes_at_the_saved_node() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let runs = Runs::default();

    // First "process": run until the interrupt, then drop everything.
    {
        let graphs = graphs_at(&path);
        let out = runtime::block_on(graphs.run(&demo(&runs), "t1", Flow { n: 1, ..Flow::default() })).unwrap();
        assert_eq!(out, Outcome::Waiting(WaitReason::ConfirmPreferences));
        assert_eq!(graphs.waiting().unwrap().len(), 1);
    }

    // Second "process": a new store and runtime state over the same file.
    let graphs = graphs_at(&path);
    let waiting = graphs.waiting().unwrap();
    assert_eq!(waiting.len(), 1);
    assert_eq!((waiting[0].thread.as_str(), waiting[0].graph.as_str()), ("t1", "demo"));
    assert_eq!(waiting[0].reason, WaitReason::ConfirmPreferences);
    let info = graphs.status("t1").unwrap().unwrap();
    assert_eq!(info.status, ThreadStatus::Waiting(WaitReason::ConfirmPreferences));

    // Without an answer it pauses again; with one it finishes.
    let graph = demo(&runs);
    let again = runtime::block_on(graphs.resume(&graph, "t1", |_| {})).unwrap();
    assert_eq!(again, Outcome::Waiting(WaitReason::ConfirmPreferences));
    let done = runtime::block_on(graphs.resume(&graph, "t1", |s| s.approved = true)).unwrap();
    assert_eq!(logged(done), ["prepare", "fan", "left", "right", "join", "finish"]);

    for node in ["prepare", "fan", "left", "right", "join", "finish"] {
        assert_eq!(runs.count(node), 1, "{node} must not run again after the restart");
    }
    assert!(graphs.waiting().unwrap().is_empty());
    assert_eq!(graphs.status("t1").unwrap().unwrap().status, ThreadStatus::Done);
}

#[test]
fn failing_node_resumes_at_that_node() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let (runs, attempts) = (Runs::default(), Arc::new(AtomicUsize::new(0)));
    let graph = GraphDef::<Flow>::new("flaky")
        .append("log")
        .node("before", step(&runs, "before"))
        .node("flaky", {
            let (record, attempts) = (step(&runs, "flaky"), attempts.clone());
            move |s| {
                let first = attempts.fetch_add(1, Ordering::SeqCst) == 0;
                ready(if first { Err(anyhow!("OpenAI: the request timed out")) } else { record(s).into_inner() })
            }
        })
        .node("after", step(&runs, "after"))
        .edge(START, "before")
        .edge("before", "flaky")
        .edge("flaky", "after")
        .edge("after", END);

    runtime::block_on(async {
        let Outcome::Failed(failure) = graphs.run(&graph, "f1", Flow::default()).await.unwrap() else {
            panic!("expected a failure")
        };
        assert_eq!((failure.node.as_str(), failure.problem), ("flaky", Problem::Network));
        let info = graphs.status("f1").unwrap().unwrap();
        assert_eq!(info.status, ThreadStatus::Failed(failure));
        assert!(graphs.waiting().unwrap().is_empty(), "a failure is not a wait");

        let done = graphs.resume(&graph, "f1", |_| {}).await.unwrap();
        assert_eq!(logged(done), ["before", "flaky", "after"]);
    });
    assert_eq!((runs.count("before"), runs.count("flaky"), runs.count("after")), (1, 1, 1));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn a_failure_in_the_first_step_can_be_resumed() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let attempts = Arc::new(AtomicUsize::new(0));
    let graph = GraphDef::<Flow>::new("first")
        .node("a", move |mut s: Flow| {
            s.n += 10;
            ready(if attempts.fetch_add(1, Ordering::SeqCst) == 0 { Err(anyhow!("boom")) } else { Ok(s) })
        })
        .edge(START, "a")
        .edge("a", END);
    runtime::block_on(async {
        let out = graphs.run(&graph, "t", Flow { n: 1, ..Flow::default() }).await.unwrap();
        assert!(matches!(out, Outcome::Failed(f) if f.node == "a" && f.problem == Problem::Other));
        let Outcome::Done(s) = graphs.resume(&graph, "t", |_| {}).await.unwrap() else { panic!("not done") };
        assert_eq!(s.n, 11, "the input survived the failed first step");
    });
}

#[test]
fn thread_rules_run_resume_cancel() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    let graph = demo(&runs);
    let input = Flow { n: 1, ..Flow::default() };
    runtime::block_on(async {
        assert!(graphs.resume(&graph, "none", |_| {}).await.is_err(), "unknown thread");
        graphs.run(&graph, "t", input.clone()).await.unwrap();
        let err = graphs.run(&graph, "t", input.clone()).await.unwrap_err();
        assert!(format!("{err:#}").contains("resume or cancel"), "{err:#}");

        let other = GraphDef::<Flow>::new("other").node("x", step(&runs, "x")).edge(START, "x").edge("x", END);
        assert!(graphs.resume(&other, "t", |_| {}).await.is_err(), "wrong graph");

        graphs.cancel("t").unwrap();
        assert!(graphs.status("t").unwrap().is_none());
        graphs.run(&graph, "t", input.clone()).await.unwrap();
        graphs.resume(&graph, "t", |s| s.approved = true).await.unwrap();
        // A finished thread is replaced by a new run, and resuming it returns the final state.
        assert!(matches!(graphs.resume(&graph, "t", |_| {}).await.unwrap(), Outcome::Done(_)));
        assert!(matches!(graphs.run(&graph, "t", input).await.unwrap(), Outcome::Waiting(_)));
    });
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Source {
    id: i64,
    challenge: Option<Challenge>,
    saved: bool,
}

impl Challenged for Source {
    fn challenge(&self) -> Option<&Challenge> {
        self.challenge.as_ref()
    }
}

/// One source: `search` meets a login wall on source 2; `wait_human` is the shared interrupt.
fn source_run(runs: &Runs) -> GraphDef<Source> {
    let runs = runs.clone();
    GraphDef::new("source_run")
        .node("search", |mut s: Source| {
            if s.id == 2 {
                s.challenge =
                    Some(Challenge { kind: ChallengeKind::Login, source_id: s.id, url: "https://x.test/login".into() });
            }
            ready(Ok(s))
        })
        .wait("wait_human", nodes::challenge::<Source>)
        .node("save", move |mut s: Source| {
            runs.0.lock().unwrap().push(format!("save{}", s.id));
            s.saved = true;
            ready(Ok(s))
        })
        .edge(START, "search")
        .edge("search", "wait_human")
        .edge("wait_human", "save")
        .edge("save", END)
}

#[test]
fn a_source_waiting_for_the_user_does_not_block_the_others() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    let graph = source_run(&runs);
    let jobs = (1..=3).map(|id| (format!("discover:1:{id}"), Source { id, ..Source::default() })).collect();

    runtime::block_on(async {
        // On the global runtime, as the app does it: the future must be `Send`.
        let rx = {
            let (graphs, graph) = (graphs.clone(), graph.clone());
            runtime::spawn(async move { graphs.run_each(&graph, jobs, 2).await })
        };
        let mut results = rx.await.unwrap();
        results.sort_by(|a, b| a.0.cmp(&b.0));
        let outcomes: Vec<_> = results.into_iter().map(|(t, r)| (t, r.unwrap())).collect();
        let login = WaitReason::Login { source_id: 2, url: "https://x.test/login".into() };
        assert!(matches!(&outcomes[0].1, Outcome::Done(s) if s.saved));
        assert_eq!(outcomes[1].1, Outcome::Waiting(login.clone()));
        assert!(matches!(&outcomes[2].1, Outcome::Done(s) if s.saved));
        assert_eq!(runs.count("save1") + runs.count("save3"), 2);
        assert_eq!(runs.count("save2"), 0);

        let waiting = graphs.waiting().unwrap();
        assert_eq!(waiting.len(), 1);
        assert_eq!((waiting[0].thread.as_str(), &waiting[0].reason), ("discover:1:2", &login));

        // "Continue" after the user solved it: the shared node clears once the challenge is gone.
        let done = graphs.resume(&graph, "discover:1:2", |s| s.challenge = None).await.unwrap();
        assert!(matches!(done, Outcome::Done(s) if s.saved));
        assert!(graphs.waiting().unwrap().is_empty());
    });
}

/// What the library does inside one run, documented: an interrupt ends the whole step, so a
/// sibling branch is held back at its next node until the thread resumes. This is why independent
/// work (one source each) runs as separate threads, see `run_each`.
#[test]
fn inside_one_run_a_paused_branch_holds_back_its_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    let graph = GraphDef::<Flow>::new("barrier")
        .append("log")
        .wait("a", |s| (!s.approved).then_some(WaitReason::UserMessage))
        .node("b", step(&runs, "b"))
        .node("b2", step(&runs, "b2"))
        .edge(START, "a")
        .edge(START, "b")
        .edge("b", "b2")
        .edge("a", END)
        .edge("b2", END);
    runtime::block_on(async {
        let out = graphs.run(&graph, "t", Flow::default()).await.unwrap();
        assert_eq!(out, Outcome::Waiting(WaitReason::UserMessage));
        assert_eq!((runs.count("b"), runs.count("b2")), (1, 0), "b ran, its successor waits");
        let done = graphs.resume(&graph, "t", |s| s.approved = true).await.unwrap();
        assert_eq!(logged(done), ["b", "b2"]);
        assert_eq!(runs.count("b"), 1, "b is not run twice");
    });
}

#[test]
fn append_fields_collect_from_parallel_nodes_and_reject_edits() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    let graph = GraphDef::<Flow>::new("lists")
        .append("log")
        .node("x", step(&runs, "x"))
        .node("y", step(&runs, "y"))
        .node("z", |mut s: Flow| {
            s.log.clear();
            ready(Ok(s))
        })
        .parallel(START, &["x", "y"], "z")
        .edge("z", END);
    runtime::block_on(async {
        let out = graphs.run(&graph, "t", Flow { log: vec!["seed".into()], ..Flow::default() }).await.unwrap();
        let Outcome::Failed(f) = out else { panic!("z clears the list: {out:?}") };
        assert_eq!(f.node, "z");
        assert!(f.message.contains("append field `log`"), "{}", f.message);
        // x and y both pushed; the failed step did not lose them.
        let saved = store_at(&db(dir.path())).graph_latest_checkpoint("t").unwrap().unwrap();
        assert!(saved.state_json.contains(r#""log":["seed","x","y"]"#), "{}", saved.state_json);
    });
}

#[test]
fn limits_node_timeout_and_max_steps() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let slow = GraphDef::<Flow>::new("slow")
        .node("sleep", |s: Flow| async move {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(s)
        })
        .edge(START, "sleep")
        .edge("sleep", END)
        .limits(Limits { node_timeout: Duration::from_millis(50), ..Limits::default() });
    let looping = GraphDef::<Flow>::new("loop")
        .node("tick", |mut s: Flow| {
            s.n += 1;
            ready(Ok(s))
        })
        .edge(START, "tick")
        .route("tick", |_| "tick", &["tick"])
        .limits(Limits { max_steps: 5, ..Limits::default() });
    runtime::block_on(async {
        let Outcome::Failed(f) = graphs.run(&slow, "s", Flow::default()).await.unwrap() else { panic!() };
        assert_eq!((f.node.as_str(), f.problem), ("sleep", Problem::Network));
        assert!(f.message.contains("timed out after"), "{}", f.message);

        let Outcome::Failed(f) = graphs.run(&looping, "l", Flow::default()).await.unwrap() else { panic!() };
        assert!(f.message.contains("Recursion limit"), "{}", f.message);
        let saved = store_at(&db(dir.path())).graph_latest_checkpoint("l").unwrap().unwrap();
        assert!(saved.state_json.contains(r#""n":5"#), "five steps ran: {}", saved.state_json);
    });
}

#[test]
fn progress_events_and_runs_on_the_global_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    let graph = demo(&runs);
    let mut events = graphs.subscribe();
    let rx = {
        let graphs = graphs.clone();
        runtime::spawn(async move { graphs.run(&graph, "p", Flow::default()).await })
    };
    let out = runtime::block_on(rx).unwrap().unwrap();
    assert_eq!(logged(out), ["prepare", "short"]);

    let mut seen = Vec::new();
    while let Ok(p) = events.try_recv() {
        assert_eq!((p.thread.as_str(), p.graph.as_str()), ("p", "demo"));
        seen.push(match p.event {
            ProgressEvent::NodeStarted { node } => format!("start {node}"),
            ProgressEvent::NodeFinished { node, .. } => format!("end {node}"),
            other => format!("{other:?}"),
        });
    }
    assert_eq!(seen, ["start prepare", "end prepare", "start short", "end short", "Done"]);
}

#[test]
fn cancel_stops_a_running_thread() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let graph = GraphDef::<Flow>::new("long")
        .node("sleep", |s: Flow| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(s)
        })
        .edge(START, "sleep")
        .edge("sleep", END);
    let mut events = graphs.subscribe();
    let rx = {
        let graphs = graphs.clone();
        runtime::spawn(async move { graphs.run(&graph, "c", Flow::default()).await })
    };
    runtime::block_on(async {
        loop {
            let p = events.recv().await.unwrap();
            if p.event == (ProgressEvent::NodeStarted { node: "sleep".into() }) {
                break;
            }
        }
        assert_eq!(graphs.status("c").unwrap().unwrap().status, ThreadStatus::Running);
        graphs.cancel("c").unwrap();
    });
    assert_eq!(runtime::block_on(rx).unwrap().unwrap(), Outcome::Cancelled);
    assert!(graphs.status("c").unwrap().is_none());
}

#[test]
fn running_lists_the_threads_of_this_process_with_the_node_they_are_at() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    assert!(graphs.running().unwrap().is_empty());
    let graph = GraphDef::<Flow>::new("long")
        .node("first", |s: Flow| ready(Ok(s)))
        .node("sleep", |s: Flow| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(s)
        })
        .edge(START, "first")
        .edge("first", "sleep")
        .edge("sleep", END);
    // A paused thread is not running.
    let paused = GraphDef::<Flow>::new("paused")
        .wait("approve", |s| (!s.approved).then_some(WaitReason::ConfirmPreferences))
        .edge(START, "approve")
        .edge("approve", END);
    let mut events = graphs.subscribe();
    let rx = {
        let graphs = graphs.clone();
        runtime::spawn(async move { graphs.run(&graph, "r", Flow::default()).await })
    };
    runtime::block_on(async {
        assert!(matches!(graphs.run(&paused, "w", Flow::default()).await.unwrap(), Outcome::Waiting(_)));
        loop {
            let p = events.recv().await.unwrap();
            if p.event == (ProgressEvent::NodeStarted { node: "sleep".into() }) {
                break;
            }
        }
        let running = graphs.running().unwrap();
        assert_eq!(running.len(), 1, "{running:?}");
        assert_eq!((running[0].thread.as_str(), running[0].graph.as_str()), ("r", "long"));
        assert_eq!(running[0].status, ThreadStatus::Running);
        assert_eq!(graphs.current_nodes("r").unwrap(), ["sleep"]);
        assert_eq!(graphs.current_nodes("w").unwrap(), ["approve"]);
        assert!(graphs.current_nodes("nope").unwrap().is_empty());
        graphs.cancel("r").unwrap();
    });
    assert_eq!(runtime::block_on(rx).unwrap().unwrap(), Outcome::Cancelled);
    assert!(graphs.running().unwrap().is_empty());
}

#[test]
fn running_is_ordered_by_thread_not_by_the_last_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let long = || {
        GraphDef::<Flow>::new("long")
            .node("first", |s: Flow| ready(Ok(s)))
            .node("sleep", |s: Flow| async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                Ok(s)
            })
            .edge(START, "first")
            .edge("first", "sleep")
            .edge("sleep", END)
    };
    let mut events = graphs.subscribe();
    let mut start = |thread: &'static str| {
        let rx = {
            let graphs = graphs.clone();
            let graph = long();
            runtime::spawn(async move { graphs.run(&graph, thread, Flow::default()).await })
        };
        runtime::block_on(async {
            loop {
                let p = events.recv().await.unwrap();
                if p.thread == thread && p.event == (ProgressEvent::NodeStarted { node: "sleep".into() }) {
                    break;
                }
            }
        });
        rx
    };
    // "b" starts first, so "a" has the newer checkpoint: the list is by thread all the same.
    let b = start("b");
    let a = start("a");
    let threads = |graphs: &Graphs| {
        graphs.running().unwrap().into_iter().map(|i| i.thread).collect::<Vec<_>>()
    };
    assert_eq!(threads(&graphs), ["a", "b"]);
    graphs.cancel("a").unwrap();
    graphs.cancel("b").unwrap();
    assert_eq!(runtime::block_on(a).unwrap().unwrap(), Outcome::Cancelled);
    assert_eq!(runtime::block_on(b).unwrap().unwrap(), Outcome::Cancelled);
}

#[test]
fn a_thread_left_running_by_a_crash_reads_as_failed_and_can_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let runs = Runs::default();
    let graph = demo(&runs);
    // A run that never finished: the first checkpoint is saved, then the process dies.
    {
        let store = Store::open(&path).unwrap();
        let compiled = graph.compile(&store, &tokio::sync::broadcast::channel(1).0, &Default::default()).unwrap();
        let first = adk_graph::Checkpoint::new(
            "crash",
            to_state(&Flow { n: 1, ..Flow::default() }),
            0,
            compiled.get_entry_nodes(),
        );
        runtime::block_on(compiled.checkpointer().unwrap().save(&first)).unwrap();
    }
    let graphs = graphs_at(&path);
    let info = graphs.status("crash").unwrap().unwrap();
    assert!(matches!(info.status, ThreadStatus::Failed(f) if f.message.contains("app closed")));
    let out = runtime::block_on(graphs.resume(&graph, "crash", |_| {})).unwrap();
    assert_eq!(out, Outcome::Waiting(WaitReason::ConfirmPreferences));
    assert_eq!(runs.count("prepare"), 1);
}

fn to_state(flow: &Flow) -> adk_graph::State {
    super::state::to_map(flow).unwrap().into_iter().collect()
}

#[test]
fn finished_threads_expire_after_thirty_days_but_waiting_ones_do_not() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let runs = Runs::default();
    let graphs = graphs_at(&path);
    let graph = demo(&runs);
    runtime::block_on(async {
        graphs.run(&graph, "done", Flow::default()).await.unwrap();
        graphs.run(&graph, "waits", Flow { n: 1, ..Flow::default() }).await.unwrap();
    });
    let store = Store::open(&path).unwrap();
    let later = |days| chrono::Utc::now() + chrono::Duration::days(days);
    assert_eq!(store.graph_prune(later(29) - chrono::Duration::days(30)).unwrap(), 0, "nothing is that old yet");
    assert_eq!(store.graph_prune(later(31)).unwrap(), 1, "only the finished thread");
    assert!(store.graph_thread("done").unwrap().is_none());
    assert!(store.graph_thread("waits").unwrap().is_some());
}

#[test]
fn only_the_newest_checkpoints_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let runs = Runs::default();
    runtime::block_on(graphs.run(&demo(&runs), "t", Flow { n: 1, ..Flow::default() })).unwrap();
    let kept = store_at(&db(dir.path())).graph_checkpoints("t").unwrap();
    assert_eq!(kept.len(), 3, "five steps were saved (prepare, fan, left/right, join, the pause); three kept");
    let steps: Vec<_> = kept.iter().map(|r| r.step).collect();
    assert_eq!(steps, [3, 4, 4], "the pause is saved again at the step it stopped in");
}

#[test]
fn wait_reasons_are_stable_json() {
    let reason = WaitReason::Captcha { source_id: 7, url: "https://x.test".into() };
    let json = serde_json::to_string(&reason).unwrap();
    assert_eq!(json, r#"{"kind":"captcha","source_id":7,"url":"https://x.test"}"#);
    assert_eq!(serde_json::from_str::<WaitReason>(&json).unwrap(), reason);
    assert_eq!(serde_json::to_string(&WaitReason::UserMessage).unwrap(), r#"{"kind":"user_message"}"#);

    let blocked = Challenge { kind: ChallengeKind::Blocked, source_id: 1, url: "u".into() };
    assert_eq!(blocked.wait_reason(), WaitReason::Blocked { source_id: 1, url: "u".into() });
    let state = Flow { challenge: Some(blocked), ..Flow::default() };
    assert!(nodes::challenge(&state).is_some());
    assert!(nodes::challenge(&Flow::default()).is_none());
}

#[test]
fn the_library_is_named_only_inside_the_graph_module() {
    fn walk(dir: &Path, hits: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "target" && !path.ends_with("crates/engine/src/graph") {
                    walk(&path, hits);
                }
            } else if name.ends_with(".rs") && std::fs::read_to_string(&path).unwrap().contains("adk_graph") {
                hits.push(path);
            }
        }
    }
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut hits = Vec::new();
    walk(&crate_dir.join("src"), &mut hits);
    walk(&crate_dir.join("tests"), &mut hits);
    assert!(hits.is_empty(), "`adk_graph` may only be named in crates/engine/src/graph/: {hits:?}");
}

/// The engine never names Tauri or the shell: its manifest declares no Tauri crate and no
/// `jobpilot` (any form: `tauri = ..`, `tauri.workspace = true`, `[dependencies.tauri]`), and its
/// sources name neither `tauri*::` nor the shell's library (`jobpilot_lib`).
#[test]
fn the_engine_does_not_depend_on_the_shell() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
    let dep = regex::Regex::new(r"^\s*\[?\s*([\w-]+\.)*(tauri[\w-]*|jobpilot)\s*[.=\]]").unwrap();
    let declared: Vec<&str> = manifest.lines().filter(|l| dep.is_match(l)).collect();
    assert!(declared.is_empty(), "the engine depends on Tauri or the shell: {declared:?}");

    fn walk(dir: &Path, shell: &regex::Regex, hits: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(&path, shell, hits);
            } else if path.extension().is_some_and(|e| e == "rs") {
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    if shell.is_match(line) {
                        hits.push((path.clone(), line.trim().to_owned()));
                    }
                }
            }
        }
    }
    // Built from parts so this file does not match itself.
    let shell = regex::Regex::new(&format!(r"\b(tauri\w*|{}_lib)::", "jobpilot")).unwrap();
    let mut hits = Vec::new();
    walk(&crate_dir.join("src"), &shell, &mut hits);
    walk(&crate_dir.join("tests"), &shell, &mut hits);
    assert!(hits.is_empty(), "the engine names the shell: {hits:?}");
}

/// The discovery pattern: a parent graph whose node starts one child thread per source.
#[test]
fn a_node_can_fan_out_to_child_threads() {
    #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
    struct Discovery {
        ids: Vec<i64>,
        saved: usize,
        waiting: usize,
    }
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let child = source_run(&Runs::default());
    let parent = GraphDef::<Discovery>::new("discovery")
        .node("sources", {
            let (graphs, child) = (graphs.clone(), child.clone());
            move |mut s: Discovery| {
                let (graphs, child) = (graphs.clone(), child.clone());
                async move {
                    let jobs =
                        s.ids.iter().map(|&id| (format!("d:{id}"), Source { id, ..Source::default() })).collect();
                    for (_, outcome) in graphs.run_each(&child, jobs, 2).await {
                        match outcome? {
                            Outcome::Done(_) => s.saved += 1,
                            Outcome::Waiting(_) => s.waiting += 1,
                            other => return Err(anyhow!("unexpected {other:?}")),
                        }
                    }
                    Ok(s)
                }
            }
        })
        .edge(START, "sources")
        .edge("sources", END);
    let input = Discovery { ids: vec![1, 2, 3], ..Discovery::default() };
    let rx = {
        let graphs = graphs.clone();
        runtime::spawn(async move { graphs.run(&parent, "discover:1", input).await })
    };
    let Outcome::Done(summary) = runtime::block_on(rx).unwrap().unwrap() else { panic!("parent not done") };
    assert_eq!((summary.saved, summary.waiting), (2, 1), "done when every source is done or waiting");
    let waiting = graphs.waiting().unwrap();
    assert_eq!(waiting.len(), 1, "the paused source is its own thread");
    assert_eq!(waiting[0].thread, "d:2");
}

#[test]
fn a_cancelled_thread_id_can_be_run_again_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let long = GraphDef::<Flow>::new("long")
        .node("sleep", |s: Flow| async move {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(s)
        })
        .edge(START, "sleep")
        .edge("sleep", END);
    let quick = GraphDef::<Flow>::new("quick").node("x", step(&Runs::default(), "x")).edge(START, "x").edge("x", END);
    runtime::block_on(async {
        // Polled by hand, so the cancelled run is not woken before the thread id is used again.
        let mut old = Box::pin(graphs.run(&long, "c", Flow::default()));
        assert!(futures::poll!(old.as_mut()).is_pending(), "the node is running");
        graphs.cancel("c").unwrap();
        // No "already running", although the cancelled run has not been polled again yet.
        let mut newer = Box::pin(graphs.run(&long, "c", Flow::default()));
        assert!(futures::poll!(newer.as_mut()).is_pending());

        // The old run winds down now; it must leave the new run's entry and rows alone.
        assert_eq!(old.await.unwrap(), Outcome::Cancelled);
        assert_eq!(graphs.status("c").unwrap().unwrap().status, ThreadStatus::Running);
        assert!(graphs.run(&quick, "c", Flow::default()).await.is_err(), "the new run still owns the thread");

        graphs.cancel("c").unwrap();
        assert_eq!(newer.await.unwrap(), Outcome::Cancelled);
        assert!(graphs.status("c").unwrap().is_none());
        assert_eq!(logged(graphs.run(&quick, "c", Flow::default()).await.unwrap()), ["x"]);
    });
}

#[test]
fn cancel_reaches_work_a_node_handed_to_its_own_task() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let graph = GraphDef::<Flow>::new("detached")
        .node("spawn", {
            let stopped = stopped.clone();
            move |s: Flow| {
                let (token, stopped) = (cancel_token(), stopped.clone());
                async move {
                    // Work that outlives the node's future, like a browser step on its own task.
                    runtime::spawn(async move {
                        token.cancelled().await;
                        stopped.store(true, Ordering::SeqCst);
                    });
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    Ok(s)
                }
            }
        })
        .edge(START, "spawn")
        .edge("spawn", END);
    let mut events = graphs.subscribe();
    let run = {
        let graphs = graphs.clone();
        runtime::spawn(async move { graphs.run(&graph, "d", Flow::default()).await })
    };
    runtime::block_on(async {
        while events.recv().await.unwrap().event != (ProgressEvent::NodeStarted { node: "spawn".into() }) {}
        assert!(!stopped.load(Ordering::SeqCst));
        graphs.cancel("d").unwrap();
        for _ in 0..100 {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    assert!(stopped.load(Ordering::SeqCst), "the spawned work saw the cancellation");
    assert_eq!(runtime::block_on(run).unwrap().unwrap(), Outcome::Cancelled);
    assert!(!cancel_token().is_cancelled(), "outside a run the token never fires");
}

#[test]
fn a_cancelled_run_writes_nothing_back() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_at(&db(dir.path()));
    let token = tokio_util::sync::CancellationToken::new();
    let checkpointer = SqliteCheckpointer::new(store.clone(), "g").with_cancel(token.clone());
    let save = |n: usize| {
        let cp = adk_graph::Checkpoint::new("t", adk_graph::State::new(), n, vec![]);
        runtime::block_on(adk_graph::Checkpointer::save(&checkpointer, &cp))
    };
    save(0).unwrap();
    token.cancel();
    store.graph_delete_thread("t").unwrap();
    assert!(save(1).is_err(), "a cancelled thread is not written back");
    assert!(store.graph_thread("t").unwrap().is_none());
}

#[test]
fn cancelling_in_the_middle_of_a_busy_run_leaves_no_thread_behind() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    // Many short steps that yield, so cancel lands at different points of the run.
    let busy = GraphDef::<Flow>::new("busy")
        .node("tick", |mut s: Flow| async move {
            tokio::task::yield_now().await;
            s.n += 1;
            Ok(s)
        })
        .edge(START, "tick")
        .route("tick", |s| if s.n < 200 { "tick" } else { END }, &["tick", END]);
    runtime::block_on(async {
        for i in 0..30u64 {
            let thread = format!("busy{i}");
            let mut events = graphs.subscribe();
            let run = {
                let (graphs, busy, thread) = (graphs.clone(), busy.clone(), thread.clone());
                runtime::spawn(async move { graphs.run(&busy, &thread, Flow::default()).await })
            };
            // Cancel only once the run has started: a cancel that comes before the spawned task
            // reaches `run` finds no such thread and does nothing, and the run that starts after it
            // is a new one that rightly ends `Done` with its row kept. Events are sent only from a
            // started run; falling behind (`Lagged`) means some were sent, and they are this
            // thread's: the previous run was awaited before this subscription.
            loop {
                match events.recv().await {
                    Ok(p) if p.thread == thread => break,
                    Ok(_) => {}
                    Err(RecvError::Lagged(_)) => break,
                    Err(e) => panic!("progress stream closed: {e}"),
                }
            }
            tokio::time::sleep(Duration::from_micros(i * 150)).await;
            graphs.cancel(&thread).unwrap();
            let outcome = run.await.unwrap().unwrap();
            assert!(matches!(outcome, Outcome::Cancelled | Outcome::Done(_)), "{outcome:?}");
            let left = graphs.status(&thread).unwrap();
            assert!(left.is_none(), "no rows left for {thread} after {outcome:?}: {left:?}");
        }
    });
}

#[test]
fn unreadable_wait_reasons_are_listed_and_can_be_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let graphs = graphs_at(&path);
    let store = store_at(&path);
    let rows = [("garbled", "not json"), ("future", r#"{"kind":"from_a_newer_version"}"#)];
    for (thread, detail) in rows {
        let row = crate::store::GraphCheckpointRow {
            id: thread.into(),
            thread: thread.into(),
            graph: "demo".into(),
            step: 1,
            state_json: "{}".into(),
            rest_json: r#"{"pending_nodes":[]}"#.into(),
            created_at: chrono::Utc::now(),
        };
        assert!(store.graph_save_checkpoint(&row, 3, || true).unwrap());
        store.graph_set_status(thread, crate::store::GRAPH_WAITING, Some(detail), || true).unwrap();
    }
    let waiting = graphs.waiting().unwrap();
    assert_eq!(waiting.len(), 2, "neither thread is dropped");
    assert!(waiting.iter().all(|w| w.reason == WaitReason::Unknown));
    for w in &waiting {
        let info = graphs.status(&w.thread).unwrap().unwrap();
        assert_eq!(info.status, ThreadStatus::Waiting(WaitReason::Unknown), "status agrees with waiting()");
        graphs.cancel(&w.thread).unwrap();
    }
    assert!(graphs.waiting().unwrap().is_empty());
}

#[test]
fn old_threads_are_pruned_in_the_background_not_when_the_runtime_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let store = store_at(&path);
    let old = chrono::Utc::now() - chrono::Duration::days(40);
    let row = crate::store::GraphCheckpointRow {
        id: "old-1".into(),
        thread: "old".into(),
        graph: "demo".into(),
        step: 1,
        state_json: "{}".into(),
        rest_json: r#"{"pending_nodes":[]}"#.into(),
        created_at: old,
    };
    assert!(store.graph_save_checkpoint(&row, 3, || true).unwrap());
    store.graph_set_status("old", crate::store::GRAPH_DONE, None, || true).unwrap();
    store.with_conn(|c| Ok(c.execute("UPDATE graph_checkpoints SET updated_at = ?1", [old])?)).unwrap();

    let graphs = Graphs::new(store.clone());
    assert!(store.graph_thread("old").unwrap().is_some(), "creating the runtime does no database work");

    // The first run starts the prune in the background.
    let runs = Runs::default();
    runtime::block_on(async {
        graphs.run(&demo(&runs), "new", Flow::default()).await.unwrap();
        for _ in 0..100 {
            if store.graph_thread("old").unwrap().is_none() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    assert!(store.graph_thread("old").unwrap().is_none());
    assert!(store.graph_thread("new").unwrap().is_some());
    assert_eq!(graphs.prune().unwrap(), 0, "and `prune` can be called by hand");
}

/// A field that serializes only when set and has no `Option` type: clearing it must not break the
/// state (a cleared channel is stored as null).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Notes {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    notes: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    extra: Option<String>,
    approved: bool,
    cleared_by_node: bool,
}

#[test]
fn a_field_cleared_by_a_resume_or_a_node_reads_back_as_its_default() {
    let dir = tempfile::tempdir().unwrap();
    let graphs = graphs_at(&db(dir.path()));
    let graph = GraphDef::<Notes>::new("notes")
        .wait("ask", |s| (!s.approved).then_some(WaitReason::UserMessage))
        .node("clear", |mut s: Notes| {
            s.notes.clear();
            s.extra = None;
            s.cleared_by_node = true;
            ready(Ok(s))
        })
        .node("check", |s: Notes| {
            ready(if s.notes.is_empty() && s.cleared_by_node { Ok(s) } else { Err(anyhow!("bad")) })
        })
        .edge(START, "ask")
        .edge("ask", "clear")
        .edge("clear", "check")
        .edge("check", END);
    let input = Notes { notes: vec!["a".into()], title: "t".into(), extra: Some("x".into()), ..Notes::default() };
    runtime::block_on(async {
        assert_eq!(graphs.run(&graph, "n", input.clone()).await.unwrap(), Outcome::Waiting(WaitReason::UserMessage));
        // The resume clears two fields; the node afterwards clears the rest.
        let done = graphs
            .resume(&graph, "n", |s| {
                s.approved = true;
                s.title.clear();
            })
            .await
            .unwrap();
        let Outcome::Done(s) = done else { panic!("expected Done, got {done:?}") };
        assert_eq!((s.notes.len(), s.title.as_str(), s.extra.as_deref()), (0, "", None));

        // Clearing during the resume itself, with the node reading the state straight away.
        graphs.cancel("n").unwrap();
        graphs.run(&graph, "n", input).await.unwrap();
        let done = graphs
            .resume(&graph, "n", |s| {
                s.approved = true;
                s.notes.clear();
            })
            .await
            .unwrap();
        assert!(matches!(done, Outcome::Done(s) if s.cleared_by_node));
    });
}
