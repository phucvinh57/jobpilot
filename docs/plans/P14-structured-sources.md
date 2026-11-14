# P14 Structured job sources

Goal: find more postings, with better data and fewer browser renders, by reading the structured data that hiring systems and careers pages already publish. There are two parts:

- **P14a**: adapters for four more public job-board APIs: SmartRecruiters, Recruitee, Personio and Workday.
- **P14b**: a better `career_page`: schema.org `JobPosting` JSON-LD, `sitemap.xml`, and spotting the hiring system behind a careers page.

Depends on: P10 (merged). There are no new tables, no migration, no prompt changes and no new graph nodes. Each new adapter is one more `JobSource` behind the existing `source_run` graph.

Out of scope: AI browser agents (Browser Use, Stagehand, Skyvern). They let the model choose actions and URLs from untrusted page text, which the rules forbid. Also out of scope: crawler services, `robots.txt` and conditional requests (a later plan), Workable and Teamtailor (see "Not now"), and LinkedIn and other login-walled boards (decision 1, second step).

## Rules this plan keeps

- **Code picks the address.** Each adapter builds its API URL from a fixed host pattern and a board name the **user** entered, checked like `board_slug`. Nothing read from a page or an API answer chooses a host.
- **What a careers page names is only a suggestion.** When a careers page embeds a Greenhouse or other board (P14b), the app does not switch to reading that board on its own. It suggests a new source, and the user adds it.
- **All requests go through `net::http`.** That means public hosts only, a check on every redirect, the per-host delay, the size cap and the honest User-Agent. Workday's POST is never redirected, as `post_json` already ensures.
- **Untrusted text stays data.** JSON-LD and API descriptions are HTML from third parties. They go through `html_to_text`, and they reach the model only through the existing scoring and insight steps.
- **No disguise.** No browser headers are copied, and no cookies or tokens are scraped from a board page. A 403 or 429 from an API is a plain network error with Retry. It is not a challenge, because there is no page for the user to deal with.

## P14a: job-board API adapters

These endpoints were checked live on 2026-11-14. Each adapter follows `lever.rs`: it caches the listing (`Cached`, `LISTING_TTL`), thins the list with `title_fits_roles`, pages with `page_of` or a native cursor, and returns the posting text from `JobSource::posting` whenever the API includes it.

| Kind | User enters | Listing | Posting text | Posting URL (`Candidate.url`) |
|------|-------------|---------|--------------|-------------------------------|
| `smartrecruiters` | `jobs.smartrecruiters.com/<Company>` or `<Company>` | `GET https://api.smartrecruiters.com/v1/companies/<id>/postings?limit=100&offset=<n>` -> `{ totalFound, content: [{ id, name, location { city, region, country, remote, hybrid, fullLocation }, releasedDate, experienceLevel, typeOfEmployment }] }` | `GET .../postings/<postingId>` -> `jobAd.sections.{companyDescription, jobDescription, qualifications, additionalInformation}.text` (HTML) | `postingUrl` from the detail call, or `https://jobs.smartrecruiters.com/<id>/<postingId>` |
| `recruitee` | `<company>.recruitee.com` or `<company>` | `GET https://<company>.recruitee.com/api/offers/` -> `{ offers: [{ id, title, slug, careers_url, location, city, country, remote, description, requirements, published_at, salary }] }` | `description` + `requirements` from the listing (HTML) | `careers_url` if it is on the company's own domain or `recruitee.com`; otherwise `https://<company>.recruitee.com/o/<slug>` |
| `personio` | `<company>.jobs.personio.de` (or `.com`) or `<company>` | `GET https://<company>.jobs.personio.<tld>/xml` -> `<workzag-jobs><position>` with `id, office, department, name, jobDescriptions, employmentType, seniority, schedule, createdAt` | `jobDescriptions/jobDescription/{name,value}` (HTML) when present. Some companies leave it empty; then `posting()` returns `None` and the runner fetches the page | `https://<company>.jobs.personio.<tld>/job/<id>` |
| `workday` | the board URL: `https://<tenant>.wd<N>.myworkdayjobs.com/[<locale>/]<site>` | `POST https://<host>/wday/cxs/<tenant>/<site>/jobs` with `{"appliedFacets":{},"limit":20,"offset":<n>,"searchText":"<role>"}` -> `{ total, jobPostings: [{ title, externalPath, locationsText, postedOn }] }` | `GET https://<host>/wday/cxs/<tenant>/<site><externalPath>` -> `jobPostingInfo.{ jobDescription (HTML), location, additionalLocations, timeType, startDate, externalUrl }` | `jobPostingInfo.externalUrl` if it is on `<host>`; otherwise `https://<host>/<site><externalPath>` |

