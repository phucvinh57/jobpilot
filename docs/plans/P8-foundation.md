# P8 Foundation

Goal: land every change to shared files once, so P9-P13 can run in parallel without conflicts. No user-visible feature except empty screens.

## Changes

1. **Migration** (append in `store/mod.rs`, never edit an existing one):
   - `preferences(id=1, json, updated_at)`: one row, the saved `Preferences` document.
   - `chat_messages(id, thread, role, content, at)`: preference chat history.
   - `sources(id, kind, name, url, purpose, enabled, session_id NULL, last_run_at, last_status)`; `purpose` is `jobs` or `trends`.
   - `leads(id, source_id, url UNIQUE, dedup_key, company, role, location, jd_path, score, score_json, insight_json, state, app_id NULL, found_at)`; `state` is `new | dismissed | promoted`.
   - `trend_items(id, source_id, url UNIQUE, title, published_at, text_path, fetched_at)`.
   - `reports(id, kind, period, title, body_md, citations_json, created_at)`.
   - `applications.insight_json` (new column).
2. **Store files**, one per table: `store/{preferences,chat,sources,leads,trends}.rs` with plain CRUD + tests.
3. **LLM**: add `Llm::chat_json(system, messages: &[Message], schema)` with a default implementation that folds the messages into one user text and calls `complete_json`. Providers need no change; they can override later.
4. **Health**: `Problem::NeedsHuman` (captcha / login / blocked) with help text and a "Open browser" call to action in `problem.rs`.
5. **Navigation**: `Screen::{Queue, Preferences, Trends}` next to `Detail` and `Settings`; sidebar gets three nav entries, each rendering a placeholder view with `.id` + `.role` (for agent-desktop).
6. **Notifications**: move `mail/notify.rs` to `core/notify.rs` (re-exported for mail) so discovery can use it.

## Checks

- `cargo test --workspace`: migration applies on an existing DB (test opens a DB at the previous version, then migrates).
- CRUD tests per new store file.
- `chat_json` default implementation tested with `llm::fake`.
- Screenshot: three new nav entries, placeholders render.

Risk: Medium. Size: small.
