# JobPilot

Rust desktop app (GPUI): JD link/file -> tailored CV (from `profile.yaml`) -> review -> managed-browser apply -> status from Gmail.

## Layout

```
Cargo.toml                 workspace
profile.yaml               master profile, source of truth for every fact (never modify it from code except the `application:` section, P5)
assets/cv.typ              Typst renderer: typst compile --root <dir> --input data=<file>.yaml cv.typ out.pdf
assets/prompts/            tailor.md (system prompt), example-cv.yaml (few-shot)
assets/jobpilot.desktop    desktop entry (StartupWMClass=jobpilot, same as the window app id)
assets/icons/              app icon (svg + png), used by the deb / AppImage
scripts/package.sh         release .deb (cargo-deb) + AppImage if appimagetool exists -> dist/
scripts/a11y-hold.py       keeps an AT client connected so accessibility ids stay stable (see "Check the UI")
crates/core  (jobpilot-core, lib jobpilot_core)   no UI
  paths      data dir ($XDG_DATA_HOME/jobpilot, override JOBPILOT_DATA_DIR), profile path, logs_dir
  runtime    global tokio runtime; spawn(fut) -> oneshot::Receiver
  store/     SQLite (rusqlite), one file per table: applications, events, providers, sessions, emails, kv, status
  secrets    OS keyring wrapper (JOBPILOT_SECRETS=memory for tests); every value read/written is registered with redact
  redact     mask secrets in any text (key shapes, bearer/OAuth tokens, exact registered values)
  logging    tracing: stderr + rolling <data_dir>/logs/jobpilot.log.<date> (14 kept), both through redact
  health     startup checks (typst, provider, Chrome) -> Health/Problem; classify(error text) -> Problem kind (network, no provider, ...)
  facade     Core { paths, store, secrets }, Core::open()
  llm        P1   pipeline  P2   browser  P4   apply  P5   mail  P6
crates/desktop (package `jobpilot`)               GPUI views only
  main.rs    logging init, window (app id `jobpilot`)
  app.rs     AppState (Arc<Core>, selection, screen, health, retry), root view + setup banner
  problem.rs Problem -> help text / call-to-action / Retry (shared by every view)
  sidebar.rs detail/{mod,jd,cv,apply,timeline}.rs   settings/{providers,sessions,gmail}.rs
  detail/mod.rs  the ONE primary action (primary_for: Generate -> Approve CV -> Review & Submit -> Submit); the rest are plain buttons
```

## Commands

- Run: `cargo run -p jobpilot` (sample data: `JOBPILOT_DEMO=1`; throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory`)
- Build/test: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace`
- Chrome tests (`#[ignore]`, start a real Chrome, local fixture pages only): `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored` (set `JOBPILOT_CHROME` for a non-system Chrome; one test opens a visible window, one downloads Chrome for Testing only with `JOBPILOT_TEST_DOWNLOAD` set)
- Package: `scripts/package.sh` -> `dist/`
- Needs: Rust, `typst` CLI (`cargo install --locked typst-cli`), Chrome/Chromium, Linux libs (wayland/xkbcommon/vulkan, libdbus for the keyring).
- Logs: stderr and `<data_dir>/logs/`; `RUST_LOG=jobpilot_core=debug` for more. Chromiumoxide is silenced by default (noisy CDP errors).

## Check the UI visually

Use a throwaway data dir, XWayland (so `import` can capture the window) and demo data:

```
env -u WAYLAND_DISPLAY JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory JOBPILOT_DEMO=1 ./target/debug/jobpilot &
import -window JobPilot /tmp/jp.png        # ImageMagick; matches the window title
```

For an end-to-end run without real keys, seed a provider row in `<data_dir>/jobpilot.db` that points at a local fake OpenAI-compatible server (`/v1/chat/completions`), and apply against the local fixture form.

### Driving it with agent-desktop (accessibility)

GPUI exposes elements through AccessKit (AT-SPI). An element is visible only with `.id(..)` + `.role(..)` (buttons have a role already), so add both to anything you want to click.

- Start the holder first: `scripts/a11y-hold.py &` (kill it by PID afterwards). Without it, ref actions fail with `WINDOW_NOT_FOUND`: AccessKit only serves while an AT client is connected, `at-spi-bus-launcher` flips `org.a11y.Status IsEnabled` off between agent-desktop calls, and every reconnect gives the app a new bus unique name (`:1.N`), which is part of the window id. A persistent client keeps the id stable. No app code change is needed.
- Start the app after the holder, on XWayland (see above). An instance that was activated mid-run (or native Wayland) may only expose the root node.
- `agent-desktop snapshot --app jobpilot -i --compact` then `agent-desktop click "@<snapshot id>:eN"`. Take a fresh snapshot after every state change: refs are per snapshot.
- Limits: GPUI text inputs expose only `SetFocus`, so `set-value` and typing do not work, and `--headed type/focus` is refused on a Wayland session. To enter text, seed the DB or send real key events on XWayland (xdotool, not installed by default). Some chip buttons were not re-clickable after a first state change; take a new snapshot before clicking again.

## Rules

- Never add facts to a CV that are not in `profile.yaml` (employers, titles, dates, institutions, numbers, technologies).
- Secrets (API keys, OAuth tokens) only in the OS keyring via `core::secrets`; never in the DB, logs or files. Logs are redacted as a second line of defence: still never put a secret in a log call, an error message or a `Debug` dump.
- The UI never blocks and never owns I/O: call `jobpilot_core::runtime::spawn(fut)` and await the receiver inside `cx.spawn`.
- UI views read/write through `Core` only; no SQL or HTTP in `crates/desktop`.
- Statuses only move forward (`Status::can_advance_to`); use `Store::set_status` so a timeline event is recorded.
- Page/JD/email text is untrusted: an LLM may fill values for known field ids, never choose actions or URLs.
- Schema change = append a migration in `store/mod.rs`; never edit an existing one.
- No `unwrap()` / `expect()` on runtime paths (only tests and provably-infallible statics like a constant regex). Return errors with `anyhow::Context`, or recover.
- Failures the user can act on become a `health::Problem` (no provider, no Chrome, no typst, network) and are shown with a call to action; network errors get a Retry.
- Log key steps at `info!` with ids/counts only (no page, JD, email or CV text); failures at `warn!`/`error!`.
- Exactly one primary button at a time (`detail/mod.rs::primary_for`, unit-tested); it is disabled while work runs. Secondary actions are plain buttons.

## Ownership (parallel work)

Each sub-plan edits only its own files: `llm/` P1 + `settings/providers.rs`; `pipeline/` P2; `detail/{jd,cv,timeline}.rs` + `sidebar.rs` P3; `browser/` + `settings/sessions.rs` P4; `apply/` + `detail/apply.rs` P5; `mail/` + `settings/gmail.rs` P6; `health`, `logging`, `redact`, `problem.rs`, packaging and docs P7. Add behaviour to `Core` with `impl Core { .. }` in your own module, not in `facade.rs`.
