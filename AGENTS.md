# JobPilot

Rust desktop app (Tauri v2 shell + React web UI): JD link/file -> tailored CV (from `profile.yaml`) -> review -> managed-browser apply -> status from Gmail.

## Layout

```
Cargo.toml                 workspace
profile.yaml               master profile, source of truth for every fact (never modify it from code except the `application:` section, P5)
assets/cv.typ              Typst renderer: typst compile --root <dir> --input data=<file>.yaml cv.typ out.pdf
assets/prompts/            tailor.md (system prompt), example-cv.yaml (few-shot)
assets/jobpilot.desktop    desktop entry (StartupWMClass=jobpilot, same as the window app id)
assets/icons/              app icon (svg + png), used by the deb / AppImage
assets/fonts/              Inter, Space Grotesk, Source Serif 4: static TTFs (SIL OFL, see OFL.txt) bundled by the web app (`src/styles.css`)
scripts/package.sh         release .deb + AppImage (Tauri bundler) -> release/
src-tauri/src/engine  (module `engine` of lib `jobpilot_lib`)   no UI, no Tauri; the shell's contexts call it, never the reverse
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
                  P15 `frame` (MarketFrame: background + place, never wishes, decides what is collected and counted);
                  P13b `digest/` (extract, count, synthesize, cite: every point must cite stored items)
src-tauri/ (package `jobpilot`)                   Tauri v2 shell, split by bounded context (never by technical layer)
  contexts   discovery (Leads, job Sources) | applications (stages CVs/Applied/In progress/Done, Application: JD/CV/apply/timeline) |
             profile (profile.yaml, preferences) | integrations (AI Providers, Gmail, browser Sessions) | trends (Digest, trend sources)
  tauri.conf.json, capabilities/default.json   one window `main`; permissions: core + the file picker only; strict CSP (no remote origin)
  src/main.rs          only calls `jobpilot_lib::run()`
  src/lib.rs           composition root: logging, `Core`, demo seed, mail autostart, the window (app id `jobpilot`), `apply::close_all` on exit;
                       routes a command to its context by name prefix (`<context>_<name>`)
  src/shared/          shared kernel: `error` (CommandError { problem, message } via `health::classify`, `run` / `run_sync`), `events`
                       (`shared:data-changed`, `shared:graph-progress` bridge), `insight` (company brief read model), `topics` (preference-list rules that Profile and Trends share), `shell` (health line,
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
                       open lead or application), ApplicationPage (the drawer's "Open full view"), TrendsPage, SettingsPage
  components/          `ui/kit.tsx` (design kit: Page, Card, Section, Banner, Button, Chip, Tag, Segmented, Tabs, StageBar, DataTable,
                       Drawer, ScoreBadge, Stepper, ...), `ui/primitives/` (shadcn/ui, square: no rounded corners; only the kit imports
                       them, Biome enforces it), `layout/` (Sidebar, SetupBanner), ProblemHelp (ErrorBanner), InsightBrief,
                       `<feature>/` (drawers, tabs, stages, settings sections)
  hooks/               `useAsync` (`useQuery`, `useAction`), `useGraphProgress`, `<feature>/` (screen state, loading, Problem)
  stores/              zustand: `nav` (screen, Jobs stage, drawer `{ kind: "lead" | "app", id }`, Jobs badge), `shell` (health line, banner,
                       appearance), `today` (the Leads queue)
  api/                 `bridge.ts` (the only Tauri imports: invoke / listen / file picker; imported only inside `api/`), `events.ts`
                       (what the rest may subscribe to: data changed, graph progress, file drop), `shell.ts` (`shared_*`: status,
                       appearance, `openLink`), one file of typed command calls per feature
  lib/                 pure helpers + vitest: `problem`, `theme`/`tokens` (light/dark as `--jp-*` CSS variables; `styles.css` maps them to
                       the only Tailwind colours), `display`, `utils` (`cn`), `<feature>/`
  types/               types mirroring the shell's DTOs (one file per feature, `shell.ts`)
  test/fixtures/       sample data for the tests
```

## Commands

