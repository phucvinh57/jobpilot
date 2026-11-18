# JobPilot

Desktop app (Rust core, Tauri + React UI, Linux, macOS, and Windows 11 x64) that turns a job description into a CV tailored from what you tell it about your work (an old CV, a LinkedIn export, a few notes), lets you edit it, fills and submits the application in a tab of your own Chrome (through the JobPilot extension) **only after you confirm**, and tracks replies from Gmail. Works with any OpenAI- or Anthropic-compatible provider.

Flow: paste a JD link (or file) -> Generate CV -> edit / Approve CV -> Review & Submit -> Submit -> status follows your Gmail replies. JobPilot can also find jobs for you (see "Finding jobs").

## Requirements

- Rust (stable), Node and bun to build
- [`typst`](https://typst.app) CLI: `cargo install --locked typst-cli` (the app tells you if it is missing)
- Chrome or Chromium (or another Chromium browser, version 125+) with the JobPilot extension (`extension/`, see Setup)
- Linux with Wayland or X11, WebKitGTK 4.1 (`libwebkit2gtk-4.1-dev libjavascriptcoregtk-4.1-dev libsoup-3.0-dev libgtk-3-dev librsvg2-dev` to build), and a Secret Service (gnome-keyring / KWallet, libdbus) for the keyring
- or macOS 11+ with Xcode's command line tools (`xcode-select --install`); secrets go to the Keychain, and `brew install typst` works too. `scripts/package.sh` builds an unsigned `.app` and `.dmg`: macOS quarantines them when downloaded, so open it with right-click > Open (or `xattr -dr com.apple.quarantine JobPilot.app`)

- On Windows 11 x64, install the Rust MSVC toolchain and Visual Studio Build Tools with **Desktop development with C++** and a Windows SDK. JobPilot uses Microsoft Edge WebView2 and Windows Credential Manager.

## Run

```
bun install
bun run build     # the UI, embedded in the binary
bun run ext:build # the browser extension, in extension/dist
cargo run -p jobpilot --release
```

Throwaway state: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory cargo run -p jobpilot`. Sample data: `JOBPILOT_DEMO=1`.

### Windows PowerShell

Install Typst separately. Open a new PowerShell window after installing it so the shell can find it.

```powershell
rustup default stable-x86_64-pc-windows-msvc
winget install --id Typst.Typst
bun install --frozen-lockfile
bun run build
bun run ext:build
cargo run -p jobpilot --release
```

If you installed Typst to a custom location, set `$env:JOBPILOT_TYPST` to the full path to `typst.exe`. JobPilot also looks on `PATH`, in Cargo’s `bin` directory, and in your WinGet Links directory. To run with sample data, set `$env:JOBPILOT_DEMO = "1"` before launching. For disposable local state, set `$env:JOBPILOT_DATA_DIR = "$env:TEMP\jobpilot-demo"` and `$env:JOBPILOT_SECRETS = "memory"`.

## Setup (Settings and Profile)

1. **AI providers**: add a provider (OpenAI-compatible or Anthropic) with base URL, model and API key, then press Test.
2. **About you** (Profile > About you): add your CV (.pdf, .md or .txt), a LinkedIn export, or notes. JobPilot reads them into your profile and shows what it understood and what it left out because no document states it. To fix something, change a document and press Re-read.
3. **Browser** (Settings > Connections > Browser): JobPilot never starts a browser; it works in your own Chrome through its extension.
   - Install it once: open `chrome://extensions`, turn on Developer mode, **Load unpacked**, and pick the folder Settings shows (`extension/dist` in a source build, the `extension` folder of the package otherwise).
   - Pair: the extension connects to the app on `127.0.0.1:47615` and shows a 6-digit code; check that Settings shows the same code and press **Pair** in Settings and in the extension's popup (both are needed). The pairing token is kept in the OS keyring and never sent again: on later connections the app and the extension prove to each other that they hold it (`extension/PROTOCOL.md`).
   - Recommended: on the extension's Details page, turn on **Allow in Incognito**. Job pages are then read, and forms without a login filled, in a private window without your cookies.
   - **Sites you're signed in to**: add a site (name and login URL) and press Sign in to open it in your Chrome. JobPilot uses your own signed-in browser for that site; it never reads or changes your cookies (it only checks which domains have some).
   - JobPilot's tabs open in a "JobPilot" window and tab group, and Chrome shows its "started debugging this browser" bar while they are driven. The extension only touches tabs it opened itself.
4. **Gmail** (optional): create a Desktop OAuth client in Google Cloud Console with the Gmail API enabled (read-only scope), paste the client ID (and secret), then Connect Gmail. The refresh token goes to the keyring.

## Finding jobs

- **Job sources** (Settings > Job sources): job boards (Greenhouse, Lever, Ashby, SmartRecruiters, Recruitee, Personio, Workday), Hacker News "Who is hiring", Y Combinator's Work at a Startup, or any company careers page. A source can be switched off or bound to a site you're signed in to.
- **Leads** (Jobs > Leads): **Search now** runs every enabled source. Known postings and those failing your hard filters are dropped; the rest are scored against your profile and preferences, best match first. A lead shows its score breakdown and a **job insight** (domain, company facts, skills, interview process), each only when a page states it.
- **Prepare CV** turns a lead into an application with a CV for that posting. Nothing is written automatically. **Dismiss** removes a lead (optionally the whole company).
- **When a site stops the search** (login, captcha, block page), JobPilot never tries to get past it: the source pauses, you get a notification, **Open browser** shows the page in a tab of your Chrome so you can deal with it, then **Continue** or **Skip**. Other sources keep going.
- **Preferences** (Profile > Preferences): a chat that asks what you want and saves it; every field can also be edited directly.
- **Trends**: a digest of market signals from sources you add, every statement linked to its source.

## Package

```
scripts/package.sh
```

On Linux, this writes a `.deb` and an AppImage to `release/` (`BUNDLES=deb` for the `.deb` only). On macOS, it writes a zipped `.app` and a `.dmg`.

On Windows, build the x64 installer from PowerShell after installing dependencies.

```powershell
bun run tauri build --target x86_64-pc-windows-msvc
```

Run the unsigned `*-setup.exe` from `target\x86_64-pc-windows-msvc\release\bundle\nsis\`. It installs JobPilot for the current user and downloads WebView2 if needed. Launch JobPilot from the Start menu. To install the bundled Chrome extension, copy the folder path shown in **Settings > Connections > Browser** and paste it into Chrome’s **Load unpacked** picker. Install Typst separately on each machine that will generate PDFs.

## Where data lives

The default directory is `$XDG_DATA_HOME/jobpilot` (usually `~/.local/share/jobpilot`) on Linux, `~/Library/Application Support/jobpilot` on macOS, and `%APPDATA%\jobpilot` on Windows. Set `JOBPILOT_DATA_DIR` to override it.

| Path | Content |
| --- | --- |
| `jobpilot.db` | SQLite: applications, timeline, providers (no keys), emails, job sources, leads, preferences, flow checkpoints |
| `applications/<slug>/` | `jd.md`, `cv.yaml`, `cv.pdf`, page previews |
| `leads/<id>/` | `jd.md` of each found posting |
| `shots/` | apply screenshots |
| `logs/jobpilot.log.<date>` | log file, 14 days kept |

API keys and OAuth tokens live only in the OS keyring (service `jobpilot`).

Other env vars: `JOBPILOT_BRIDGE_PORT` (port of the extension bridge, default 47615; set the same port in the extension's popup), `JOBPILOT_TYPST` (typst path), `JOBPILOT_DRY_RUN=1` (every apply is a dry run), `RUST_LOG`. Tests only: `JOBPILOT_CHROME` (the Chromium the browser tests start), `JOBPILOT_NO_SANDBOX=1` (that Chromium without its sandbox).

## Privacy and safety

- No telemetry. The app talks only to your AI provider, the sites you apply on, the job and trend sources you add (and a company's own site for the job insight), and Google (if you connect Gmail). The browser extension talks only to the app on `127.0.0.1`.
- Your documents are sent to your AI provider when read into your profile; the JD and profile when a CV is written; a posting, your preferences and a compact profile (no name or contact details) when a lead is scored.
- Logs are redacted (API keys, tokens, every keyring secret) and never contain page, JD or email bodies.
- Nothing is submitted until you press Submit on the review screen. **Dry run** fills the form and never submits.
- The LLM only fills values for known form fields; it never picks actions or URLs. Page, JD and email text is untrusted.
- Discovery is read-only and polite, refuses this machine and private networks, does not hide that it is automated (`navigator.webdriver` is true in its tabs), and hands every login, captcha or block page to you.
- The extension is a relay with a fixed list of browser commands: it acts only on tabs it opened, never reads your other tabs, and returns cookie domains, never cookie values. It accepts the app only after you paired it. Keeping discovery tabs off private networks is weaker than the old proxy: each request is checked by host, but Chrome resolves the name again afterwards (`extension/PROTOCOL.md`, "The guard").
- CVs only contain facts from your documents: the profile keeps only facts a document states, and every CV is checked against it.

## Develop

```
cargo test --workspace
bun run ext:build && JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine -- --ignored   # real Chromium with the extension
JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-engine --features test-browser --test e2e_discovery -- --include-ignored  # + dry-run apply
cargo test -p jobpilot-engine --test e2e_discovery                      # source -> lead -> CV -> approve, fake provider and board
```

On Windows, use PowerShell for the individual checks.

```powershell
bun run check
bun run build
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
typst compile --root . --input data=assets/prompts/example-cv.yaml assets/cv.typ "$env:TEMP\jobpilot-example.pdf"
```

Use Git Bash for `scripts/check.sh` and `scripts/risk.sh`. Keep Typst on Git Bash’s `PATH` for the example render. For the browser dry-run fixture, set `$env:JOBPILOT_CHROME` to a Chromium or Chrome for Testing executable and run `cargo test -p jobpilot-engine --features test-browser --test e2e_discovery -- --ignored` after building the extension.

See `AGENTS.md` for the module map, conventions and how to check the UI visually.
