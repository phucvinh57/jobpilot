# Activity diagrams for P8-P13

Status: **draft**.

"Graph engineering" is read here as: each feature is a **graph of small steps over a typed state**, with conditional edges, parallel branches, and **interrupt nodes** where the run stops and waits for the user. The diagrams below are those graphs.

Legend: rounded = start/end, box = step done by the app, slanted = **interrupt, the user acts**, diamond = decision, hexagon = branches run in parallel.

## 1. Overview

```mermaid
flowchart TD
    start([App start]) --> hasPrefs{Preferences saved?}

    subgraph CHAT["Preference chat (P9)"]
        ask["AI asks up to 3 questions"] --> answer[/"User answers, may attach a CV"/]
        answer --> merge["Merge patch into preferences and save"]
        merge --> complete{"Complete and confirmed by user?"}
        complete -- no --> ask
    end

    hasPrefs -- no --> ask
    hasPrefs -- yes --> trigger[/"User presses Search now, or daily run"/]
    complete -- yes --> trigger

    trigger --> fork{{"in parallel"}}
    fork --> discovery["Job discovery (diagram 2)"]
    fork --> trends["Market trends (diagram 4)"]

    discovery --> queue["Queue and apply (diagram 3)"]
    queue -- "dismiss reason" --> merge
    trends -- "add skill or domain" --> ask
    queue --> finish([Applied, status tracked from Gmail])
```

## 2. Job discovery (P10 + P11)

```mermaid
flowchart TD
    start([Run starts with preferences]) --> next["Take next enabled source"]
    next --> search["Source adapter searches candidates"]
    search --> challenge{"Login, captcha or block page?"}

    challenge -- yes --> pause["Pause this source, save its position, notify user"]
    pause --> human[/"User solves it in the visible browser, presses Continue"/]
    human --> search

    challenge -- no --> dedup["Drop postings already in leads or applications"]
    dedup --> fetch["Fetch posting text and store it"]
    fetch --> filter{"Passes hard filters?"}
    filter -- no --> dismissed["Store as dismissed with the reason"]

    filter -- yes --> fork{{"in parallel"}}
    fork --> score["Score per criterion, weighted sum in code"]
    fork --> insight["Insight: domain, company, skills, interview process"]
    score --> lead["Save lead with score and insight"]
    insight --> lead

    lead --> more{"More sources or candidates?"}
    dismissed --> more
    more -- yes --> next
    more -- no --> done([Queue updated])
```

Other sources keep running while one waits at the interrupt.

## 3. Queue, CV and apply (P12 + existing pipeline and apply flow)

```mermaid
flowchart TD
    start([Queue sorted by score]) --> open[/"User opens a lead: brief and score breakdown"/]
    open --> choice{"User decision"}
    choice -- Dismiss --> exclude["Mark dismissed, feed reason back to preferences"]
    exclude --> start

    choice -- "Prepare CV" --> promote["Promote lead to application"]
    promote --> analyze["Analyze JD"]
    analyze --> tailor["Tailor CV from profile.yaml"]
    tailor --> guard{"Only facts from profile.yaml?"}
    guard -- no --> repair["Repair with the violations"]
    repair --> guard
    guard -- yes --> render["Render PDF with Typst"]

    render --> review[/"User reviews the CV"/]
    review --> approve{"Approve?"}
    approve -- "ask for changes" --> tailor
    approve -- yes --> fill["Open the form in the managed browser and fill it"]

    fill --> drivable{"Form drivable, no challenge?"}
    drivable -- no --> manual[/"User finishes in the visible browser"/]
    drivable -- yes --> answers[/"User reviews answers, presses Submit"/]
    answers --> submit["Submit and save proof"]
    submit --> applied([Applied])
    manual --> applied
    applied --> mail["Gmail sync moves status: interview, rejected, offer"]
    applied --> nextLead["Next in queue"]
    nextLead --> start
```

## 4. Market trends (P13)

```mermaid
flowchart TD
    start([Run starts with preferences and trend sources]) --> fork{{"in parallel"}}
    fork --> hn["Hacker News API"]
    fork --> yc["YC directory"]
    fork --> feeds["RSS feeds and user URLs"]
    fork --> key{"Search API key saved?"}
    key -- yes --> reports["Search annual and quarterly reports"]
    key -- no --> skip["Skip report search"]

    hn --> store["Deduplicate and store items"]
    yc --> store
    feeds --> store
    reports --> store
    skip --> store

    store --> extract["Per item: topics and claims with quotes"]
    extract --> count["Count skill frequency in code"]
    count --> synth["Synthesize digest"]
    synth --> cited{"Every point has a citation?"}
    cited -- no --> drop["Drop uncited points"]
    drop --> save["Save report"]
    cited -- yes --> save
    save --> read[/"User reads the digest"/]
    read --> add{"Add a skill or domain to preferences?"}
    add -- yes --> chat([Open preference chat with the suggestion])
    add -- no --> done([Done])
```

A challenge on a trend source uses the same interrupt as diagram 2.

## How this maps to code

| Graph concept | In JobPilot |
|---------------|-------------|
| State | One typed struct per graph (`ChatState`, `DiscoveryState`, `ApplyState`, `TrendState`), saved in SQLite after every node |
| Node | An async function `state -> state`, run through `runtime::spawn`; no UI code inside |
| Edge | A plain Rust `match` on the state; the LLM fills state fields, it never picks the next node or a URL |
| Parallel branch | Tasks joined before the next node (score + insight, trend sources) |
| Interrupt | A node that saves the state and returns `WaitingForUser`; the UI shows the one primary button, and resuming reloads the state |
| Retry | A failed node keeps the last saved state; Retry re-runs that node only |

Proposed change to P8, not yet written into the P8 plan: add a small `core/graph` module (node trait, runner, checkpoint table) so P9-P13 share one runner instead of four hand-written loops. The existing `pipeline::generate` and `apply::flow` already follow this shape (stages, hand-off) and can move onto the runner later.
