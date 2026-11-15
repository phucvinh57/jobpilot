# JobPilot

Desktop app (Rust core, Tauri + React UI, Linux) that turns a job description into a CV tailored from what you tell it about your work (an old CV, a LinkedIn export, a few notes), lets you edit it, fills and submits the application in a managed browser **only after you confirm**, and tracks replies from Gmail. Works with any OpenAI- or Anthropic-compatible provider.

Flow: paste a JD link (or file) -> Generate CV -> edit / Approve CV -> Review & Submit -> Submit -> status follows your Gmail replies. JobPilot can also find jobs for you (see "Finding jobs").

## Requirements

- Rust (stable), Node and bun to build
- [`typst`](https://typst.app) CLI: `cargo install --locked typst-cli` (the app tells you if it is missing)
- Chrome or Chromium (Settings > Browser sessions can download Chrome for Testing when you click it)
- Linux with Wayland or X11, WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev` to build), and a Secret Service (gnome-keyring / KWallet, libdbus) for the keyring

## Run

```
bun install
bun run build     # the UI, embedded in the binary
cargo run -p jobpilot --release
```

Throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory cargo run -p jobpilot`. Sample data: `JOBPILOT_DEMO=1`.

## Setup (Settings)

1. **AI providers**: add a provider (OpenAI-compatible or Anthropic) with base URL, model and API key, then press Test.
2. **About you**: add your CV (.pdf, .md or .txt), a LinkedIn export, or notes. JobPilot reads them into your profile and shows what it understood and what it left out because no document states it. To fix something, change a document and press Re-read.
3. **Browser sessions**: New session, give a name and login URL, Open browser, sign in by hand, Done. JobPilot reuses that browser profile for the site.
4. **Gmail** (optional): create a Desktop OAuth client in Google Cloud Console with the Gmail API enabled (read-only scope), paste the client ID (and secret), then Connect Gmail. The refresh token goes to the keyring.

## Finding jobs

- **Job sources** (Settings > Job sources): job boards (Greenhouse, Lever, Ashby, SmartRecruiters, Recruitee, Personio, Workday), Hacker News "Who is hiring", Y Combinator's Work at a Startup, or any company careers page. A source can be switched off or bound to a Browser session.
- **Leads** (Jobs > Leads): **Search now** runs every enabled source. Known postings and those failing your hard filters are dropped; the rest are scored against your profile and preferences, best match first. A lead shows its score breakdown and a **job insight** (domain, company facts, skills, interview process), each only when a page states it.
- **Prepare CV** turns a lead into an application with a CV for that posting. Nothing is written automatically. **Dismiss** removes a lead (optionally the whole company).
- **When a site stops the search** (login, captcha, block page), JobPilot never tries to get past it: the source pauses, you get a notification, **Open browser** shows the page so you can deal with it, then **Continue** or **Skip**. Other sources keep going.
- **Preferences** (Settings > Profile & preferences): a chat that asks what you want and saves it; every field can also be edited directly.
- **Trends**: a digest of market signals from sources you add, every statement linked to its source.

## Package

```
scripts/package.sh
```

Writes a `.deb` and an AppImage to `release/` (`BUNDLES=deb` for the `.deb` only). The `typst` CLI is not bundled.

## Where data lives

Under `$XDG_DATA_HOME/jobpilot` (usually `~/.local/share/jobpilot`, override with `JOBPILOT_DATA_DIR`):

| Path | Content |
| --- | --- |
| `jobpilot.db` | SQLite: applications, timeline, providers (no keys), emails, job sources, leads, preferences, flow checkpoints |
| `applications/<slug>/` | `jd.md`, `cv.yaml`, `cv.pdf`, page previews |
| `leads/<id>/` | `jd.md` of each found posting |
| `sessions/<id>/` | browser profiles (mode 0700) |
| `shots/` | apply screenshots |
| `logs/jobpilot.log.<date>` | log file, 14 days kept |
| `chrome/` | Chrome for Testing, if downloaded |

API keys and OAuth tokens live only in the OS keyring (service `jobpilot`).

Other env vars: `JOBPILOT_CHROME` (Chrome path), `JOBPILOT_TYPST` (typst path), `JOBPILOT_DRY_RUN=1` (every apply is a dry run), `JOBPILOT_NO_SANDBOX=1` (Chrome without its sandbox, for containers/tests), `RUST_LOG`.

## Privacy and safety

- No telemetry. The app talks only to your AI provider, the sites you apply on, the job and trend sources you add (and a company's own site for the job insight), Google (if you connect Gmail), and the Chrome-for-Testing index if you press Download.
- Your documents are sent to your AI provider when read into your profile; the JD and profile when a CV is written; a posting, your preferences and a compact profile (no name or contact details) when a lead is scored.
- Logs are redacted (API keys, tokens, every keyring secret) and never contain page, JD or email bodies.
- Nothing is submitted until you press Submit on the review screen. **Dry run** fills the form and never submits.
- The LLM only fills values for known form fields; it never picks actions or URLs. Page, JD and email text is untrusted.
- Discovery is read-only and polite, refuses this machine and private networks, does not hide that it is automated, and hands every login, captcha or block page to you.
- CVs only contain facts from your documents: the profile keeps only facts a document states, and every CV is checked against it.

## Develop

```
cargo test --workspace
JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot -- --ignored        # real Chrome
cargo test -p jobpilot --test e2e_discovery                      # source -> lead -> CV -> approve, fake provider and board
```

See `AGENTS.md` for the module map, conventions and how to check the UI visually.
