# JobPilot

Desktop app that turns a job description into a tailored CV (from your own `profile.yaml`), lets you edit it, fills and submits the application in a managed browser after you confirm, and tracks replies from Gmail. Works with any OpenAI- or Anthropic-compatible provider.

## Requirements

- Rust (stable)
- [`typst`](https://typst.app) CLI on `PATH`
- Chrome or Chromium (for browser automation)
- Linux: wayland/X11, xkbcommon, vulkan and a Secret Service (libdbus) for the keyring

## Run

```
cargo run -p jobpilot
```

Private state lives in `$XDG_DATA_HOME/jobpilot` (SQLite DB, generated CVs, browser profiles). API keys and tokens go to the OS keyring. Set `JOBPILOT_DEMO=1` to start with sample applications.

## Develop

```
cargo build --workspace
cargo test --workspace
```

See `AGENTS.md` for the layout and rules.