Notes per adapter:

- **SmartRecruiters.** The listing has no description, so `posting()` makes one detail call per candidate. That is fine, because candidates are thinned by role first and the runner's page limits apply. Location comes from `fullLocation`, and the adapter adds "(Remote)" or "(Hybrid)". Large companies list thousands of postings (Bosch: 5,091). Page through `offset` with `limit=100`, as the cursor, rather than loading everything: the cursor is the API offset plus an index into the filtered page.
- **Recruitee.** One request returns everything. `careers_url` can point to the company's own domain (`careers.bunq.com/o/...`). It is accepted when it is http(s) and its path ends in `/o/<slug>`; otherwise the adapter builds the address itself.
- **Personio.** The answer is XML, read with `quick-xml` (already a dependency, used by `trends/source/rss.rs`). The listing has no location beyond `office`. The `<tld>` is `de` or `com`, depending on what the user entered (default `de`).
- **Workday.** This is the endpoint the board page itself calls. It is not a documented API, so treat the adapter as best effort: a shape change is a clear error ("Workday answered in an unknown format"), never a guess. `searchText` comes from the preferences: one query per wanted role, deduplicated by `externalPath`. With no roles set, it sends one query with empty `searchText` and a lower page cap. The cursor `"<role index>:<offset>"` stays stable across a resume. The API allows at most 20 per page. `postedOn` ("Posted 15 Days Ago") is shown as written and not parsed. Hosts are checked against `*.myworkdayjobs.com`; `myworkdaysite.com` boards are a later step.

### Code changes (P14a)

| File | Change |
|------|--------|
| `engine/discover/source/{smartrecruiters,recruitee,personio,workday}.rs` | New adapters with unit tests on fixtures |
| `engine/discover/source/http.rs` | `Http::post_json(url, body) -> Result<Value>` (only Workday uses it), `NetHttp` on `net::http::Http::post_json`, and `FakeHttp` answering POSTs by URL + body and recording them |
| `engine/discover/source/mod.rs` | `adapter_for` and `check_source` arms. A `board_host(input, suffix)` helper for the subdomain kinds (`<company>.recruitee.com`, `<company>.jobs.personio.de`, `<tenant>.wdN.myworkdayjobs.com/<site>`), with the same checks as `board_slug` (no dots at the start, `[A-Za-z0-9_-]` only). Tests: bad names, look-alike hosts (`recruitee.com.evil.test`), `..` |
| `engine/discover/sources.rs` | Four rows in `JOB_SOURCE_KINDS` (in display order after Ashby) and `source_url_hint` entries |
| `engine/discover/fixtures/` | `smartrecruiters_postings.json`, `smartrecruiters_posting.json`, `recruitee_offers.json`, `personio.xml`, `workday_jobs.json`, `workday_job.json`. These are trimmed copies of real answers with company names and text replaced by Acme data |
| `src-tauri/src/discovery/dto.rs` | Update the comment listing the kinds. The kinds already reach the UI through `JOB_SOURCE_KINDS`, so there are no web changes besides the fixtures in `src/contexts/discovery/domain/fixtures.ts`, if a test lists the kinds |
| `engine/mail/classify.rs` (P6) | Add `recruitee.com` and `personio.de` / `personio.com` to `ATS_DOMAINS` if missing (this is a one-line, shared fact; ask P6 or do it in the same PR, noting it) |

### Tests (P14a)

For each adapter: the listing is parsed; candidates get URL, title, company and location; roles thin the list; pages follow the cursor, and the same cursor gives the same page; `posting()` returns text, or `None` for empty Personio descriptions; an unknown shape is an error that names the source; a non-2xx answer is a network error. For Workday also: one query per role, dedup across roles, a POST body built only from preferences (asserted on the recorded body), and `externalUrl` on another host is ignored. `check_source` gets accepted and refused inputs for each kind. Nothing touches the network.

## P14b: a better `career_page`

Today `career_page` reads the page, plain or rendered, and keeps the same-site links that look like postings. P14b adds three things, in this order of what the adapter tries.

### 1. `JobPosting` JSON-LD

There is a new module, `engine/discover/source/jsonld.rs`, with pure functions and unit tests:

