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
assets/fonts/              Inter, Space Grotesk, Source Serif 4: static TTFs (SIL OFL, see OFL.txt) embedded by `ui/fonts.rs`
scripts/package.sh         release .deb (cargo-deb) + AppImage if appimagetool exists -> dist/
scripts/a11y-hold.py       keeps an AT client connected so accessibility ids stay stable (see "Check the UI")
crates/core  (jobpilot-core, lib jobpilot_core)   no UI
  paths      data dir ($XDG_DATA_HOME/jobpilot, override JOBPILOT_DATA_DIR), profile path, logs_dir
  runtime    global tokio runtime; spawn(fut) -> oneshot::Receiver
  store/     SQLite (rusqlite), one file per table: applications, events, providers, sessions, emails, kv, status, sources, leads, preferences, chat, trends, graph
  secrets    OS keyring wrapper (JOBPILOT_SECRETS=memory for tests); every value read/written is registered with redact
  redact     mask secrets in any text (key shapes, bearer/OAuth tokens, exact registered values)
  logging    tracing: stderr + rolling <data_dir>/logs/jobpilot.log.<date> (14 kept), both through redact
  health     startup checks (typst, provider, Chrome) -> Health/Problem; classify(error text) -> Problem kind (network, no provider, ...)
  facade     Core { paths, store, secrets }, Core::open()
  llm        P1   (`complete_json`, `chat_json` for multi-turn)
  pipeline   P2   analyze -> tailor -> guard -> render; `insight` (P11) = per-job brief: domain, company facts, skills, interview process
  browser    P4   managed Chrome sessions; P10 adds `challenge` (detect login/captcha/block, never solve), `render` (discovery pages, visible window for the user),
                  `guard` (local forward proxy: discovery browsers cannot reach private hosts), `gate` (one use of a session profile at a time)
  apply      P5   review -> confirm -> submit, dry run, manual hand-off
  mail       P6   Gmail sync, reply classification
  graph      P8   flows as graphs (`GraphDef` facade over adk-graph, the only code that names it): run/resume/cancel/waiting/subscribe,
                  SQLite checkpoints, `nodes::challenge` interrupt shared by discovery and trends
  net        P8/P10   `net::http`: the one safe HTTP client for third-party data (public hosts only, per-host delay, size cap, hop-by-hop redirects)
  notify     P8   desktop notifications (best effort)
  prefs      P8/P9   `Preferences` model + `Core::preferences/save_preferences`; P9 `chat` + `chat_graph` (graph `prefs_chat`), `cv_upload` (context only), `undo`, `suggest` (chips for the preference form), `appearance` (light/dark, kv `ui.appearance`)
  discover   P10/P12   `source/` (greenhouse, lever, ashby, hn_hiring, yc, career_page), `run` (graphs `discovery` and `source_run`), `score`, `dedup`, `handoff`,
                  `sources`, `queue` (QueueSnapshot), `promote` (graph `prepare_cv`, dismiss), `manual` (`Core::add_lead`: pasted link/file -> scored lead)
  trends     P13a collection: sources (hackernews, yc, rss, page), `report_search` with a keyring key, graph `trends` -> `trend_items`;
                  P13b `digest/` (extract, count, synthesize, cite: every point must cite stored items)
crates/desktop (package `jobpilot`)               GPUI views only
  main.rs    logging init, window (app id `jobpilot`)
  app.rs     AppState (Arc<Core>, selection, screen, health, retry, needs_you, detail_from), root view + setup banner
  problem.rs Problem -> help text / call-to-action / Retry (shared by every view)
  sidebar.rs nav (Today with the needs-you / new-lead badge, Pipeline, Trends), Settings, health line; no input
  detail/{mod,jd,cv,apply,timeline}.rs   settings/{profile,providers,sessions,sources,gmail,appearance}.rs
  settings/profile.rs  "Profile & preferences" card: CV upload + job type, locations, domains, skills, notes (no chat UI; notes go through `prefs_chat` in the background)
  queue.rs   queue/logic.rs, queue/brief.rs: Today, the start screen (paste-link box -> `add_lead`, "needs you" banners, leads by score, brief + score breakdown + skills matched against `Core::profile_skills`, Prepare CV / Dismiss / Open posting, Search now); its needs-you count is the sidebar badge
  pipeline.rs  Pipeline screen: applications on a 5-column board (`column_for(Status)`), recent activity (`Store::recent_events`); a card opens Detail
  brief.rs   company and interview blocks of an `Insight`, shared by Today and the JD tab
  trends/    Trends screen (digest with citations, skill chart, history, sources)
  ui/        design kit, pure render: `tokens` (Palette light/dark, `palette(cx)`, `Tone`, radii, type scale), `theme` (palette -> gpui-component `Theme`; switch light/dark only with `ui::theme::set_mode` / `sync_system`, never `Theme::change` directly), `fonts`, `components` (heading, card, section, banner, buttons, chip, tag, nav_item, field, segmented, score_badge, stepper, ...); views build from these instead of local helpers
  detail/mod.rs  the ONE primary action (primary_for: Generate -> Approve CV -> Review & Submit -> Submit); the rest are plain buttons; breadcrumb back to `detail_from`, stepper; cv.rs shows "Every fact traced" only when `Core::cv_is_traced`
