// Realistic data for the trends tests (the shape of the demo seed: `JOBPILOT_DEMO=1`).

import type { Cadence, Chart, ChartScope, Challenge, DigestSection, Overview, ReportRow } from "@/types/trends";

export const ACME = "Acme | Senior Rust Engineer | REMOTE | robotics platform";
export const GLOBEX = "Globex | Platform Engineer | Berlin | Rust, Go, Kubernetes";

export function challengeFixture(patch: Partial<Challenge> = {}): Challenge {
  return {
    period: "2026-W46",
    sourceName: "Example annual report",
    kind: "login",
    what: "needs you to sign in",
    url: "https://reports.example.com/annual-2026.html",
    ...patch,
  };
}

const PERIOD_TITLES: Record<string, { title: string; cadence: Cadence }> = {
  "2026-W46": { title: "Week of 9 November", cadence: "weekly" },
  "2026-W45": { title: "Week of 2 November", cadence: "weekly" },
  "2026-M11": { title: "November 2026", cadence: "monthly" },
  "2026-Q4": { title: "Q4 2026", cadence: "quarterly" },
  "2026": { title: "2026", cadence: "yearly" },
};

/** A saved report; its title and cadence follow `period` (one of the labels above). */
export function reportRowFixture(patch: Partial<ReportRow> = {}): ReportRow {
  const period = patch.period ?? "2026-W46";
  const known = PERIOD_TITLES[period] ?? { title: period, cadence: "weekly" };
  return {
    id: 3,
    title: "Weekly digest 2026-W46",
    when: "2026-11-14 09:12",
    period,
    periodTitle: known.title,
    cadence: known.cadence,
    points: 6,
    rerun: true,
    ...patch,
  };
}

export function overviewFixture(patch: Partial<Overview> = {}): Overview {
  return {
    period: { label: "2026-W46", title: "Week of 9 November" },
    summary: "Week of 9 November · from 6 postings and articles in your sources",
    digest: {
      reportId: 3,
      title: "Weekly digest 2026-W46",
      when: "2026-11-14 09:12",
      itemsRead: 6,
      body: {
        kind: "data",
        dropped: 1,
        uncatalogued: [
          { term: "Temporal", items: 5, kind: "hiring postings", rising: true },
          { term: "dbt", items: 4, kind: "hiring postings", rising: false },
          { term: "vector databases", items: 14, kind: "Hacker News stories", rising: false },
        ],
        sections: [
          {
            key: "rising_skills",
            title: "Rising skills",
            gap: null,
            wishes: null,
            points: [
              {
                index: 0,
                number: 1,
                topic: "Rust",
                text: "Rust shows up in more postings and in engineering write-ups about latency.",
                quote: "cut tail latency in half with Rust",
                citations: [
                  { number: 1, title: ACME, url: "https://news.example.com/item/1001" },
                  { number: 2, title: GLOBEX, url: "https://news.example.com/item/1002" },
                ],
                suggest: { field: "skills", presence: "absent" },
              },
              {
                index: 1,
                number: 2,
                topic: "Kubernetes",
                text: "Platform roles keep asking for Kubernetes next to Rust and Go.",
                quote: "",
                citations: [{ number: 2, title: GLOBEX, url: "https://news.example.com/item/1002" }],
                suggest: { field: "skills", presence: "added" },
              },
            ],
          },
          {
            key: "domains",
            title: "Active domains",
            gap: null,
            wishes: null,
            points: [
              {
                index: 3,
                number: 3,
                topic: "fintech",
                text: "Fintech companies are hiring backend engineers while funding slows.",
                quote: "",
                citations: [{ number: 3, title: "Fintech outlook 2026", url: null }],
                suggest: { field: "domains", presence: "avoided" },
              },
            ],
          },
          {
            key: "hiring",
            title: "Hiring signals",
            gap: null,
            wishes: null,
            points: [
              {
                index: 6,
                number: 4,
                topic: "",
                text: "Remote Rust roles are posted in the monthly hiring thread.",
                quote: "Senior Rust Engineer",
                citations: [{ number: 1, title: ACME, url: "https://news.example.com/item/1001" }],
                suggest: null,
              },
            ],
          },
          gapSection(),
          wishesSection(),
        ],
      },
    },
    chart: chartFixture(),
    history: [
      reportRowFixture({ id: 5, title: "Monthly digest 2026-M11", period: "2026-M11" }),
      reportRowFixture(),
      reportRowFixture({ id: 2, title: "Weekly digest 2026-W45", when: "2026-11-07 09:00", period: "2026-W45" }),
    ],
    challenges: [],
    sources: [
      {
        id: 1,
        name: "Hacker News",
        enabled: true,
        description: "Hacker News",
        statusText: "last run: ok",
        tag: { label: "ok", tone: "ok" },
      },
      {
        id: 3,
        name: "Example engineering blog",
        enabled: false,
        description: "RSS / Atom feed · blog.example.com",
        statusText: "not collected yet",
        tag: { label: "not run", tone: "neutral" },
      },
    ],
    sourceKinds: [
      { kind: "hackernews", label: "Hacker News", needsUrl: false, needsName: false, added: true },
      { kind: "yc", label: "YC companies", needsUrl: false, needsName: false, added: false },
      { kind: "rss", label: "RSS / Atom feed", needsUrl: true, needsName: false, added: true },
      { kind: "page", label: "Web page", needsUrl: true, needsName: false, added: false },
      { kind: "person", label: "Person to follow", needsUrl: false, needsName: true, added: false },
    ],
    ...patch,
  };
}

