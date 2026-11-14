# JobPilot

Desktop app (Rust core, Tauri + React UI, Linux) that turns a job description into a CV tailored from your own `profile.yaml`, lets you edit it, fills and submits the application in a managed browser **only after you confirm**, and tracks replies from Gmail. Works with any OpenAI- or Anthropic-compatible provider.

Flow: paste a JD link (or file) -> Generate CV -> edit / Approve CV -> Review & Submit -> Submit -> status follows your Gmail replies.

It can also find the jobs for you: job sources (Greenhouse, Lever, Ashby boards, Hacker News "Who is hiring", Y Combinator, career pages) fill a **Queue** of scored postings, and a CV is written only for the ones you pick (see "Finding jobs").

## Requirements

- Rust (stable), Node and bun to build
- [`typst`](https://typst.app) CLI: `cargo install --locked typst-cli` (the app tells you if it is missing)
- Chrome or Chromium for browser automation (Settings > Browser sessions can also download Chrome for Testing, only when you click it)
- Linux with Wayland or X11, WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev` to build), and a Secret Service (gnome-keyring / KWallet, libdbus) for the keyring
- `profile.yaml`: your master profile, the only source of facts for CVs. Found by walking up from the working directory, else `<data dir>/profile.yaml`, else set its path in Settings

## Run

```
bun install
bun run build     # the UI, embedded in the binary
cargo run -p jobpilot --release
```

Throwaway state for trying it out: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory cargo run -p jobpilot`. Sample applications: `JOBPILOT_DEMO=1`.

## First-time setup (Settings)

1. **AI provider** (Settings > AI providers): add a provider (OpenAI-compatible or Anthropic), base URL, model and API key, then press Test. Until one works, the app shows a "set up a provider" banner.
2. **Browser sessions** (Settings > Browser sessions): press New session, enter a name and the login URL (e.g. `https://www.linkedin.com/login`), press Open browser to log in, sign in by hand in the window that opens, then press Done. JobPilot keeps that browser profile and reuses it for that site (Check re-tests that you are still logged in).
3. **Gmail** (Settings > Gmail, optional): create an OAuth client in Google Cloud Console (APIs & Services > Credentials > OAuth client ID, type Desktop app; enable the Gmail API, read-only scope). Paste the client ID (and secret) into Settings, then Connect Gmail. The consent page opens in your browser and redirects to `http://127.0.0.1:<port>`. The refresh token goes to the keyring.

## Finding jobs

**Job sources** (Settings > Job sources): add where JobPilot looks. Pick a kind and give a board name or address: `greenhouse` (`boards.greenhouse.io/<company>` or just the name), `lever` (`jobs.lever.co/<company>`), `ashby` (`jobs.ashbyhq.com/<company>`), `hn_hiring` (no address), `yc` (the Work at a Startup listing) or `career_page` (any company careers page). Each source can be switched off, and a source that needs a login can be bound to one of your Browser sessions. Boards are read through their public job APIs; pages are read in the managed browser only when plain HTTP is not enough.

**Queue** (sidebar > Queue): press **Search now** to run every enabled source. For each new posting JobPilot drops the ones it already knows (same link, or same company and role), applies your hard filters (excluded companies, location, seniority) and scores the rest against your profile and preferences. The Queue lists the leads best match first, with the top reasons for the score. Open one to see the score breakdown and the **job insight**: business domain, company facts (size, stage, funding, HQ, founded), required and preferred skills and the interview process, each only when the posting or the company's own pages say it, otherwise "not found". Filters: source, minimum score, location.

- **Prepare CV** writes a CV for that posting only (it becomes a normal application, ready for Approve CV and the rest of the flow above). Nothing is promoted or written automatically.
- **Dismiss** removes a lead, with an optional reason and "Don't show this company again".
- **Open posting** opens the original page. **Next in queue** takes you to the next lead after you submit or dismiss one.

**When a site stops the search.** If a source meets a login, a captcha or a block page, JobPilot never tries to get past it. That source pauses, you get a desktop notification and a "needs you" line in the Queue. **Open browser** shows the page in a visible window (in the source's session, if it has one); log in or solve the check yourself, then press **Continue** (or **Skip** to leave the source for the next run). The other sources keep going meanwhile, and a paused source survives restarting the app. Three stops in a row rest that source for the day.

**Preferences** is a chat that asks what you want (it can read an uploaded CV for context, without changing your profile) and saves it; you can also edit every field directly. Discovery scores leads with these preferences. **Trends** builds a digest of market signals from sources you add, with every statement linked to its source; "Add to preferences" puts a suggestion in the Preferences chat for you to send.

## Package

```
scripts/package.sh
```

Writes a `.deb` and an AppImage to `release/` with the Tauri bundler (`BUNDLES=deb scripts/package.sh` for the `.deb` only; the AppImage step downloads its tooling on first use). The `.deb` depends on WebKitGTK 4.1. The `typst` CLI is not bundled.

## Where data lives

Everything private stays on this machine, under `$XDG_DATA_HOME/jobpilot` (usually `~/.local/share/jobpilot`, override with `JOBPILOT_DATA_DIR`):

| Path | Content |
| --- | --- |
| `jobpilot.db` | SQLite: applications, timeline, providers (no keys), emails, job sources, leads, preferences, flow checkpoints |
| `applications/<slug>/` | `jd.md`, `cv.yaml`, `cv.pdf`, page previews |
| `leads/<id>/` | `jd.md` of each found posting |
| `sessions/<id>/` | browser profiles (mode 0700) |
| `shots/` | apply screenshots |
| `logs/jobpilot.log.<date>` | log file, 14 days kept |
| `chrome/` | Chrome for Testing, if you downloaded it |

API keys and OAuth tokens are only in the OS keyring (service `jobpilot`).

Other env vars: `JOBPILOT_CHROME` (Chrome path), `JOBPILOT_TYPST` (typst path), `JOBPILOT_DRY_RUN=1` (force every apply to be a dry run), `JOBPILOT_NO_SANDBOX=1` (Chrome without its sandbox, for containers/tests), `RUST_LOG` (log filter).

## Privacy

- No telemetry. The app talks only to: your AI provider, the job sites you apply on, the job sources you add (and, for the job insight, the company's own site when the posting you give is on it), Google (if you connect Gmail), and the Chrome-for-Testing index if you press Download.
- The JD text and the parts of `profile.yaml` needed for the CV are sent to your AI provider. Scoring a found posting sends the posting text, your preferences and a compact profile (headline, summary, skills, roles and degrees; no name or contact details).
- Logs go to stderr and `logs/`, and pass through a redaction layer (API keys, bearer tokens, OAuth codes/tokens, and every secret the app has read from the keyring are masked). Page, JD and email bodies are not logged.

## Safety

- Nothing is submitted until you press Submit on the review screen; the submit button stays disabled while required fields are empty.
- **Dry run** fills the form and never submits; `JOBPILOT_DRY_RUN=1` forces it everywhere.
- The LLM only supplies values for known field ids. It never picks actions or URLs, and page/JD/email text is treated as untrusted.
- Job discovery is read-only and polite (a delay per site, a cap on pages per run). Every request to a third-party address, from the app or from the discovery browser, is refused when it targets this machine or a private network. The discovery browser does not hide that it is automated, and a login, captcha or block page is always handed to you.
- CVs only contain facts from `profile.yaml`.

## Develop

```
cargo build --workspace
cargo test --workspace
JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored   # real Chrome
cargo test -p jobpilot-core --test e2e_discovery                 # source -> lead -> CV -> approve, fake provider and board
```

See `AGENTS.md` for the module map, conventions and how to check the UI visually.