```

## Commands

- Run: `cargo run -p jobpilot` (sample data: `JOBPILOT_DEMO=1`; throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory`)
- Build/test: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace`
- Chrome tests (`#[ignore]`, start a real Chrome, local fixture pages only): `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored` (set `JOBPILOT_CHROME` for a non-system Chrome; one test opens a visible window, one downloads Chrome for Testing only with `JOBPILOT_TEST_DOWNLOAD` set)
- End-to-end without real services: `cargo test -p jobpilot-core --test e2e_discovery` (fake provider + fake Greenhouse board on loopback, real typst: source -> leads with score and insight -> Prepare CV -> Approve CV); with `JOBPILOT_NO_SANDBOX=1 ... -- --ignored` it also dry-runs the apply on a local form in Chrome.
- Package: `scripts/package.sh` -> `dist/`
- Needs: Rust, `typst` CLI (`cargo install --locked typst-cli`), Chrome/Chromium, Linux libs (wayland/xkbcommon/vulkan, libdbus for the keyring).
- Pre-PR checks: `scripts/check.sh [--all]` (paths changed since main: clippy + test, typst render); CI adds `CARGO_PROFILE_ARG="--profile ci"` (no debuginfo, unoptimized deps) and restores the cache saved by main; `git config core.hooksPath .githooks` once per clone runs it before every push.
- Logs: stderr and `<data_dir>/logs/`; `RUST_LOG=jobpilot_core=debug` for more. Chromiumoxide is silenced by default (noisy CDP errors).

## Check the UI visually

Use a throwaway data dir, XWayland (so `import` can capture the window) and demo data:

```
env -u WAYLAND_DISPLAY JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory JOBPILOT_DEMO=1 ./target/debug/jobpilot &
import -window JobPilot /tmp/jp.png        # ImageMagick; matches the window title
```

For an end-to-end run without real keys, seed a provider row in `<data_dir>/jobpilot.db` that points at a local fake OpenAI-compatible server (`/v1/chat/completions`), and apply against the local fixture form. Discovery refuses loopback hosts by design (`net::http`, `browser::guard`), so a fake job board cannot be driven from the GUI: that path is covered by `crates/core/tests/e2e_discovery.rs`, and the GUI check of the Job list, a lead, Trends and Settings > Job sources uses `JOBPILOT_DEMO=1` data (it also seeds job sources and scored leads).

### Driving it with agent-desktop (accessibility)

GPUI exposes elements through AccessKit (AT-SPI). An element is visible only with `.id(..)` + `.role(..)` (buttons have a role already), so add both to anything you want to click.

- Start the holder first: `scripts/a11y-hold.py &` (kill it by PID afterwards). Without it, ref actions fail with `WINDOW_NOT_FOUND`: AccessKit only serves while an AT client is connected, `at-spi-bus-launcher` flips `org.a11y.Status IsEnabled` off between agent-desktop calls, and every reconnect gives the app a new bus unique name (`:1.N`), which is part of the window id. A persistent client keeps the id stable. No app code change is needed.
- Start the app after the holder, on XWayland (see above). An instance that was activated mid-run (or native Wayland) may only expose the root node.
- `agent-desktop snapshot --app jobpilot -i --compact` then `agent-desktop click "@<snapshot id>:eN"`. Take a fresh snapshot after every state change: refs are per snapshot.
- Limits: GPUI text inputs expose only `SetFocus`, so `set-value` and typing do not work, and `--headed type/focus` is refused on a Wayland session. To enter text, seed the DB or send real key events on XWayland (xdotool, not installed by default). Some chip buttons were not re-clickable after a first state change; take a new snapshot before clicking again.