- `job_postings(html, base) -> Vec<JsonLdPosting>` reads every `<script type="application/ld+json">` block. It accepts a single object, an array, `@graph`, and `ItemList.itemListElement[].item`. `@type` may be a string or an array containing `JobPosting`. It skips blocks that do not parse; malformed JSON is common.
- `JsonLdPosting { url, title, company, location, remote, description_html, date_posted, valid_through, employment_type, salary }`. Location comes from `jobLocation[].address.{addressLocality, addressRegion, addressCountry}`, joined. `jobLocationType == "TELECOMMUTE"` sets remote. Salary comes from `baseSalary.{currency, value.{minValue, maxValue, value, unitText}}` as one line.
- Limits: at most 500 postings per page and at most 64 KB per script block.

It is used in two places:

- **On the listing.** When the careers page carries `JobPosting` items, they become the candidates, ahead of the link heuristic. A candidate's `url` must still pass `same_site(configured, url)`; JSON-LD is page text, so it gets the same rule as a link. A posting whose `validThrough` is in the past is dropped. Location is filled in, which the hard filter already uses.
- **On the posting page** (`discover/fetch.rs`, both the plain path and the browser path). When the page has exactly one `JobPosting` with a description of at least `MIN_POSTING_CHARS`, the text is built from it: a short header (`Title`, `Company`, `Location`, `Remote`, `Employment type`, `Salary`, `Posted`, `Valid through`), then `html_to_text(description)`. Otherwise extraction goes on as today. This gives cleaner text than Readability on many ATS-hosted pages (for example Recruitee, which serves JSON-LD), and it gives salary and dates without the model.

### 2. `sitemap.xml`

When neither the plain page nor JSON-LD gives postings, the adapter tries the site's sitemap before it falls back to the browser. That is cheaper, and nothing needs rendering.

- The address is fixed by code: `<scheme>://<configured host>/sitemap.xml`. If that is a sitemap index, it follows at most 3 child sitemaps whose URLs contain a job word (`JOB_WORDS`), on the same site only. Gzip sitemaps (`.xml.gz`) are skipped in this version.
- Candidates are the `<loc>` entries that pass `same_site` and `looks_like_posting`. Titles come from `title_from_url`. `<lastmod>` is kept for ordering, newest first. There are at most `MAX_POSTINGS`.
- Reading uses `quick-xml` with the existing size cap. A missing or invalid sitemap is not an error: the adapter moves on to the browser as today.

### 3. Spotting the hiring system behind a careers page

There is a new pure function, `detect_boards(html, base) -> Vec<BoardHint { kind, input }>`, that scans links, `iframe src` and `script src` for known board addresses:

| Pattern | Suggests |
|---------|----------|
| `boards.greenhouse.io/<slug>`, `job-boards.greenhouse.io/<slug>`, `boards.greenhouse.io/embed/job_board?for=<slug>`, `boards.greenhouse.io/embed/job_board/js?for=<slug>` | `greenhouse` |
| `jobs.lever.co/<slug>` | `lever` |
| `jobs.ashbyhq.com/<slug>` | `ashby` |
| `jobs.smartrecruiters.com/<Company>` | `smartrecruiters` |
| `<company>.recruitee.com` | `recruitee` |
| `<company>.jobs.personio.de` / `.com` | `personio` |
| `<tenant>.wd<N>.myworkdayjobs.com/[<locale>/]<site>` | `workday` |

Each hint goes through the same `check_source(kind, input)` as a typed source, so a bad name is never suggested. The result is used in two places, and in neither does the app read the board by itself:

- **When adding a career page** (Settings > Job sources): `Core::add_job_source` is unchanged. A new read-only `Core::suggest_boards(url) -> Vec<BoardHint>` (one plain read through `NetPages`, no browser) is called by the add form after a career page is entered. The form shows "This page lists its jobs on Greenhouse (acme). Add it as a Greenhouse source instead?" with one button, which calls the existing add command with that kind and input. The user may still keep the career page.
- **After a run**: when a career-page run found no postings but the page had board hints, the source's `last_status` detail says so ("No postings on the page. It links to a Greenhouse board: add it as a Greenhouse source"). The Job sources list already shows the status line.

`check_site` already refuses a board host typed as a career page. Its message now names the matching kind for all seven boards, using the same pattern table.

### Code changes (P14b)

