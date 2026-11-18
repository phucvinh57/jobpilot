# JobPilot

Rust desktop app (Tauri v2 shell + React web UI): JD link/file -> tailored CV (from the profile read from the user's documents) -> review -> apply in a tab of the user's own Chrome (JobPilot extension) -> status from Gmail.

## Layout

```
Cargo.toml                 workspace
assets/cv.typ              Typst renderer: typst compile --root <dir> --input data=<file>.yaml cv.typ out.pdf
assets/trends.typ          market-insight report -> PDF (`engine::trends::pdf`): typst compile --root <dir> --input data=<file>.json trends.typ out.pdf
assets/prompts/            tailor.md (CV system prompt), example-cv.yaml (few-shot), profile.md (documents -> profile), preferences.md, trends.md
assets/jobpilot.desktop    desktop entry (StartupWMClass=jobpilot, same as the window app id)
assets/icons/              app icon (SVG, PNG, ICO), used by the desktop packages and the extension
extension/                 the JobPilot Chrome extension (MV3, TypeScript, `bun run ext:build` -> extension/dist): an allowlisted relay
                           between the app and the user's Chrome; `PROTOCOL.md` is the contract with `engine::browser::bridge` (change both together)
assets/fonts/              Inter, Space Grotesk, Source Serif 4: static TTFs (SIL OFL, see OFL.txt) bundled by the web app (`src/styles.css`)
scripts/package.sh         release .deb + AppImage on Linux, .app (zip) + .dmg on macOS (Tauri bundler) -> release/
crates/engine/  (package `jobpilot-engine`, lib `jobpilot_engine`)   no UI, no Tauri dependency; the shell's contexts call it, never the reverse.
             `src/` the modules below (`lib.rs` = the module index); `tests/` integration tests (`e2e_discovery`) and `fixtures/profile.yaml`;
             features `test-util` (test profile helpers for the shell's unit tests) and `test-browser` (the bridge's test kit for `tests/`)
  paths      OS user data dir (override JOBPILOT_DATA_DIR), logs_dir
  runtime    global tokio runtime; spawn(fut) -> oneshot::Receiver
  store/     SQLite (rusqlite), one file per table: applications, events, providers, sessions, emails, kv, status, sources, leads, preferences, chat, trends, graph,
             profile (the user's documents + the profile read from them), answers (remembered application answers),
             source_suggestions (suggested job sources: new / added / dismissed)
  secrets    OS keyring wrapper (JOBPILOT_SECRETS=memory for tests); every value read/written is registered with redact
  redact     mask secrets in any text (key shapes, bearer/OAuth tokens, exact registered values)
  logging    tracing: stderr + rolling <data_dir>/logs/jobpilot.log.<date> (14 kept), both through redact
  health     startup checks (typst, provider, browser extension connected) -> Health/Problem; classify(error text) -> Problem kind (network, no provider, ...)
  facade     Core { paths, store, secrets }, Core::open()
  llm        AI providers (OpenAI / Anthropic and compatible endpoints, Codex via a ChatGPT sign-in or the Codex CLI login: `llm::codex`): `complete_json`, `chat_json` (multi-turn); the JSON mode is learned per provider (schema -> json_object -> prompt), never a setting
  pipeline   analyze -> tailor -> guard -> render; `insight`: per-job brief (domain, company facts, skills, interview process)
  browser    the user's own Chrome, driven through the JobPilot extension: `bridge` (WebSocket server on 127.0.0.1:47615, Origin-checked,
             pairing token in the keyring; `Core::browser()`), `tab` (`Tab`: navigate, eval, content, click, type, keys, files, screenshots;
             `Profile::Private` = incognito JobPilot window, `Profile::Signed` = the user's signed-in profile), `session` (sites the user is
             signed in to; no profiles, never touches cookies), `challenge` (detect login/captcha/block, never solve), `render`/`fetch`
             (discovery pages), `guard` (`Policy`: which hosts a guarded tab may reach; every request is checked by the app)
  apply      review -> confirm -> submit, dry run, manual hand-off
  mail       Gmail sync, reply classification
  graph      flows as graphs (`GraphDef` facade over adk-graph, the only code that names it): run/resume/cancel/waiting/subscribe,
             SQLite checkpoints, `nodes::challenge` interrupt shared by discovery and trends
  net        `net::http`: the one safe HTTP client for third-party data (public hosts only, per-host delay, size cap, hop-by-hop redirects)
  notify     desktop notifications (best effort)
  prefs      `Preferences` model, `chat` + `chat_graph` (graph `prefs_chat`), `undo`, `suggest` (chips), `appearance` (kv `ui.appearance`)
  profile    `sources` (uploaded .pdf/.md/.txt files and typed notes), `extract` (`Core::read_profile_sources`: documents -> CV schema `Cv`),
             `trace` (keep only facts found in the documents; the rest is `left_out`), `typst` (escape markup);
             `Core::profile` / `require_profile` (NoProfile) is the one way to read it
  discover   `source/` (ATS APIs: greenhouse, lever, ashby, smartrecruiters, recruitee, personio, workday; hn_hiring, yc, career_page
             = "Job page": any page that lists jobs, with JSON-LD, sitemap and board detection), `run` (graphs `discovery`, `source_run`), `score`, `dedup`, `handoff`, `sources`,
             `queue`, `promote` (graph `prepare_cv`, dismiss), `dismissed` (dismiss a CV, list dismissed jobs, Restore),
             `manual` (`Core::add_lead`: pasted link/file -> scored lead), `check` (`Core::check_job_source`: a new source is read
             once and refused unless it lists jobs), `suggest/` (graph `source_suggest`: searches in the user's Chrome, Private
             tab -> results parsed in code -> checked -> `source_suggestions`; a captcha/consent page pauses it, never solved)
  trends     collection (hackernews, yc, rss, page; graph `trends` -> `trend_items`), `period` (week / month / quarter / year;
             one report per period, a rerun replaces it), `frame` (MarketFrame: background + place, never wishes; decides what is counted),
             `digest/` (extract, count, synthesize, cite: every point cites stored items), `pdf` (report -> PDF via `assets/trends.typ`)
src-tauri/ (package `jobpilot`, lib `jobpilot_lib`)   Tauri v2 shell, split by bounded context (never by technical layer)
  contexts   discovery (Leads, job Sources) | applications (stages CVs/Applied/In progress/Done, Application: JD/CV/apply/timeline) |
             profile (About you: the documents the profile is read from; preferences) | integrations (AI Providers, Gmail, Browser: extension pairing + signed-in sites) | trends (Digest, trend sources)
  tauri.conf.json, capabilities/default.json   one window `main`; permissions: core + the file picker only; strict CSP (no remote origin)
  src/main.rs          only calls `jobpilot_lib::run()`
  src/lib.rs           composition root: logging, `Core`, demo seed, mail autostart, the window (app id `jobpilot`), the extension bridge, `apply::close_all` on exit;
                       routes a command to its context by name prefix (`<context>_<name>`)
  src/shared/          shared kernel: `error` (CommandError { problem, message } via `health::classify`, `run` / `run_sync`), `events`
                       (`shared:data-changed`, `shared:graph-progress` bridge), `insight` (company brief read model), `topics` (preference-list rules that Profile and Trends share), `activity` (the sidebar's background jobs: running and waiting graph runs, titles and step labels), `shell` (health line,
                       setup banner, badges, appearance, `shared_open_url`: the only way a link leaves the app)
  src/<context>/       `dto.rs` read models (serde camelCase; core types stay free of serde/Tauri; never a secret),
                       `rules.rs` pure rules + unit tests (`primary_for`, `stage_for`, queue ordering, timeline grouping, ...),
                       `commands.rs` application services: thin `#[tauri::command]`s that call `Core` through `shared::run`
  src/demo.rs          `JOBPILOT_DEMO=1` sample data
  src/devtools.rs      cargo feature `dev-tools` only: webview snapshot + UI script runner (see "Check the UI")
package.json, bun.lock, index.html, vite.config.ts, tsconfig.json, biome.jsonc   the web app (bun), standard Tauri layout; biome.jsonc = lint, format and the import rules
lefthook.yml           git hooks: pre-commit Biome on staged files, pre-push `scripts/check.sh`
src/                   React + Vite + TypeScript + Tailwind; feature folders (applications, discovery, integrations, profile, trends) inside each kind
  main.tsx, App.tsx    entry and screen switch (file drop)
  pages/               JobsPage (header, stage bar Leads | CVs | Applied | In progress | Done, the stage's list, right-side drawer for the
                       open lead or application), ApplicationPage (the drawer's "Open full view"), TrendsPage (New research dialog,
                       tabs Reports | Sources, a report opens from its row), ProfilePage (About you +
                       Preferences), SettingsPage (Connections, Job sources, Appearance)
  components/          `ui/kit.tsx` (design kit: Page, Card, Section, Banner, Button, Chip, Tag, Segmented, Tabs, StageBar, DataTable,
                       Drawer, Dialog, HoverCard, Combobox, FormMessage, ToastCard, ScoreBadge, Stepper, ...), `ui/primitives/` (shadcn/ui, square:
                       no rounded corners; only the kit imports them, Biome enforces it), `layout/` (Sidebar, SetupBanner, Toaster),
                       ProblemHelp (LoadError, FormError, ProblemActions), InsightBrief,
                       `<feature>/` (drawers, tabs, stages, settings sections)
  hooks/               `useAsync` (`useQuery`, `useAction`), `useGraphProgress`, `useErrorToast`, `<feature>/` (screen state, loading, Problem)
  stores/              zustand: `nav` (screen, Jobs stage, drawer `{ kind: "lead" | "app", id }`, Jobs badge), `shell` (health line, banner,
                       appearance), `activity` (background jobs beside the app title, `shared_activity`), `today` (the Leads queue), `toast` (`toast.ok` / `toast.error`)
  api/                 `bridge.ts` (the only Tauri imports: invoke / listen / file picker; imported only inside `api/`), `events.ts`
                       (what the rest may subscribe to: data changed, graph progress, file drop), `shell.ts` (`shared_*`: status,
                       appearance, `openLink`), one file of typed command calls per feature
  lib/                 pure helpers + vitest: `problem`, `theme`/`tokens` (light/dark as `--jp-*` CSS variables; `styles.css` maps them to
                       the only Tailwind colours), `display`, `utils` (`cn`), `<feature>/`
  types/               types mirroring the shell's DTOs (one file per feature, `shell.ts`)
  test/                `setup.ts` (vitest: clears the toasts), `render.tsx` (`renderWithToasts`), `fixtures/` (sample data)
```

## Commands

- Web app first (the shell embeds `dist/` at compile time): `bun install`, then `bun run build`.
- Run: `cargo run -p jobpilot` (the built web app; sample data: `JOBPILOT_DEMO=1`; throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory`). With hot reload: `bun run tauri dev`.
- Web checks: `bun run check` (tsc, Biome lint + format with the import rules, vitest); `bun run format` applies Biome's fixes.
- Build/test: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace`. The engine is its own crate without Tauri: `cargo test -p jobpilot-engine` needs no `dist/` and no webkit/gtk libs (libdbus only). The shell (`-p jobpilot`) needs a `dist/` (`scripts/check.sh` writes a placeholder) and the Linux libs below.
- Browser tests (`#[ignore]`, start Chromium with the built extension and an auto-approving bridge, local fixture pages only): `bun run ext:build`, then `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine -- --ignored` (`browser::bridge::testkit`; it takes `JOBPILOT_CHROME` or Playwright's Chromium under `/opt/pw-browsers`: branded Google Chrome ignores `--load-extension`). Guarded tabs block `127.0.0.1` literals, so their fixtures are reached as `http://<name>.test:<port>`.
- End-to-end without real services: `cargo test -p jobpilot-engine --test e2e_discovery` (fake provider + fake Greenhouse board on loopback, real typst: a text document -> profile, source -> leads with score and insight -> Prepare CV -> Approve CV); with `bun run ext:build` then `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine --features test-browser --test e2e_discovery -- --ignored` it also dry-runs the apply on a local form in a Chromium driven through the JobPilot extension (the cargo feature `test-browser` exposes the bridge's test kit to `tests/`).
- Package on Linux/macOS: `scripts/package.sh` -> `release/`. On Windows 11 x64: `bun run tauri build --target x86_64-pc-windows-msvc` -> `target/x86_64-pc-windows-msvc/release/bundle/nsis/` (unsigned current-user installer with the extension bundled).
- Needs: Rust, Node + bun, `typst` CLI (`cargo install --locked typst-cli`), Chrome/Chromium 125+ with the JobPilot extension (Settings > Browser shows how to install and pair it), Linux libs (`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev`, libdbus for the keyring); on macOS Xcode's command line tools instead of those libs (secrets go to the Keychain; the `dev-tools` feature is Linux-only).
- Windows build tools: Rust MSVC, Visual Studio Build Tools (Desktop development with C++ and a Windows SDK), and WebView2. Install Typst separately with `winget install --id Typst.Typst` or set `JOBPILOT_TYPST`. Use Git Bash for the gate and hooks. Data defaults to `%APPDATA%/jobpilot` and secrets use Windows Credential Manager.
- Pre-PR checks (there is no CI; this is the gate, run by the pre-push hook and `/pre-pr`): `scripts/check.sh [--all] [--only web|rust|render]` runs the groups touched since main: `web` (typecheck, Biome, vitest, vite build, the extension's typecheck + build), `rust` (clippy + test), `render` (typst on `assets/prompts/example-cv.yaml`). `CARGO_PROFILE_ARG="--profile ci"` makes a cold run faster. `scripts/risk.sh [--label <tier>]` prints the risk tier the changed paths require (`docs/REVIEW.md`).
- Hooks: `bun install` installs lefthook (pre-commit: Biome on staged files; pre-push: `check.sh`). If `core.hooksPath` was set before: `git config --unset core.hooksPath && bunx lefthook install`.
- Logs: stderr and `<data_dir>/logs/`; `RUST_LOG=jobpilot_engine=debug` for more.

## Check the UI visually

On Linux, a window capture cannot see the webview on a Wayland session and a screen capture asks the user for consent, so the app checks itself: with the cargo feature `dev-tools` (off by default, never packaged) the webview takes its own picture and runs a script that drives the page.

```
bun run build
JOBPILOT_UI_SCRIPT=/tmp/check.js JOBPILOT_UI_SHOTS=/tmp/shots \
JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory JOBPILOT_DEMO=1 \
  cargo run -p jobpilot --features dev-tools
```

`check.js` is the body of an async function; the app exits when it ends. Helpers (`src-tauri/src/devtools.rs`): `shot(name)` (PNG in `JOBPILOT_UI_SHOTS`), `log(text)` (app log), `sleep(ms)`, `waitFor(fn, ms)`, `$(selector)`, `byText(text, selector)`, `click(target)` (a selector, or the text / aria-label of a button), `type(target, text)`, `invoke(command, args)`. Example:

```text
await waitFor(() => $("#nav-jobs"));
await shot("jobs-leads");
await click("#stage-cv");
await click(await waitFor(() => $("#table-cv tr[aria-label]")));
await waitFor(() => $("#drawer"));
await shot("jobs-cv-drawer");
await click("#drawer-close");
await click("#open-settings");
await click(byText("Light"));
await shot("settings-light");
```

Page errors, unhandled rejections and CSP violations are written to the app log (`ui script: ...`). Every interactive element has an `id` or an accessible name, so a script (or assistive technology) can reach it.

For an end-to-end run without real keys, seed a provider row in `<data_dir>/jobpilot.db` that points at a local fake OpenAI-compatible server (`/v1/chat/completions`), and apply against the local fixture form. Discovery refuses loopback hosts by design (`net::http`, `browser::guard`), and the GUI needs the extension paired in a real Chrome,, so a fake job board cannot be driven from the GUI: that path is covered by `crates/engine/tests/e2e_discovery.rs`, and the GUI check of the Jobs page (Leads and a lead drawer), Trends and Settings > Job sources uses `JOBPILOT_DEMO=1` data (it also seeds job sources and scored leads).

## Workflow

- **One worktree per change** (`claude -w <name>`, in `.claude/worktrees/`). If the user explicitly requests a hotfix, work directly in the current branch without creating a new worktree.
- **Docs-only changes skip all of that.** A change that touches only `docs/` (except `docs/REVIEW.md`) is committed on `main` and pushed directly: no worktree, no PR, no `/pre-pr`.
- **Execute an approved plan with subagents.** Split it into independent tasks, each a `general-purpose` subagent with a self-contained brief (goal, files, steps, checks). The main session checks their results, then runs `/pre-pr`.
- **Show the result, don't claim it.** Report the output of `scripts/check.sh`. Never use `--no-verify`.
- **Finish with `/pre-pr` without asking.** Low and Medium PRs are merged by `/pre-pr` once `scripts/check.sh` and `scripts/risk.sh` pass on the PR's final commit. High PRs wait for a human.

## Rules

- Never add facts to a CV that are not in the user's documents (employers, titles, dates, institutions, numbers, technologies). The profile is read from them (`engine::profile`), every fact must trace back to a document (`profile::trace`), and every CV is checked against the profile (`pipeline::guard`). Code never writes a fact into the profile; the user changes a document and reads again.
- Secrets (API keys, OAuth tokens) only in the OS keyring via `engine::secrets`; never in the DB, logs or files. Logs are redacted as a second line of defence: still never put a secret in a log call, an error message or a `Debug` dump.
- The UI never blocks and never owns I/O: a command calls the engine through `shared::run(fut)` / `shared::run_sync(f)` (the engine runtime); long work streams progress as events and the command returns the final result.
- The shell reads/writes through `Core` only; no SQL or HTTP in `src-tauri` (they live in `crates/engine`). The web app talks to the shell only through `src/api/` (Tauri is imported in `src/api/bridge.ts` only, and the bridge only inside `src/api/`).
- Bounded contexts do not depend on each other: Rust `src-tauri/src/<context>` uses `shared` and `engine` only. What two contexts share goes to the shared kernel. (The web app is not split by context: see the `src/` layout above.)
- DTOs (`<context>/dto.rs`) never carry a secret: keys and tokens are write-only from the web app (it can learn that one is saved, never its value).
- The webview shows the app only: no remote origin in the CSP, navigation is refused (`main.rs::may_show`), and http(s) links open in the system browser through `shared_open_url`. Untrusted text is rendered as text, never as HTML.
- Statuses only move forward (`Status::can_advance_to`); use `Store::set_status` so a timeline event is recorded.
- Page/JD/email text is untrusted: an LLM may fill values for known field ids, never choose actions or URLs.
- Schema change = append a migration in `store/mod.rs`; never edit an existing one.
- No `unwrap()` / `expect()` on runtime paths (only tests and provably-infallible statics like a constant regex). Return errors with `anyhow::Context`, or recover.
- Failures the user can act on become a `health::Problem` (no provider, browser extension not connected or not allowed in Incognito, no typst, network) and are shown with a call to action; network errors get a Retry.
- Log key steps at `info!` with ids/counts only (no page, JD, email or CV text); failures at `warn!`/`error!`.
- Views use the design kit (`src/components/ui/kit.tsx`) and the theme classes (`bg-surface`, `text-ink`, ...) for colours and widgets: no hard-coded colours (Tailwind's stock palette is removed), no local copies of kit components.
- Add/edit flows open a kit `Dialog` (Save closes it and says so in a toast), not a form that expands inside a card.
  A modal dialog is its own screen: its main button is its one primary (the screen behind is inert while it is open).
- Errors outside a form are toasts: `useErrorToast` for a failed action, `LoadError` for a failed read. Red text is only a
  form's own error under its fields (`FormError` / `FormMessage`, also inside a dialog).
- Exactly one primary button at a time (`applications/rules.rs::primary_for`, unit-tested; the web app renders what the DTO says); it is disabled while work runs. Secondary actions are plain buttons.
- Outgoing HTTP to a URL that is not the user's own (job boards, feeds, postings, search results) goes through `net::http`; discovery tabs are opened with a `guard` policy (`OpenOptions::guard`): the extension blocks private literals and the app checks every other request (`guard::allows`). Both refuse machine-local and private hosts; the tab check is weaker than a proxy (Chrome resolves names again), see `extension/PROTOCOL.md`. `allow_private_hosts` exists for tests only.
- JobPilot's tabs never disguise automation (`navigator.webdriver` is set to true, no user-agent change, Chrome's debugging bar stays), and a login, captcha or block page is never solved, evaded or waited out: the source pauses and the user deals with it in a focused tab of their Chrome.
- The extension stays a dumb relay: it acts only on tabs it opened, passes only allowlisted CDP methods, returns cookie domains and never cookie names or values, and holds no logic the app could decide. Every decision is made in Rust. Untrusted pages are read in `Profile::Private` (no logins) unless the user bound a signed-in site; never fall back to the signed-in profile silently.
- Graphs (`engine/graph`): state is one `serde` struct, nodes return the new state and must be idempotent (a resume re-runs the node). A node with several plain incoming edges is an AND join and waits for all of them, so a loop or an alternative path must use `route`, never a second edge. A paused branch holds back its siblings in the same run: independent work runs as its own thread (`Graphs::run_each`). Parallel nodes only share a field declared with `append`.
- Leads are not applications: a found job is a `leads` row; it becomes an application only when the user presses Prepare CV (`Core::promote`, idempotent). Discovery never promotes and never writes a CV.
- Doc examples that are not compilable use ```` ```text ````, not ```` ```ignore ````: `cargo test -- --ignored` (the browser tests) also runs ignored doctests, and they fail to compile.
- Untrusted text (postings, feed items, company pages) reaches the model only between markers, as data, and the model fills fixed schemas; URLs come from adapters in code.

## Ownership (parallel work)

- Engine (`crates/engine`): add behaviour to `Core` with `impl Core { .. }` in your own module, not in `facade.rs`. An item the shell calls is `pub` (not `pub(crate)`).
- Desktop: a change belongs to one bounded context and edits only `src-tauri/src/<context>/` and that feature's web files: every path under `src/` named after it (`<feature>/` folders, `<feature>.ts`; integrations also `api/{gmail,providers,sessions}.ts`; the extension (`extension/`) belongs to no context: it changes with `engine::browser::bridge`), plus its page or store (applications: `pages/ApplicationPage.tsx`; discovery: `stores/today.ts`; trends: `pages/TrendsPage.tsx`; profile: `pages/ProfilePage.tsx`).
- The shared kernel (`src-tauri/src/shared`, `src-tauri/src/main.rs`) and every other web file under `src/` (the kit, layout, nav and shell stores, `api/{bridge,events,shell}.ts`, shared hooks, `lib/`, `types/`, the Jobs and Settings pages) change only for something every context needs.
- Prompts under `assets/prompts/` are High risk (a human reads the diff).