const MONTHS = ["2026-06", "2026-07", "2026-08", "2026-09", "2026-10", "2026-11"];
const MONTH_LABELS = ["Jun", "Jul", "Aug", "Sep", "Oct", "Nov"];

function months(documents: number[]) {
  return MONTHS.map((month, i) => ({ month, label: MONTH_LABELS[i] ?? month, documents: documents[i] ?? 0 }));
}

export function localScope(patch: Partial<ChartScope> = {}): ChartScope {
  return {
    key: "local",
    label: "Local",
    covers: "Ho Chi Minh City, Singapore or remote",
    documents: 593,
    months: months([92, 98, 88, 104, 110, 101]),
    maxPercent: 21,
    note: null,
    skills: [
      {
        name: "Python",
        total: 124,
        percent: 21,
        share: "21%",
        description:
          "124 in total · 20 so far this month · last month 20.0% of documents, about the same as the month before (21.2%)",
        now: "21.5%",
        range: "18.0% to 25.4%",
        presence: "absent",
        direction: { label: "steady", tone: "accent" },
        inProfile: true,
        wished: false,
      },
      {
        name: "Kubernetes",
        total: 79,
        percent: 13,
        share: "13%",
        description:
          "79 in total · 15 so far this month · last month 14.5% of documents, up from 13.5% the month before",
        now: "14.4%",
        range: "11.2% to 18.3%",
        presence: "added",
        direction: { label: "rising", tone: "ok" },
        inProfile: false,
        wished: true,
      },
      {
        name: "Rust",
        total: 56,
        percent: 9,
        share: "9.4%",
        description:
          "56 in total · 13 so far this month · last month 10.9% of documents, up from 9.6% the month before",
        now: "11.2%",
        range: "8.6% to 14.4%",
        presence: "absent",
        direction: { label: "rising", tone: "ok" },
        inProfile: true,
        wished: true,
      },
    ],
    ...patch,
  };
}

export function globalScope(patch: Partial<ChartScope> = {}): ChartScope {
  return {
    key: "global",
    label: "Global",
    covers: "every posting",
    documents: 2106,
    months: months([340, 355, 330, 362, 371, 348]),
    maxPercent: 20,
    note: null,
    skills: [
      {
        name: "Python",
        total: 427,
        percent: 20,
        share: "20%",
        description: "427 in total · 70 so far this month",
        now: "20.1%",
        range: "18.5% to 21.8%",
        presence: "absent",
        direction: { label: "steady", tone: "accent" },
        inProfile: true,
        wished: false,
      },
      {
        name: "PHP",
        total: 96,
        percent: 5,
        share: "4.6%",
        description: "96 in total · 10 so far this month",
        now: "3.2%",
        range: "2.4% to 4.2%",
        presence: "avoided",
        direction: { label: "falling", tone: "warn" },
        inProfile: true,
        wished: true,
      },
    ],
    ...patch,
  };
}

export function chartFixture(patch: Partial<Chart> = {}): Chart {
  return {
    scopes: [localScope(), globalScope()],
    scope: "local",
    fromMonth: "2026-06",
    toMonth: "2026-11",
    marks: true,
    pipeline:
      "Your own leads, for comparison: 26 over the window; they name Rust, Go, Kubernetes most. They are not counted in the market above.",
    coverage: [
      "Counted 2106 hiring postings over 2026-06 to 2026-11.",
      "Not compared with the period before (too few items or too uneven): YC companies.",
    ],
    partial: false,
    ...patch,
  };
}

