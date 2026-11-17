// Realistic sample data for the screens' tests (not shipped: only the tests import it).

import type { Insight } from "@/components/InsightBrief";

import type { Criterion, FailedPrepare, JobSources, Lead, Queue, Source, Waiting } from "@/types/discovery";

export const INSIGHT: Insight = {
  domain: "B2B payments",
  businessSummary: "Acme sells payment rails to marketplaces.",
  facts: "Series B · 80 people · HQ Berlin",
  requiredSkills: ["Rust", "PostgreSQL", "Kafka"],
  preferredSkills: ["Kubernetes"],
  interviewSteps: ["Recruiter call", "Take-home", "Onsite"],
  interviewNotes: "Two weeks end to end.",
  sources: ["https://acme.test/about"],
};

export const CRITERIA: Criterion[] = [
  { criterion: "skills", weight: 35, known: true, score: 90, reason: "Rust and Postgres match" },
  { criterion: "domain", weight: 20, known: true, score: 40, reason: "payments, not your domain" },
  { criterion: "location", weight: 15, known: true, score: 100, reason: "remote" },
  { criterion: "salary", weight: 15, known: false, score: 0, reason: "" },
  { criterion: "seniority", weight: 15, known: true, score: 70, reason: "" },
];

export function lead(id: number, extra: Partial<Lead> = {}): Lead {
  return {
    id,
    sourceId: 1,
    url: `https://jobs.acme.test/${id}`,
    company: "Acme",
    role: `Backend Engineer ${id}`,
    location: "Remote",
    score: 90 - id * 10,
    foundAt: new Date(Date.now() - 3 * 3_600_000).toISOString(),
    reasons: [
      { criterion: "skills", score: 90, text: "Rust and Postgres match" },
      { criterion: "location", score: 100, text: "remote" },
    ],
    criteria: CRITERIA,
    insight: INSIGHT,
    ...extra,
  };
}

export const SOURCE: Source = {
  id: 1,
  kind: "greenhouse",
  name: "Acme careers",
  host: "boards.greenhouse.io",
  enabled: true,
  sessionId: null,
  lastRunAt: new Date(Date.now() - 3_600_000).toISOString(),
  lastStatus: "ok",
};

export const WAITING: Waiting = {
  thread: "discover:1:2",
  sourceId: 2,
  sourceName: "Beta jobs",
  reason: "login",
  url: "https://beta.test/login",
};

export const FAILED: FailedPrepare = {
  leadId: 7,
  company: "Gamma",
  role: "SRE",
  message: "the fact guard refused the CV\nline two",
  problem: { kind: "other", challenge: null, advice: null, settingsSection: null, retryable: false },
  applicationId: 31,
};

export function queue(extra: Partial<Queue> = {}): Queue {
  return {
    items: [
      lead(1),
      lead(2),
      lead(3, { sourceId: null, url: "manual:abc", score: null, criteria: null, insight: null, reasons: [] }),
    ],
    failed: [],
    sources: [SOURCE],
    waiting: [],
    lastRun: { finishedAt: new Date(Date.now() - 5 * 60_000).toISOString(), saved: 3, waiting: 0, failed: 0 },
    searching: false,
    dismissed: 0,
    ...extra,
  };
}

export function jobSources(extra: Partial<JobSources> = {}): JobSources {
  return {
    sources: [
      SOURCE,
      {
        ...SOURCE,
        id: 2,
        kind: "career_page",
        name: "",
        host: "beta.test",
        enabled: false,
        sessionId: 5,
        lastStatus: "needs_human",
      },
    ],
    waiting: [2],
    sessions: [
      { id: 5, name: "LinkedIn" },
      { id: 6, name: "Work account" },
    ],
    daily: false,
    kinds: [
      { kind: "greenhouse", label: "Greenhouse", hint: "Board URL or name, e.g. boards.greenhouse.io/acme" },
      { kind: "hn_hiring", label: "HN Who is hiring", hint: "No address needed" },
      { kind: "career_page", label: "Job page", hint: "Any page that lists jobs, e.g. https://acme.com/careers" },
    ],
    ...extra,
  };
}