- Web app first (the shell embeds `dist/` at compile time): `bun install`, then `bun run build`.
- Run: `cargo run -p jobpilot` (the built web app; sample data: `JOBPILOT_DEMO=1`; throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory`). With hot reload: `bun run tauri dev`.
- Web checks: `bun run check` (tsc, Biome lint + format with the import rules, vitest); `bun run format` applies Biome's fixes.
- Build/test: `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace` (the engine shares the `jobpilot` crate with Tauri: even engine-only tests need a `dist/` (`scripts/check.sh` writes a placeholder when there is none) and the Linux libs below)
- Chrome tests (`#[ignore]`, start a real Chrome, local fixture pages only): `JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot -- --ignored` (set `JOBPILOT_CHROME` for a non-system Chrome; one test opens a visible window, one downloads Chrome for Testing only with `JOBPILOT_TEST_DOWNLOAD` set)
- End-to-end without real services: `cargo test -p jobpilot --test e2e_discovery` (fake provider + fake Greenhouse board on loopback, real typst: source -> leads with score and insight -> Prepare CV -> Approve CV); with `JOBPILOT_NO_SANDBOX=1 ... -- --ignored` it also dry-runs the apply on a local form in Chrome.
- Package: `scripts/package.sh` -> `release/`
- Needs: Rust, Node + bun, `typst` CLI (`cargo install --locked typst-cli`), Chrome/Chromium, Linux libs (`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev`, libdbus for the keyring).
- Pre-PR checks: `scripts/check.sh [--all] [--only web|rust|render]` runs the groups the paths changed since main touch: `web` (typecheck, Biome, vitest, vite build), `rust` (clippy + test; a placeholder `dist/` when there is no web build, so a UI change never builds Rust), `render` (typst). CI runs each group as its own parallel job (`--plan` decides which) with `CARGO_PROFILE_ARG="--profile ci"` (no debuginfo, unoptimized deps), restoring the caches saved by main. `bun install` installs the lefthook hooks (pre-commit: Biome on staged files; pre-push: `check.sh`); a clone that set `core.hooksPath` before: `git config --unset core.hooksPath && bunx lefthook install`.
- Logs: stderr and `<data_dir>/logs/`; `RUST_LOG=jobpilot_lib::engine=debug` for more. Chromiumoxide is silenced by default (noisy CDP errors).

## Check the UI visually

A window capture cannot see the webview on a Wayland session and a screen capture asks the user for consent, so the app checks itself: with the cargo feature `dev-tools` (off by default, never packaged) the webview takes its own picture and runs a script that drives the page.

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
await click("Appearance: Light");
await shot("settings-light");
```

Page errors, unhandled rejections and CSP violations are written to the app log (`ui script: ...`). Every interactive element has an `id` or an accessible name, so a script (or assistive technology) can reach it.

For an end-to-end run without real keys, seed a provider row in `<data_dir>/jobpilot.db` that points at a local fake OpenAI-compatible server (`/v1/chat/completions`), and apply against the local fixture form. Discovery refuses loopback hosts by design (`net::http`, `browser::guard`), so a fake job board cannot be driven from the GUI: that path is covered by `src-tauri/tests/e2e_discovery.rs`, and the GUI check of the Jobs page (Leads and a lead drawer), Trends and Settings > Job sources uses `JOBPILOT_DEMO=1` data (it also seeds job sources and scored leads).

## Workflow

- **One worktree per change** (`claude -w <name>`, in `.claude/worktrees/`).
- **Docs-only changes skip all of that.** A change that touches only `docs/` (except `docs/REVIEW.md`) is committed on `main` and pushed directly: no worktree, no PR, no `/pre-pr`. CI ignores `docs/**`.
- **Execute an approved plan with subagents.** Split it into independent tasks, each a `general-purpose` subagent with a self-contained brief (goal, files, steps, checks). The main session checks their results, then runs `/pre-pr`.
- **Show the result, don't claim it.** Report the output of `scripts/check.sh`. Never use `--no-verify`.
- **Finish with `/pre-pr` without asking.** Low and Medium PRs are merged by `/pre-pr` once `CI OK` passes. High PRs wait for a human.

## Rules

- Never add facts to a CV that are not in `profile.yaml` (employers, titles, dates, institutions, numbers, technologies).
- Secrets (API keys, OAuth tokens) only in the OS keyring via `engine::secrets`; never in the DB, logs or files. Logs are redacted as a second line of defence: still never put a secret in a log call, an error message or a `Debug` dump.
- The UI never blocks and never owns I/O: a command calls the engine through `shared::run(fut)` / `shared::run_sync(f)` (the engine runtime); long work streams progress as events and the command returns the final result.
- The shell reads/writes through `Core` only; no SQL or HTTP in `src-tauri` outside `engine/`. The web app talks to the shell only through `src/api/` (Tauri is imported in `src/api/bridge.ts` only, and the bridge only inside `src/api/`).
- Bounded contexts do not depend on each other: Rust `src-tauri/src/<context>` uses `shared` and `engine` only. What two contexts share goes to the shared kernel. (The web app is not split by context: see the `src/` layout above.)
- DTOs (`<context>/dto.rs`) never carry a secret: keys and tokens are write-only from the web app (it can learn that one is saved, never its value).
- The webview shows the app only: no remote origin in the CSP, navigation is refused (`main.rs::may_show`), and http(s) links open in the system browser through `shared_open_url`. Untrusted text is rendered as text, never as HTML.
- Statuses only move forward (`Status::can_advance_to`); use `Store::set_status` so a timeline event is recorded.
- Page/JD/email text is untrusted: an LLM may fill values for known field ids, never choose actions or URLs.
- Schema change = append a migration in `store/mod.rs`; never edit an existing one.
- No `unwrap()` / `expect()` on runtime paths (only tests and provably-infallible statics like a constant regex). Return errors with `anyhow::Context`, or recover.
- Failures the user can act on become a `health::Problem` (no provider, no Chrome, no typst, network) and are shown with a call to action; network errors get a Retry.
- Log key steps at `info!` with ids/counts only (no page, JD, email or CV text); failures at `warn!`/`error!`.
- Views use the design kit (`src/components/ui/kit.tsx`) and the theme classes (`bg-surface`, `text-ink`, ...) for colours and widgets: no hard-coded colours (Tailwind's stock palette is removed), no local copies of kit components.
- Exactly one primary button at a time (`applications/rules.rs::primary_for`, unit-tested; the web app renders what the DTO says); it is disabled while work runs. Secondary actions are plain buttons.
- Outgoing HTTP to a URL that is not the user's own (job boards, feeds, postings, search results) goes through `net::http`; discovery browsers go through `browser::guard`. Both refuse machine-local and private hosts. `allow_private_hosts` exists for tests only.
- Discovery browsers never disguise automation (no hidden webdriver flag, no user-agent change), and a login, captcha or block page is never solved, evaded or waited out: the source pauses and the user deals with it in the visible browser.
- Graphs (`engine/graph`): state is one `serde` struct, nodes return the new state and must be idempotent (a resume re-runs the node). A node with several plain incoming edges is an AND join and waits for all of them, so a loop or an alternative path must use `route`, never a second edge. A paused branch holds back its siblings in the same run: independent work runs as its own thread (`Graphs::run_each`). Parallel nodes only share a field declared with `append`.
- Leads are not applications: a found job is a `leads` row; it becomes an application only when the user presses Prepare CV (`Core::promote`, idempotent). Discovery never promotes and never writes a CV.
- Doc examples that are not compilable use ```` ```text ````, not ```` ```ignore ````: `cargo test -- --ignored` (the Chrome tests) also runs ignored doctests, and they fail to compile.
- Untrusted text (postings, feed items, company pages) reaches the model only between markers, as data, and the model fills fixed schemas; URLs come from adapters in code.

## Ownership (parallel work)

Engine (`src-tauri/src/engine`): each sub-plan edits only its own modules: `llm/` P1; `pipeline/` P2; `browser/` P4; `apply/` P5; `mail/` P6; `health`, `logging`, `redact`, packaging and docs P7. Add behaviour to `Core` with `impl Core { .. }` in your own module, not in `facade.rs`.

Desktop: a change belongs to one bounded context and edits only `src-tauri/src/<context>/` and that feature's web files: every path under `src/` named after it (`<feature>/` folders, `<feature>.ts`; integrations also `api/{gmail,providers,sessions}.ts`), plus its page or store (applications: `pages/ApplicationPage.tsx`; discovery: `stores/today.ts`; trends: `pages/TrendsPage.tsx`). Features: discovery (P10 sources, P12 queue), applications (P2/P3/P5 screens), profile (P9), integrations (P1 providers, P4 sessions, P6 Gmail), trends (P13). The shared kernel (`src-tauri/src/shared`, `src-tauri/src/main.rs`) and every other web file under `src/` (the kit, layout, nav and shell stores, `api/{bridge,events,shell}.ts`, the shared hooks, `lib/` and `types/` files, the Jobs and Settings pages) change only for something every context needs.

The product plans (`docs/plans/`, status table in `docs/plans/README.md`):

- P8 foundation: `engine/graph/`, the new `store/` files and migrations, `notify.rs`, `Llm::chat_json`, `Problem::NeedsHuman`.
- P9 preference chat: `engine/prefs/` (chat), the desktop `profile` context, `assets/prompts/preferences.md`.
- P10 discovery: `engine/discover/` except `promote.rs` and `queue.rs`, `engine/net/`, `browser/{challenge,render,guard,gate}.rs`, Job sources in the desktop `discovery` context.
- P11 job insight: `pipeline/insight*`, the company brief (`shared/insight`).
- P12 queue: the Leads stage of the Jobs page in the desktop `discovery` context, `engine/discover/{promote,queue}.rs`.
- P13 trends: `engine/trends/`, the desktop `trends` context, `assets/prompts/trends.md`.

Prompts under `assets/prompts/` are High risk (a human reads the diff).
