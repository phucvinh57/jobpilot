# JobPilot

Desktop app (Rust, GPUI, Linux) that turns a job description into a CV tailored from your own `profile.yaml`, lets you edit it, fills and submits the application in a managed browser **only after you confirm**, and tracks replies from Gmail. Works with any OpenAI- or Anthropic-compatible provider.

Flow: paste a JD link (or file) -> Generate CV -> edit / Approve CV -> Review & Submit -> Submit -> status follows your Gmail replies.

## Requirements

- Rust (stable) to build
- [`typst`](https://typst.app) CLI: `cargo install --locked typst-cli` (the app tells you if it is missing)
- Chrome or Chromium for browser automation (Settings > Browser sessions can also download Chrome for Testing, only when you click it)
- Linux with Wayland or X11, xkbcommon, Vulkan, and a Secret Service (gnome-keyring / KWallet, libdbus) for the keyring
- `profile.yaml`: your master profile, the only source of facts for CVs. Found by walking up from the working directory, else `<data dir>/profile.yaml`, else set its path in Settings

## Run

```
cargo run -p jobpilot --release
```

Throwaway state for trying it out: `JOBPILOT_DATA_DIR=/tmp/jp JOBPILOT_SECRETS=memory cargo run -p jobpilot`. Sample applications: `JOBPILOT_DEMO=1`.

## First-time setup (Settings)

1. **AI provider** (Settings > AI providers): add a provider (OpenAI-compatible or Anthropic), base URL, model and API key, then press Test. Until one works, the app shows a "set up a provider" banner.
2. **Browser sessions** (Settings > Browser sessions): press New session, enter a name and the login URL (e.g. `https://www.linkedin.com/login`), press Open browser to log in, sign in by hand in the window that opens, then press Done. JobPilot keeps that browser profile and reuses it for that site (Check re-tests that you are still logged in).
3. **Gmail** (Settings > Gmail, optional): create an OAuth client in Google Cloud Console (APIs & Services > Credentials > OAuth client ID, type Desktop app; enable the Gmail API, read-only scope). Paste the client ID (and secret) into Settings, then Connect Gmail. The consent page opens in your browser and redirects to `http://127.0.0.1:<port>`. The refresh token goes to the keyring.

## Package

```
scripts/package.sh
```

Writes `dist/jobpilot_<version>_<arch>.deb` (needs `cargo install --locked cargo-deb`) and, when `appimagetool` is installed, `dist/JobPilot-<arch>.AppImage` (GPU/Wayland/X11/dbus libraries come from the host). Otherwise that step is skipped with a message. The `typst` CLI is not bundled.

## Where data lives

Everything private stays on this machine, under `$XDG_DATA_HOME/jobpilot` (usually `~/.local/share/jobpilot`, override with `JOBPILOT_DATA_DIR`):

| Path | Content |
| --- | --- |
| `jobpilot.db` | SQLite: applications, timeline, providers (no keys), emails |
| `applications/<slug>/` | `jd.md`, `cv.yaml`, `cv.pdf`, page previews |
| `sessions/<id>/` | browser profiles (mode 0700) |
| `shots/` | apply screenshots |
| `logs/jobpilot.log.<date>` | log file, 14 days kept |
| `chrome/` | Chrome for Testing, if you downloaded it |

API keys and OAuth tokens are only in the OS keyring (service `jobpilot`).

Other env vars: `JOBPILOT_CHROME` (Chrome path), `JOBPILOT_TYPST` (typst path), `JOBPILOT_DRY_RUN=1` (force every apply to be a dry run), `JOBPILOT_NO_SANDBOX=1` (Chrome without its sandbox, for containers/tests), `RUST_LOG` (log filter).

## Privacy

- No telemetry. The app talks only to: your AI provider, the job sites you apply on, Google (if you connect Gmail), and the Chrome-for-Testing index if you press Download.
- The JD text and the parts of `profile.yaml` needed for the CV are sent to your AI provider.
- Logs go to stderr and `logs/`, and pass through a redaction layer (API keys, bearer tokens, OAuth codes/tokens, and every secret the app has read from the keyring are masked). Page, JD and email bodies are not logged.

## Safety

- Nothing is submitted until you press Submit on the review screen; the submit button stays disabled while required fields are empty.
- **Dry run** fills the form and never submits; `JOBPILOT_DRY_RUN=1` forces it everywhere.
- The LLM only supplies values for known field ids. It never picks actions or URLs, and page/JD/email text is treated as untrusted.
- CVs only contain facts from `profile.yaml`.

## Develop

```
cargo build --workspace
cargo test --workspace
JOBPILOT_NO_SANDBOX=1 cargo test -p jobpilot-core -- --ignored   # real Chrome
```

See `AGENTS.md` for the module map, conventions and how to check the UI visually.