| File | Change |
|------|--------|
| `engine/discover/source/jsonld.rs` | New: `job_postings`, `posting_text` (header + description), tests on fixtures |
| `engine/discover/source/sitemap.rs` | New: parse a sitemap or an index (pure), `read_sitemap(pages, site)` with the limits above |
| `engine/discover/source/boards.rs` | New: the pattern table, `detect_boards`, and `kind_for_host` (used by `check_site`'s message) |
| `engine/discover/source/career_page.rs` | Listing order: plain page (JSON-LD, then links) -> sitemap -> browser (JSON-LD, then links). It keeps the board hints of the last read |
| `engine/discover/source/web.rs` | `check_site` message from `kind_for_host` |
| `engine/discover/fetch.rs` | The JSON-LD path ahead of `extract_html` / `page_text`, on both the plain and browser paths |
| `engine/discover/sources.rs` | `Core::suggest_boards(url)` |
| `engine/discover/run.rs` | A career-page source with no postings and board hints gets the status detail above (no graph change: the value is set where the run already records `last_status`) |
| `src-tauri/src/discovery/{dto,commands}.rs` | `BoardHintDto { kind, label, input }`, the command `discovery_suggest_boards(url)` |
| `src/contexts/discovery/{api,hooks,ui}/` | The suggestion under the add form (kit `Banner` + a plain `Button`; the form's Add stays the one primary button) |
| `engine/discover/fixtures/` | `career_page_jsonld.html` (an `ItemList` of `JobPosting`, one expired, one on another site), `posting_jsonld.html`, `career_page_embed.html` (a Greenhouse iframe and a Lever link), `sitemap.xml`, `sitemap_index.xml` |

### Tests (P14b)

- `jsonld`: a single object, an array, `@graph` and `ItemList`; `@type` as an array; invalid JSON skipped; an oversized block skipped; HTML in the description turned into text; salary and location lines; expired postings dropped; a posting URL on another site dropped.
- `sitemap`: a plain sitemap, an index with job and non-job children (only job children read, at most 3), another-site `<loc>` dropped, a broken XML file is not an error.
- `career_page`: order of attempts, asserted with `FakePages::asked()`: a JSON-LD page is never rendered and its sitemap never read; a page with no postings reads the sitemap before rendering; challenge behaviour is unchanged.
- `boards`: every pattern, look-alikes refused (`boards.greenhouse.io.evil.test`, `evil.test/boards.greenhouse.io/acme`), and hints passed through `check_source`.
- `fetch`: a posting page with JSON-LD gives the header + description; one without it, or with two postings, is extracted as today.
- Web: `bun run check`; the suggestion banner shows only when a hint exists, and the Add form keeps one primary button.
- UI check (dev-tools script): Settings > Job sources > add a career page with demo data. `JOBPILOT_DEMO=1` gets a seeded hint so the banner can be captured without the network.

## Not now

- **Workable.** In the live check, `apply.workable.com/api/v1/widget/accounts/<x>` answered with a Cloudflare rate-limit error (1015) and `api/v3/.../jobs` answered 404. Its pages may still carry JSON-LD, which P14b reads where a careers page links to them. Revisit with a stable endpoint.
- **Teamtailor.** Its public API needs a company key. Teamtailor careers pages are handled by P14b (JSON-LD and links).
- **Workday on `myworkdaysite.com`** and Workday facet filters (location, job family).
- **A `posted_at` on `Candidate` and leads.** This would sort leads by freshness, but it touches the `leads` table (a migration) and the queue. It should be its own small plan.
- **`robots.txt`, conditional requests (`ETag` / `If-Modified-Since`), content hashes.** These belong to `net::http` and need their own plan.

## Work split and risk

| PR | Content | Risk | Can run in parallel with |
|----|---------|------|--------------------------|
| P14a-1 | SmartRecruiters, Recruitee, Personio + `board_host` | Medium (new outgoing requests, all through `net::http`) | P14b-1 |
| P14a-2 | Workday + `Http::post_json` | Medium | P14b-1 |
| P14b-1 | `jsonld.rs` + the `fetch.rs` path + the `career_page` listing via JSON-LD, `sitemap.rs` | Medium | P14a-* |
| P14b-2 | `boards.rs`, `suggest_boards`, the run status detail, the Settings banner | Medium (crosses engine and the desktop `discovery` context) | after P14a-1 and P14a-2 (the pattern table names their kinds) |

Each PR is in its own worktree and ends with `/pre-pr`. No prompt under `assets/prompts/` changes, so no PR is High. After the last PR: run `e2e_discovery` (unchanged), update `docs/plans/README.md` (status), and update the P10 line in `CLAUDE.md` (`discover/source/` lists the new adapters).
