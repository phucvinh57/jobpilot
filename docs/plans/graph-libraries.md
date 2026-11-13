# Graph runtime: library choice

Status: **decided on 2026-11-13: `adk-graph`**. The P8 spike verifies it before P9-P13 build on it. Checked on 2026-11-13 (crates.io metadata and docs.rs pages; no code was compiled).

## What the flows need

From [activity-diagram.md](activity-diagram.md):

1. Nodes as plain async Rust functions (our own `Llm`, browser and store stay in charge).
2. Conditional edges and loops.
3. Parallel branches that join (score + insight, trend sources).
4. Interrupt and resume, for login/captcha, CV review and answer review.
5. Checkpoints in the app's existing SQLite file (`rusqlite`), so a paused run survives a restart.
6. One native binary: the app ships as a `.deb` / AppImage.

## Candidates

| Library | Rust? | Fit |
|---------|-------|-----|
| **Google ADK** (official) | No. Python, Go, TypeScript, Java, Kotlin; the 2.0 graph runtime is in Python, Go and TypeScript | Would need a sidecar process in another language. Breaks need 6. |
| **LangGraph / LangChain SDK** (official) | No. Python and JavaScript | Same: sidecar. Breaks need 6. |
| **`adk-graph`** 2.3.0 (community `adk-rust`, not from Google; Apache-2.0) | Yes | Covers needs 1-4: `node_fn` closures, conditional edges, nodes from one source run in parallel and fan in, `interrupt` + resume by thread id. Public `Checkpointer` trait; built-in memory and SQLite (`sqlx`) checkpointers. About 13k downloads in the last 90 days, 27 releases since December 2025. |
| **`graph-flow`** 0.8.1 (`rs-graph-llm`; MIT) | Yes | Covers 1, 2, 4: `Task` trait, `NextAction::WaitForInput`, step and time limits. Conditional edges are yes/no only; parallel work is a `FanOutTask` inside one node. Storage: memory and Postgres (default feature, must be turned off); SQLite needs our own `SessionStorage`. About 12k recent downloads, 16 releases since June 2025. |
| `langgraph` 0.2.5 (community port; MIT) | Yes | Has `interrupt()` and a SQLite checkpointer, but created in June 2026 with about 1.4k downloads in total. Too young. |
| `rust-langgraph`, `langchainrust` | Yes | Very low use; `langchainrust` brings a whole LLM framework. Not considered further. |
| `rig-core` 0.46 | Yes | Widely used LLM client library (4.3M downloads), but no graph runtime of its own; it points to `graph-flow`. We already have our own `llm/`. |
| Own `core/graph` module | Yes | A few hundred lines for exactly the six needs, no dependency. We maintain it. |

## Decision

**Use `adk-graph`, behind a thin `core/graph` facade, with our own checkpointer on `rusqlite`.**

- It is the only Rust crate that matches the diagrams as drawn (parallel fan-out and join, multi-way edges, interrupts).
- Only `adk-graph` with default features (which pulls `adk-core`). Not the `adk-rust` umbrella, not its model or tool crates: LLM calls keep going through `crates/core/src/llm`, so the rules on secrets, redaction and untrusted text are unchanged.
- **Do not enable its `sqlite` feature.** It brings `sqlx`, a second SQLite binding next to `rusqlite` in the same binary. Implement `Checkpointer` over `Store` instead (one new table).
- The facade (`core/graph`: build, run, resume, events for the UI) is the only code that names `adk_graph`. If the crate stalls or breaks, the fallback is `graph-flow`, then an own runner, without touching P9-P13.

Risks: a young project with frequent major versions (pin the exact version); state is a map of JSON channels, so each graph converts to and from its typed struct at the node boundary.

## Spike (first task of P8, half a day)

Build diagram 2 with fake nodes and confirm, before anything else depends on it:

1. `Checkpointer` can be implemented on `rusqlite` (the trait's methods were not readable from the docs index).
2. Interrupt -> app restart -> resume continues from the saved node.
3. Two sources run in parallel; one interrupted source does not block the other.
4. Cost of `adk-core`: added dependencies, build time, binary size; `cargo clippy` and `scripts/check.sh` still pass.
5. Runs on the app's global tokio runtime (`runtime::spawn`).

If 1, 2 or 3 fails, stop and report to the user before switching; the prepared fallbacks are `graph-flow`, then an own runner.

## Sources

- https://cloud.google.com/blog/topics/developers-practitioners/one-agent-development-kit-five-languages
- https://docs.rs/crate/adk-graph/latest
- https://docs.rs/crate/graph-flow/latest
- https://docs.rs/crate/langgraph/0.2.1
- https://docs.rs/crate/rig/0.36.0/source/ECOSYSTEM.md
- crates.io API for versions, dates, downloads and dependencies
