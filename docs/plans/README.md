# Product plans: preferences, discovery, queue, market trends

Status: **decisions 1, 2, 3 and 6 confirmed on 2026-11-13**; 4 and 5 use the recommended default until changed.

## Status

Updated 2026-11-14: every plan is merged. On `main` unless noted.

| Plan | PR | State | Notes |
|------|----|-------|-------|
| P8 foundation | #10 (tables, preferences model, `chat_json`, `NeedsHuman`, nav), #11 (graph runtime) | merged | |
| P9 preference chat | #13 | merged | Trends suggestions go into the chat's message box (#22) |
| P10 discovery | #15 (runner, ATS and HN sources, scoring, `net::http`), #18 (challenge hand-off, SSRF guard proxy, YC and career-page sources, Settings > Job sources) | merged | Optional daily run while the app is open (decision 5): #20 |
| P11 job insight | #14 | merged | |
| P12 queue | #17 | merged | Queue, Prepare CV, Dismiss with exclusion, Next in queue |
| P13a trend collection | #12 | merged | `core/trends/` collectors and the `trends` graph; no screen |
| P13b digest and view | #16 | merged | |
| Phase 3 end-to-end and docs | #19 | merged | `crates/core/tests/e2e_discovery.rs`, CLAUDE.md / README updates |
| No browser disguise | #21 | merged | The login, apply and JD-reading browsers no longer hide automation either |
| P14 structured job sources | #35 (SmartRecruiters, Recruitee, Personio), #33 (Workday), #34 (JSON-LD, sitemap), board detection (this PR) | merged | SmartRecruiters, Recruitee, Personio, Workday adapters; JSON-LD, sitemap and board detection for career pages |

End-to-end check (fake provider and fake Greenhouse board on loopback, real typst, real Chrome for the apply step): source -> `discover_now` -> leads with score and insight -> `prepare_cv` -> Approve CV -> dry-run apply on a local form passes. It was checked before P9 and P13b merged: discovery and the queue work with default preferences, as planned. Still unchecked end to end: any live site, the live provider, Gmail, and the P9 / P13b screens with a live provider.

## Intent vs. what exists today

| # | Intent | Today | Gap |
|---|--------|-------|-----|
| 1 | AI chat that asks questions, takes an uploaded CV as context, and saves job-search preferences | Nothing. `Llm` only has single-turn `complete_json`. PDF text extraction exists (`pipeline/ingest`). | Whole feature: chat loop, preference model, storage, view. |
| 2a | Browser searches for jobs from the preferences | Nothing. The app only takes a JD link/file/text the user gives it. | Sources, crawl runner, dedup. |
| 2b | Login / captcha / anti-bot -> notify the user to do it themselves | Login is handled (`browser/probe`, Settings > Sessions, `Handoff` in apply). No captcha / block-page detection. Desktop notifications exist (`mail/notify`). | Challenge detection + a "needs you" handoff during discovery. |
| 2c | Suitable jobs go to a queue sorted by match | Sidebar lists applications by creation. No score, no "found but not started" state. | `leads` table, scoring, queue view. |
| 2d | Per job: business domain, company info, skill list, interview process | `Analysis` has company, role, location, must/nice-to-have, key terms, seniority. | Domain, company info, interview process. |
| 2e | Per job: a CV for that JD only, from the base profile | **Done** (`pipeline`: analyze -> tailor -> guard -> render), for one JD at a time on user action. | Trigger it from the queue. |
| 3 | User reviews the CV, clicks one button, the app applies | **Done**: Approve CV -> Review & Submit -> Submit (`apply/flow`), with manual hand-off when the form cannot be driven. | Only the entry from the queue. |
| 4 | Market trends from sources the user adds (YC, Hacker News) + annual/quarterly reports | Nothing. | Whole feature. |

So: items 2e and 3 already match. Items 1, 2a-2d and 4 are new.

## Plans

Numbering continues the existing P1-P7 ownership scheme in `CLAUDE.md`.

