# P13 Market trends

Goal: beyond single jobs, show where the market is going: what is being hired for, what is being funded and discussed, what annual/quarterly reports say. Every statement links to its source.

Depends on: P8. Uses `Preferences` (P9) to focus, and leads (P10) as one more data set when present.

## P13a Collection (`core/trends/source/`) - Phase 1, parallel

Sources are rows in `sources` (purpose `trends`), added by the user in the Trends view.

| Adapter | What it collects |
|---------|------------------|
| `hackernews` | top stories and "Who is hiring" threads through the HN Algolia API |
| `yc` | YC company directory: new batches, sectors, hiring flags |
| `rss` | any feed the user adds (engineering blogs, newsletters) |
| `page` | any URL the user adds; fetched and cleaned like a JD |
| `report_search` | annual/quarterly reports (decision 6): enabled when a search API key (Brave or Tavily) is saved in the keyring through `core::secrets`; queries built in code from preferences (domains, roles, year/quarter); results come from a search API; only http/https, read-only, PDF text through `pdf-extract` |

Items go to `trend_items` (deduplicated by URL), text under `<data_dir>/trends/`. Same politeness limits and challenge hand-off as P10.

As built (P13a, `core/trends/`):

- Source rows (`purpose = trends`) have kind `hackernews`, `yc`, `rss` or `page`; `report_search` has no row, it runs while a key is saved in the keyring (`trends:brave_api_key` or `trends:tavily_api_key`, Brave first). Helpers: `Core::{add,remove}_trend_source`, `set_search_key`; run with `Core::collect_trends(period)`; `Period` is an ISO week (`2026-W46`) or a quarter (`2026-Q4`).
- `yc` reads the community `yc-oss` mirror of the directory (`meta.json`, `batches/<slug>.json`): YC has no JSON API of its own. The root is injectable (`Endpoints`).
- No-key skip is the first thing the `report_search` node does, not a route in front of it: the library only joins unconditional edges, so a skipped branch that never reaches `store_items` would hold it back forever.
- Slow sources: each source has a timeout and each collector a budget (`CollectLimits`); what runs out is left out (`SourceStatus::TimedOut`) and the run still ends. `wait_human` (shared `challenge` node) follows `store_items`; P13b continues after it. Only `page` detects a wall (HTTP 401 / 403 / 429 / 503) until P10's `browser/challenge.rs` exists.
- Search results and feed entries are fetched only when http(s) and on a public host.

## P13b Digest and view - Phase 2

- `trends::digest(period)`: map-reduce over the period's items. Per item: `{ topics[], claims[{text, quote}] }`. Then one synthesis call: rising / falling skills, active domains, funding and hiring signals, what it means for this user's preferences. Each point carries `citations[]` (item ids); points without a citation are dropped in code.
- Deterministic numbers next to the prose: skill frequency across leads and hiring threads per month (counted in code, not by the model).
- Stored in `reports` (weekly digest, quarterly summary). Regenerate on demand.
- `desktop/trends.rs`: source list with add/remove, latest digest with clickable citations, a skill-frequency chart, history of past reports.
- Link back to the job flow: "Add to preferences" on a skill or domain opens the chat with that suggestion.

## Graph `trends` (thread `trends:<period>`)

| Node | Does | Next |
|------|------|------|
| `plan` | enabled trend sources, period, queries built from preferences | the collectors, in parallel |
| `hackernews`, `yc`, `feeds` | P13a adapters | join at `store_items` |
| `report_search` | route first: no search API key -> skip | join at `store_items` |
| `store_items` | deduplicate and store; waits for all collectors, with a timeout so a slow source is left out of this run | `extract` |
| `extract` | per item: topics and claims with quotes (batched LLM calls, bounded concurrency) | `count` |
| `count` | skill frequency in code | `synthesize` |
| `synthesize` | one LLM call | `enforce_citations` |
| `enforce_citations` | drop points without a citation, in code | `save_report` -> end |

A collector that meets a login or captcha uses the shared `challenge` node and interrupts like a job source; the digest is built from the other collectors and can be regenerated after the user continues. P13a delivers the collectors and `store_items`; P13b the rest and the view.

## Checks

- Unit: adapters against fixtures, dedup, citation enforcement, frequency counting.
- Graph test with `llm::fake`: an uncited claim is removed; without a search API key `report_search` is skipped; a collector that times out does not fail the run.
- Screenshot of the Trends view on demo data.

Risk: Medium for code, High for the prompt file. Size: medium to large (two PRs: P13a, P13b).
