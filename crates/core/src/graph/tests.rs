//! Graph runtime tests: no network, no LLM; checkpoints in a real SQLite file.

use std::future::ready;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

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
fn a_thread_left_running_by_a_crash_reads_as_failed_and_can_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = db(dir.path());
    let runs = Runs::default();
    let graph = demo(&runs);
    // A run that never finished: the first checkpoint is saved, then the process dies.
    {
        let store = Store::open(&path).unwrap();
        let compiled = graph.compile(&store, &tokio::sync::broadcast::channel(1).0).unwrap();
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
                if name != "target" && !path.ends_with("core/src/graph") {
                    walk(&path, hits);
                }
            } else if name.ends_with(".rs") && std::fs::read_to_string(&path).unwrap().contains("adk_graph") {
                hits.push(path);
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut hits = Vec::new();
    walk(crates, &mut hits);
    assert!(hits.is_empty(), "`adk_graph` may only be named in crates/core/src/graph/: {hits:?}");
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
