# P10 Job discovery

Goal: from the saved preferences, find job postings, hand over to the user when a site wants a login or a captcha, and store scored leads.

Depends on: P8. Reads `Preferences` (default if P9 is not merged).

## Sources (`core/discover/source/`)

A source is code that turns preferences into candidate postings. The LLM never picks URLs.

```
trait JobSource { fn search(&self, prefs, ctx) -> Result<Vec<Candidate>>; }
Candidate { url, title, company, location, snippet }
```

First version (decision 1):

| Adapter | How | Needs browser |
|---------|-----|---------------|
| `greenhouse`, `lever`, `ashby` | public board JSON for a company slug | no |
| `hn_hiring` | HN Algolia API, latest "Who is hiring" thread, one comment = one candidate | no |
| `yc` | Work at a Startup listing | maybe |
| `career_page` | user-added URL; same-site links that look like postings | yes if JS-rendered |

Second step: `linkedin`, `itviec`, `topdev` through a managed session (`sessions.session_id` on the source), logged in by the user in Settings > Sessions.

Sources are rows in `sources` (purpose `jobs`), managed in `settings/sources.rs`: add, enable, bind a session, see last run status.

## Graphs (`core/discover/run.rs`)

Two graphs, so that one paused source never blocks the others.

**`discovery`** (thread `discover:<run id>`): `plan` (enabled sources, preferences) -> one `source_run` subgraph per source, in parallel under the concurrency limit -> `summary` (counts, `kv: discover.last_run`). The run is reported as finished when every source is done or waiting.

**`source_run`** (thread `discover:<run id>:<source id>`), state `SourceState { cursor, candidates (append), current, saved, dismissed, challenge }`:

| Node | Does | Next |
|------|------|------|
| `search` | adapter returns the next page of candidates from `cursor` | `challenge` |
| `challenge` | shared node: login, captcha or block page? | found -> `wait_human`, else `dedup` |
| `wait_human` | **interrupt** `Login / Captcha / Blocked`; notification; `sources.last_status = needs_human` | resume -> `search` (same cursor) |
| `dedup` | canonical URL and `slug(company) + slug(role)` against `leads` and `applications` | `next_candidate` |
| `next_candidate` | take one candidate | none left and no more pages -> end; none left -> `search`; else `fetch` |
| `fetch` | posting text (`pipeline::ingest`, browser fallback) -> `<data_dir>/leads/<id>/jd.md` | `challenge` check, then `filter` |
| `filter` | hard filters in code | fail -> `dismiss`, pass -> `score` and `insight` in parallel |
| `score`, `insight` | one LLM call each (P11 provides `insight`) | join -> `save_lead` |
| `save_lead` / `dismiss` | insert the lead, or store it as dismissed with the reason | `next_candidate` |

Limits: per-host delay, pages per run, three challenges in a row disable the source for the day. One source failing ends its own thread with `Failed`; the others continue. Logs carry ids and counts only.

Trigger (decision 5): "Search now" in the queue, optional daily run while the app is open (Settings > Job sources > "Search daily while JobPilot is open", off by default, kv `discover.daily`). The Queue view checks `Core::discover_due` a minute after start and every 15 minutes (setting on, an enabled source, no run in progress, last run or daily attempt over 24 h ago) and then starts the same path as "Search now". On start, the app lists `waiting()` threads so a paused source from an earlier session can be continued or cancelled.

## Challenge hand-off (`browser/challenge.rs`)

- `detect(page) -> Option<Challenge { Login | Captcha | Blocked }>`: login via the existing `probe`; captcha by known widget frames/selectors (reCAPTCHA, hCaptcha, Turnstile) and interstitial titles; blocked by HTTP 403/429 and block-page text.
- On a challenge the `wait_human` node interrupts that source's thread: `Problem::NeedsHuman`, a desktop notification, and "Open browser" in the queue. The managed browser opens visible on that page; "Continue" calls `graph::resume` and the source goes on from its saved cursor.
- The app never tries to solve or evade a challenge. Repeated challenges disable the source for the day.

## Scoring (`core/discover/score.rs`)

- Hard filters in code first (excluded company, location/remote mismatch, seniority far off): filtered leads are stored as `dismissed` with the reason, so they are not fetched again.
- Then one `complete_json` call per lead: per-criterion scores (skills, domain, location, salary, seniority) with a one-line reason each, against `Preferences` + the compact profile.
- Final score = weighted sum using `Preferences.weights`, computed in code; stored in `score` + `score_json`.

## Checks

- Unit: each adapter against saved fixture responses; dedup; hard filters; weighted sum.
- `challenge::detect` against local fixture pages (login, captcha widget, 403), as `#[ignore]` Chrome tests.
- Graph test with fake sources + `llm::fake`: leads inserted, one source thread waiting at `wait_human` while the others finish; resuming it completes the run; a restart in between loses nothing.
- No network in tests.

Risk: Medium. Size: large (split into two PRs: runner + ATS/HN adapters, then challenge hand-off + browser adapters).
