# P15 Market scan: market first, you second, wishes last

Goal: the trends feature shows what the market in the user's place is hiring for, and what the user should learn to get those jobs. The user's own wishes must not shape that picture.

Depends on: P13 (collection and digest). It reuses the `trends` and `trends_digest` graphs and adds no new node, table or migration.

## Why

Before P15, the user's wishes shaped the digest at every step:

| Where | What it did | Bias |
|---|---|---|
| `report_search::build_queries` | Built searches only from wished domains and target roles | It found reports only on what the user already wanted, and never searched by location. |
| `digest::count` | Counted HN hiring postings together with leads | Discovery had already filtered leads by the user's roles, preferences and chosen sources, so the market numbers repeated the user's wishes. |
| `digest::count` | Raw counts over postings from everywhere | There was no local view, and a month with more postings looked like growth. |
| `hackernews::hiring_posts` | Read only the newest hiring thread | The 6-month window filled only after 6 monthly runs. |
| `digest::synthesize` + prompt | Sent every preference and asked the model to prefer points that match them | The model picked evidence that confirmed the wishes, and never saw the user's real background. |

## The method

1. **Frame from background and place** (`trends/frame.rs`, `MarketFrame`).
   - *Background* comes from `profile.yaml`: the visible skills, the role phrases from the visible positions and the headline, and the compact profile, which has no contact details.
   - *Place* comes from `Preferences.locations` and the work mode: remote counts as local unless the user wants on-site work.
   - The frame reads nothing else from the preferences. Wished skills, avoided skills, wished domains and target roles never decide what is collected or counted. A test asserts this.
2. **Sample the market, not the user's search.**
   - Market numbers come only from HN "Who is hiring" postings.
   - The first run reads the last 6 monthly threads. Threads read in full are not downloaded again; the newest thread is read on every run.
   - Leads are shown as the user's *pipeline*, for comparison, and never counted as market.
   - Report searches are built from place and role phrases.
3. **Two scopes, shares.**
   - *Global* covers every posting. *Local* covers postings that name a preferred location, or say remote when that is allowed.
   - Code labels a skill rising, falling, steady or unclear by comparing its share in the second half of the window with the first half.
   - A label needs at least 30 postings in each half and 5 mentions. Otherwise it is "unclear", and the digest says there is too little data.
4. **Open vocabulary.**
   - The market lists and the gap count the built-in list and the profile's skills. Wished skills are measured with a separate vocabulary, for the wishes table only: a wished skill outside the built-in list never enters the market lists or the gap, so two runs that differ only in wishes give the same market numbers. A built-in skill you wish for is still counted, and marked as wished.
   - Topics that the extraction finds in at least 3 items but that are not in the list are shown as "terms not in the list".
5. **Gap against background.**
   - The gap is judged on the local scope when it has enough postings, otherwise on the global one, and only on postings in the user's field (the role words of the profile).
   - Code puts each skill in one bucket:
     - *build on*: in the profile, and rising or steadily asked for
     - *learn next*: not in the profile, and rising or often asked for
     - *at risk*: in the profile, and falling
   - The model explains a bucket with citations. It cannot move a skill to another bucket.
6. **Wishes last.**
   - Each wished or avoided skill and domain is measured on the gap's scope: direction and mentions, or "no data".
   - The view always shows this table, built by code.
   - The model's `wishes_check` points add cited context. They only get to see the wishes in this block.
7. **Checks in code** (`digest/cite.rs`). The following points are dropped:
   - a rising or falling point that contradicts the direction code computed
   - a `for_you` point about a skill that is not in the gap
   - a `wishes_check` point about anything that is not a wish

## How to use it

- Keep `profile.yaml` current. It is the background the gap is measured against.
- Set the locations you would work in and the work mode. Leave wished skills and domains as they are: they no longer bias the scan.
- Keep Hacker News as a trend source. Add feeds for your region (for Vietnam and Singapore: e27, Tech in Asia). Add a search API key if you want reports.
- Run weekly to follow news, funding and hiring. Make decisions after a quarterly run: read the local scope first, then *learn next* and *at risk*, then the wishes table.
- When a wish shows "falling" or "no data", decide on purpose whether to keep it.

## Limits

- HN hiring postings lean towards US and remote companies, so local numbers for Vietnam or Singapore may be thin. The digest says so instead of guessing. Regional job boards (ITviec, TopDev) are login-walled and remain the second step of decision 1.
- A posting counts as local when it names the place. A posting that only says "APAC" does not count as Singapore.

## Changes

- Engine:
  - `trends/frame.rs` (new)
  - `source/hackernews.rs` (backfill, `CollectLimits::hiring_threads`)
  - `source/report_search.rs` (queries from the frame)
  - `collect.rs`
  - `digest/{count,synthesize,cite}.rs`
  - `digest/mod.rs`: `Direction`, `GapBucket`, `ScopeCounts`, `GapSkill`, `TermCount`, `WishSignal`, `Section::WishesCheck`, `DigestConfig.profile`. Every new field uses `serde(default)`, so old reports still load.
- Desktop `trends` context: DTOs, rules and demo data. Web `src/contexts/trends`: scope switch, shares, direction tags, gap and wishes.
- `assets/prompts/trends.md`: background is given; directions and buckets come from the app; wishes only in `wishes_check`. Because the prompt changes, the PR is High risk and waits for a human.

Risk: Medium for code, High for the prompt.