/** What a report whose counts did not see everything says under its chart. */
export const PARTIAL_COVERAGE = [
  "Counted 2106 hiring postings over 2026-06 to 2026-11.",
  "14 Hacker News items could not be read; some may have been postings.",
  "In this run the sources left 120 items unread.",
  "Not compared with the period before (too few items or too uneven): YC companies.",
];

/** A chart of a report saved before the app knew the user's place and background. */
export function oldChartFixture(): Chart {
  return {
    scopes: [
      {
        key: "global",
        label: "Global",
        covers: "every posting",
        documents: 398,
        months: months([60, 64, 62, 68, 70, 74]),
        maxPercent: 29,
        note: null,
        skills: [
          {
            name: "Python",
            total: 117,
            percent: 29,
            share: "29%",
            description: "117 in total · 19 this month, about the same as last month (28.6%)",
            now: null,
            range: null,
            presence: "absent",
            direction: null,
            inProfile: false,
            wished: false,
          },
          {
            name: "Rust",
            total: 52,
            percent: 13,
            share: "13%",
            description: "52 in total · 15 this month, up from 17.6% last month",
            now: null,
            range: null,
            presence: "added",
            direction: null,
            inProfile: false,
            wished: false,
          },
        ],
      },
    ],
    scope: "global",
    fromMonth: "2026-06",
    toMonth: "2026-11",
    marks: false,
    pipeline: null,
    coverage: [],
    partial: false,
  };
}

function gapSection(): DigestSection {
  return {
    key: "for_you",
    title: "Your gap: build on, learn next, at risk",
    points: [
      {
        index: 7,
        number: 5,
        topic: "Kubernetes",
        text: "Kubernetes is the skill platform roles ask for most next to Rust and Go.",
        quote: "",
        citations: [{ number: 2, title: GLOBEX, url: "https://news.example.com/item/1002" }],
        suggest: null,
      },
    ],
    gap: {
      scope: "local",
      hint: "Judged by the app on postings in Ho Chi Minh City, Singapore or remote for backend engineer.",
      groups: [
        {
          bucket: "build_on",
          title: "Build on",
          hint: "In your profile, and rising or steadily asked for.",
          empty: "No skill of your profile is in steady demand here yet.",
          skills: [
            {
              skill: "Python",
              share: "21%",
              now: "21.5%",
              range: "18.0% to 25.4%",
              direction: { label: "steady", tone: "accent" },
            },
            {
              skill: "Rust",
              share: "9.4%",
              now: "13.0%",
              range: "9.0% to 22.0%",
              direction: { label: "rising", tone: "ok" },
            },
          ],
        },
        {
          bucket: "learn_next",
          title: "Learn next",
          hint: "Not in your profile, and rising or often asked for in your field.",
          empty: "Nothing your field asks for is missing from your profile.",
          skills: [
            {
              skill: "Kubernetes",
              share: "13%",
              now: "14.4%",
              range: "11.2% to 18.3%",
              direction: { label: "rising", tone: "ok" },
            },
          ],
        },
        {
          bucket: "at_risk",
          title: "At risk",
          hint: "In your profile, and asked for less and less.",
          empty: "No skill of your profile is falling.",
          skills: [],
        },
      ],
    },
    wishes: null,
  };
}

function wishesSection(): DigestSection {
  return {
    key: "wishes_check",
    title: "Your wishes vs the market",
    points: [],
    gap: null,
    wishes: {
      hint: "Judged by the app on postings in Ho Chi Minh City, Singapore or remote.",
      wishes: [
        {
          wish: "PHP",
          kind: "skill",
          avoided: true,
          signal: { label: "falling", tone: "warn" },
          detail: "29 postings",
        },
        {
          wish: "fintech",
          kind: "domain",
          avoided: false,
          signal: { label: "steady", tone: "accent" },
          detail: "18 postings",
        },
        {
          wish: "climate tech",
          kind: "domain",
          avoided: false,
          signal: { label: "no data", tone: "neutral" },
          detail: "no posting names it",
        },
      ],
    },
  };
}

/** A promise settled by hand: a command that is still running. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