## Workflow

- **One worktree per change** (`claude -w <name>`, in `.claude/worktrees/`).
- **Docs-only changes skip all of that.** A change that touches only `docs/` (except `docs/REVIEW.md`) is committed on `main` and pushed directly: no worktree, no PR, no `/pre-pr`. CI ignores `docs/**`.
- **Execute an approved plan with subagents.** Split it into independent tasks, each a `general-purpose` subagent with a self-contained brief (goal, files, steps, checks). The main session checks their results, then runs `/pre-pr`.
- **Show the result, don't claim it.** Report the output of `scripts/check.sh`. Never use `--no-verify`.
- **Finish with `/pre-pr` without asking.** Low and Medium PRs are merged by `/pre-pr` once `CI OK` passes. High PRs wait for a human.

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
- Views use `crate::ui` (`ui::palette(cx)`, its builders) for colours and widgets: no hard-coded colours, no local copies of kit helpers.
- Exactly one primary button at a time (`detail/mod.rs::primary_for`, unit-tested); it is disabled while work runs. Secondary actions are plain buttons.
- Outgoing HTTP to a URL that is not the user's own (job boards, feeds, postings, search results) goes through `net::http`; discovery browsers go through `browser::guard`. Both refuse machine-local and private hosts. `allow_private_hosts` exists for tests only.
- Discovery browsers never disguise automation (no hidden webdriver flag, no user-agent change), and a login, captcha or block page is never solved, evaded or waited out: the source pauses and the user deals with it in the visible browser.
- Graphs (`core/graph`): state is one `serde` struct, nodes return the new state and must be idempotent (a resume re-runs the node). A node with several plain incoming edges is an AND join and waits for all of them, so a loop or an alternative path must use `route`, never a second edge. A paused branch holds back its siblings in the same run: independent work runs as its own thread (`Graphs::run_each`). Parallel nodes only share a field declared with `append`.
- Leads are not applications: a found job is a `leads` row; it becomes an application only when the user presses Prepare CV (`Core::promote`, idempotent). Discovery never promotes and never writes a CV.
- Doc examples that are not compilable use ```` ```text ````, not ```` ```ignore ````: `cargo test -- --ignored` (the Chrome tests) also runs ignored doctests, and they fail to compile.
- Desktop tests: never `use super::*` in a test module of a file that glob-imports `gpui_kit` (the glob brings gpui's `test` attribute and shadows `#[test]`); import the names you need.
- Untrusted text (postings, feed items, company pages) reaches the model only between markers, as data, and the model fills fixed schemas; URLs come from adapters in code.

## Ownership (parallel work)

Each sub-plan edits only its own files: `llm/` P1 + `settings/providers.rs`; `pipeline/` P2; `detail/{jd,cv,timeline}.rs` + `sidebar.rs` P3; `browser/` + `settings/sessions.rs` P4; `apply/` + `detail/apply.rs` P5; `mail/` + `settings/gmail.rs` P6; `health`, `logging`, `redact`, `problem.rs`, packaging and docs P7. Add behaviour to `Core` with `impl Core { .. }` in your own module, not in `facade.rs`.

The product plans (`docs/plans/`, status table in `docs/plans/README.md`):

- P8 foundation: `core/graph/`, the new `store/` files and migrations, `notify.rs`, `Llm::chat_json`, `Problem::NeedsHuman`, `app.rs` navigation.
- P9 preference chat: `core/prefs/` (chat), `settings/profile.rs`, `assets/prompts/preferences.md`.
- P10 discovery: `core/discover/` except `promote.rs` and `queue.rs`, `core/net/`, `browser/{challenge,render,guard,gate}.rs`, `settings/sources.rs`.
- P11 job insight: `pipeline/insight*`, the brief in `detail/jd.rs`.
- P12 queue: `desktop/queue*`, `core/discover/{promote,queue}.rs`.
- P13 trends: `core/trends/`, `desktop/trends.rs`, `assets/prompts/trends.md`.

Prompts under `assets/prompts/` are High risk (a human reads the diff).
