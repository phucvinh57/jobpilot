# P8 Foundation

Goal: land every change to shared files once, so P9-P13 can run in parallel without conflicts. No user-visible feature except empty screens.

## Changes

0. **Graph runtime** (`core/graph/`) on `adk-graph`, see [graph-libraries.md](graph-libraries.md) and [activity-diagram.md](activity-diagram.md):
   - Dependency: `adk-graph = "=2.3.0"` with default features only (no `sqlite`, no `action-*`). No other `adk-*` crate is added by hand.
   - First task: the spike in graph-libraries.md. Its result is reported before the rest of P8 continues.
   - Facade, the only code that names `adk_graph`:
     - `GraphDef` builder: `node(name, async fn)`, `edge`, `route(from, fn(&State) -> &'static str, targets)`, `parallel(from, [..], join)`, `subgraph`.
     - `run(graph, thread, input)`, `resume(thread, input)`, `cancel(thread)`, `status(thread)`, `waiting() -> Vec<Waiting>` (feeds the "needs you" banner), and a progress stream (`node started / finished / failed`) for the UI.
     - `Outcome::{Done, Waiting(WaitReason), Failed(Problem)}`; `WaitReason::{UserMessage, ConfirmPreferences, Login, Captcha, Blocked}` carries the data the UI needs (source id, page URL).
     - Typed state: each graph has a `serde` struct; the facade converts it to and from the library's JSON channels, so nodes never touch raw JSON.
   - `SqliteCheckpointer`: implements `adk_graph::checkpoint::Checkpointer` over `Store`; table `graph_checkpoints(thread, graph, step, state_json, status, updated_at)` in the migration below. Finished threads are pruned after 30 days.
   - Limits on every graph: per-node timeout, maximum steps, maximum concurrency.
   - Shared node `graph::nodes::challenge` (login / captcha / blocked -> interrupt), used by P10 and P13; the detection itself is P10's `browser/challenge.rs`.
   - Logging: node name, thread and duration at `info!`; never state content.
   - `pipeline::generate` and `apply::flow` are not rewritten. They already keep their progress in `applications.status`, and are called from graph nodes where needed (P12).
1. **Migration** (append in `store/mod.rs`, never edit an existing one):
   - `preferences(id=1, json, updated_at)`: one row, the saved `Preferences` document.
   - `chat_messages(id, thread, role, content, at)`: preference chat history.
   - `sources(id, kind, name, url, purpose, enabled, session_id NULL, last_run_at, last_status)`; `purpose` is `jobs` or `trends`.
   - `leads(id, source_id, url UNIQUE, dedup_key, company, role, location, jd_path, score, score_json, insight_json, state, app_id NULL, found_at)`; `state` is `new | dismissed | promoted`.
   - `trend_items(id, source_id, url UNIQUE, title, published_at, text_path, fetched_at)`.
   - `reports(id, kind, period, title, body_md, citations_json, created_at)`.
   - `applications.insight_json` (new column).
2. **Store files**, one per table: `store/{preferences,chat,sources,leads,trends}.rs` with plain CRUD + tests.
3. **LLM**: add `Llm::chat_json(system, messages: &[Message], schema)` with a default implementation that folds the messages into one user text and calls `complete_json`. Providers need no change; they can override later.
4. **Health**: `Problem::NeedsHuman` (captcha / login / blocked) with help text and a "Open browser" call to action in `problem.rs`.
5. **Navigation**: `Screen::{Queue, Preferences, Trends}` next to `Detail` and `Settings`; sidebar gets three nav entries, each rendering a placeholder view with `.id` + `.role` (for agent-desktop).
6. **Notifications**: move `mail/notify.rs` to `core/notify.rs` (re-exported for mail) so discovery can use it.

## Checks

- `cargo test --workspace`: migration applies on an existing DB (test opens a DB at the previous version, then migrates).
- Graph facade: a test graph with a conditional edge, a parallel pair and an interrupt; state survives reopening the store and resumes; a failing node resumes at that node; `waiting()` lists the paused thread.
- No `adk_graph` import outside `core/graph` (a grep check in the test suite).
- CRUD tests per new store file.
- `chat_json` default implementation tested with `llm::fake`.
- Screenshot: three new nav entries, placeholders render.

Risk: Medium. Size: medium (two PRs: graph runtime, then the rest).