| Plan | What | Owns |
|------|------|------|
| [P8](P8-foundation.md) | Shared groundwork: graph runtime ([library choice](graph-libraries.md)), migration, navigation, multi-turn LLM call, `NeedsHuman` problem | `core/graph/`, `store/` new files, `app.rs` nav, `llm/mod.rs` (one method), `health` |
| [P9](P9-preferences-chat.md) | Preference chat + CV upload + saved preferences | `core/prefs/`, `desktop/prefs.rs`, `assets/prompts/preferences.md` |
| [P10](P10-discovery.md) | Job sources, crawl runner, challenge hand-off, scoring | `core/discover/`, `browser/challenge.rs`, `settings/sources.rs` |
| [P11](P11-job-insight.md) | Domain, company info, skills, interview process per job | `pipeline/insight.rs`, `detail/jd.rs` |
| [P12](P12-queue.md) | Queue view, lead -> application, CV preparation, entry to apply | `desktop/queue.rs`, `core/discover/promote.rs` |
| [P13](P13-market-trends.md) | Trend sources, report search, cited digest | `core/trends/`, `desktop/trends.rs`, `assets/prompts/trends.md` |
| [P14](P14-structured-sources.md) | More job-board API adapters; JSON-LD, sitemap and board detection for career pages | `engine/discover/source/`, `engine/discover/fetch.rs`, desktop `discovery` context (Job sources) |

## Execution flow

Activity diagrams of the four flows, with the points where the app waits for the user: [activity-diagram.md](activity-diagram.md) (draft).

```
Phase 0   P8 foundation                     alone, first (touches shared files once)
            |
Phase 1   P9 chat | P10 discovery | P11 insight | P13a trend collection     in parallel
            |          |               |             |
Phase 2   P12 queue (needs P10 + P11)   |   P13b digest + view (uses P9 preferences if present)
            |
Phase 3   end-to-end run on demo data + fake LLM, update CLAUDE.md / README (High: human review)   <- done for what is on main, see Status
```

- P8 goes first so that no Phase 1 plan edits `store/mod.rs`, `app.rs` or the `Llm` trait.
- Phase 1 plans touch disjoint files. P10 and P11 work without P9 by reading a default `Preferences`.
- P12 is last of the job flow: it only wires P10 leads + P11 insight into the existing pipeline and apply flow.
- One worktree and one PR per plan; each ends with `/pre-pr`. Prompts under `assets/prompts/` are High risk and wait for a human.

## Rules that shape the design

- **No bypassing anti-bot measures.** Captcha, login and block pages are detected, the run for that source pauses, the user is notified and solves it in the visible managed browser. No solver services, no fingerprint spoofing.
- **Untrusted text.** Job pages, HN comments and reports are data. Code decides which URLs are visited (source adapters, search results restricted to http/https, read-only). The LLM only extracts, scores and summarises into fixed schemas.
- **Facts only from `profile.yaml`.** An uploaded CV is chat context; it never becomes CV facts by itself (see decision 4).
- **Forward-only statuses stay as they are.** A found job is a `lead` (own table), not a new application status; it becomes an application when a CV is prepared.
- **Flows are graphs.** Each plan lists its graph (nodes, edges, interrupts, thread id). Edges are Rust code over the state; a paused or failed run resumes from its checkpoint.
- **Cost is bounded.** Discovery scores and summarises each lead (two LLM calls); a CV is written only when the user asks for it (decision 2).

## Decisions

1. **Job sources for the first version (confirmed).** Public ATS boards (Greenhouse, Lever, Ashby), HN "Who is hiring", YC Work at a Startup, plus user-added career-page URLs. Login-walled boards (LinkedIn, ITviec, TopDev) in a second step through managed sessions.
2. **When a CV is written (confirmed).** Only on demand: the queue shows score and summary, "Prepare CV" writes the CV.
3. **Apply (confirmed).** Keep the current flow (app fills the form, user reviews answers and presses Submit; manual hand-off when it cannot).
4. **Uploaded CV (default, not yet confirmed).** context for the chat only; a separate, explicit "Import into profile" action shows a diff the user approves before `profile.yaml` changes.
5. **When discovery runs (default, not yet confirmed).** a "Search now" button plus an optional daily run while the app is open.
6. **Market reports search (confirmed).** Fixed APIs first (HN Algolia, YC directory, RSS, user URLs); general web search through an optional search API key (Brave / Tavily) rather than driving a search engine in the browser, which triggers captchas.
7. **Graph runtime (confirmed).** Every new flow is an `adk-graph` graph behind the `core/graph` facade, with checkpoints in the app's SQLite file. See [graph-libraries.md](graph-libraries.md).
