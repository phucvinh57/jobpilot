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

## Runner (`core/discover/run.rs`)

1. For each enabled source: `search` -> candidates. One source failing does not stop the others.
2. Dedup: canonical URL (strip tracking params) and `dedup_key = slug(company) + slug(role)` against `leads` and `applications`.
3. Fetch the posting text (`pipeline::ingest`, browser fallback already exists) and save it under `<data_dir>/leads/<id>/jd.md`.
4. Score (below), insert the lead.
5. Progress and counts are reported to the UI; logs carry ids and counts only.

Politeness: per-host delay, small concurrency, per-run cap of pages, one visible window at most.

Trigger (decision 5): "Search now" in the queue, optional daily run while the app is open (`kv: discover.last_run`).

## Challenge hand-off (`browser/challenge.rs`)

- `detect(page) -> Option<Challenge { Login | Captcha | Blocked }>`: login via the existing `probe`; captcha by known widget frames/selectors (reCAPTCHA, hCaptcha, Turnstile) and interstitial titles; blocked by HTTP 403/429 and block-page text.
- On a challenge: pause that source, set `sources.last_status = needs_human`, raise `Problem::NeedsHuman`, send a desktop notification, and show "Open browser" in the queue. The managed browser opens visible on that page; when the user is done they press "Continue" and the source resumes from where it stopped.
- The app never tries to solve or evade a challenge. Repeated challenges disable the source for the day.

## Scoring (`core/discover/score.rs`)

- Hard filters in code first (excluded company, location/remote mismatch, seniority far off): filtered leads are stored as `dismissed` with the reason, so they are not fetched again.
- Then one `complete_json` call per lead: per-criterion scores (skills, domain, location, salary, seniority) with a one-line reason each, against `Preferences` + the compact profile.
- Final score = weighted sum using `Preferences.weights`, computed in code; stored in `score` + `score_json`.

## Checks

- Unit: each adapter against saved fixture responses; dedup; hard filters; weighted sum.
- `challenge::detect` against local fixture pages (login, captcha widget, 403), as `#[ignore]` Chrome tests.
- Runner test with fake sources + `llm::fake`: leads inserted, one source in `needs_human`, others complete.
- No network in tests.

Risk: Medium. Size: large (split into two PRs: runner + ATS/HN adapters, then challenge hand-off + browser adapters).
