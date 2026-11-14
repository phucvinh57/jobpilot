// Realistic data for the tests of this context (the shape of the demo seed: `JOBPILOT_DEMO=1`).

import type { Challenge, Overview } from "./domain/types";

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
        sections: [
          {
            title: "Rising skills",
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
            title: "Active domains",
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
            title: "Hiring signals",
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
        ],
      },
    },
    chart: {
      skills: [
        {
          name: "Python",
          total: 117,
          description: "117 in total · 19 this month, about the same as last month (28.6%)",
          presence: "absent",
        },
        {
          name: "Kubernetes",
          total: 61,
          description: "61 in total · 12 this month",
          presence: "added",
        },
        {
          name: "Rust",
          total: 52,
          description: "52 in total · 15 this month, up from 17.6% last month",
          presence: "absent",
        },
      ],
      max: 117,
      documents: 398,
      fromMonth: "2026-06",
      toMonth: "2026-11",
    },
    history: [
      { id: 3, title: "Weekly digest 2026-W46", when: "2026-11-14 09:12" },
      { id: 2, title: "Weekly digest 2026-W45", when: "2026-11-07 09:00" },
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
      { kind: "hackernews", label: "Hacker News", needsUrl: false, added: true },
      { kind: "yc", label: "YC companies", needsUrl: false, added: false },
      { kind: "rss", label: "RSS / Atom feed", needsUrl: true, added: true },
      { kind: "page", label: "Web page", needsUrl: true, added: false },
    ],
    searchKeys: [
      { provider: "brave", label: "Brave Search", saved: false },
      { provider: "tavily", label: "Tavily", saved: true },
    ],
    ...patch,
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
