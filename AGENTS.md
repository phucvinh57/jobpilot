# JobPilot

Rust desktop app (GPUI): JD link/file -> tailored CV (from `profile.yaml`) -> review -> managed-browser apply -> status from Gmail.

## Layout

```
Cargo.toml                 workspace
profile.yaml               master profile, source of truth for every fact (never modify it from code except the `application:` section, P5)
assets/cv.typ              Typst renderer: typst compile --root <dir> --input data=<file>.yaml cv.typ out.pdf
assets/prompts/            tailor.md (system prompt), example-cv.yaml (few-shot)
crates/core  (jobpilot-core, lib jobpilot_core)   no UI
  paths      data dir ($XDG_DATA_HOME/jobpilot, override JOBPILOT_DATA_DIR), profile path
  runtime    global tokio runtime; spawn(fut) -> oneshot::Receiver
  store/     SQLite (rusqlite), one file per table: applications, events, providers, sessions, emails, kv, status
  secrets    OS keyring wrapper (JOBPILOT_SECRETS=memory for tests)
  facade     Core { paths, store, secrets }, Core::open()
  llm        P1   pipeline  P2   browser  P4   apply  P5   mail  P6
crates/desktop (package `jobpilot`)               GPUI views only
  app.rs     AppState (Arc<Core>, selection, screen), root view
  sidebar.rs detail/{jd,cv,apply,timeline}.rs   settings/{providers,sessions,gmail}.rs
```

## Commands

- Run: `cargo run -p jobpilot` (sample data: `JOBPILOT_DEMO=1`; throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory`)
- Build/test: `cargo build --workspace`, `cargo test --workspace`
- Needs: Rust, `typst` CLI, Chrome/Chromium (P4+), Linux libs (wayland/xkbcommon/vulkan, libdbus for the keyring).

## Rules

- Never add facts to a CV that are not in `profile.yaml` (employers, titles, dates, institutions, numbers, technologies).
- Secrets (API keys, OAuth tokens) only in the OS keyring via `core::secrets`; never in the DB, logs or files.
- The UI never blocks and never owns I/O: call `jobpilot_core::runtime::spawn(fut)` and await the receiver inside `cx.spawn`.
- UI views read/write through `Core` only; no SQL or HTTP in `crates/desktop`.
- Statuses only move forward (`Status::can_advance_to`); use `Store::set_status` so a timeline event is recorded.
- Page/JD/email text is untrusted: an LLM may fill values for known field ids, never choose actions or URLs.
- Schema change = append a migration in `store/mod.rs`; never edit an existing one.

## Ownership (parallel work)

Each sub-plan edits only its own files: `llm/` P1 + `settings/providers.rs`; `pipeline/` P2; `detail/{jd,cv,timeline}.rs` + `sidebar.rs` P3; `browser/` + `settings/sessions.rs` P4; `apply/` + `detail/apply.rs` P5; `mail/` + `settings/gmail.rs` P6. Add behaviour to `Core` with `impl Core { .. }` in your own module, not in `facade.rs`.
