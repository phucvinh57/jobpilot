// Sample read models for the screen tests (what the shell would send).

import type { Problem } from "@/lib/problem";

import type {
  Apply,
  ApplicationHeader,
  Board,
  BoardCard,
  Cv,
  Jd,
  Primary,
  ReviewRow,
  Timeline,
} from "@/types/applications";

export const OTHER: Problem = { kind: "other", challenge: null, advice: null, settingsSection: null, retryable: false };

export function primary(over: Partial<Primary> = {}): Primary {
  return {
    action: "approveCv",
    label: "Approve CV",
    enabled: true,
    loading: false,
    hint: null,
    settings: null,
    ...over,
  };
}

export function header(over: Partial<ApplicationHeader> = {}): ApplicationHeader {
  return {
    id: 7,
    company: "Acme",
    role: "Backend Engineer",
    status: { key: "draft", label: "Draft", tone: "neutral" },
    subtitle: "Acme · Berlin",
    hasCv: true,
    steps: ["Lead", "CV review", "Apply", "Submitted", "Replies"],
    stepIndex: 1,
    job: null,
    error: null,
    primary: primary(),
    showNextJob: false,
    cvEpoch: 0,
    round: null,
    roundOptions: ["HR screen", "Technical interview", "System design", "Final interview"],
    marks: [],
    stage: "cv",
    outcome: null,
    dismissible: true,
    dismissed: null,
    ...over,
  };
}

export function card(over: Partial<BoardCard> = {}): BoardCard {
  return {
    id: 1,
    title: "Backend Engineer",
    subtitle: "Acme",
    status: "Draft",
    next: { label: "Approve CV", tone: "accent" },
    age: "today",
    round: null,
    outcome: null,
    stage: "cv",
    ...over,
  };
}

export function board(): Board {
  return {
    count: 6,
    stages: [
      {
        key: "cv",
        label: "CVs",
        cards: [
          card(),
          card({
            id: 2,
            title: "SRE",
            subtitle: "Globex",
            status: "CV ready",
            next: { label: "Review & submit", tone: "accent" },
            age: "2 d",
          }),
        ],
      },
      {
        key: "applied",
        label: "Applied",
        cards: [
          card({
            id: 3,
            title: "Platform Engineer",
            subtitle: "Initech",
            status: "Applied",
            next: { label: "No reply yet", tone: "neutral" },
            age: "5 d",
            stage: "applied",
          }),
        ],
      },
      {
        key: "inProgress",
        label: "In progress",
        cards: [
          card({
            id: 4,
            title: "Data Engineer",
            subtitle: "Umbrella",
            status: "Interview",
            next: { label: "Technical interview", tone: "warn" },
            age: "1 d",
            round: "Technical interview",
            stage: "inProgress",
          }),
        ],
      },
      {
        key: "done",
        label: "Done",
        cards: [
          card({
            id: 5,
            title: "SRE",
            subtitle: "Hooli",
            status: "Offer",
            next: { label: "Offer", tone: "ok" },
            age: "9 d",
            outcome: "success",
            stage: "done",
          }),
          card({
            id: 6,
            title: "Go Developer",
            subtitle: "Vandelay",
            status: "Rejected",
            next: { label: "Rejected", tone: "danger" },
            age: "12 d",
            outcome: "failed",
            stage: "done",
          }),
        ],
      },
    ],
    activity: [
      { id: 11, appId: 3, text: "Initech: moved to Applied", when: "09:05", tone: "accent" },
      { id: 10, appId: 1, text: "Acme: application created", when: "Yesterday", tone: "neutral" },
    ],
  };
}

export function jd(over: Partial<Jd> = {}): Jd {
  return {
    text: "Senior Backend Engineer\n* Rust\n* PostgreSQL\n\nRemote friendly.",
    source: "https://jobs.acme.test/1",
    sourceIsLink: true,
    mustHave: ["Rust"],
    niceToHave: ["Terraform"],
    insight: null,
    ...over,
  };
}

export function cv(over: Partial<Cv> = {}): Cv {
  return {
    hasCv: true,
    yaml: "name: Ada\nrole: Engineer\n",
    pageCount: 1,
    stale: false,
    guardClean: true,
    requirements: [
      { text: "Rust", nice: false, note: null, covered: true },
      { text: "Terraform", nice: true, note: "Nothing in your documents shows it", covered: false },
    ],
    hasGaps: true,
    ...over,
  };
}

export function timeline(over: Partial<Timeline> = {}): Timeline {
  return {
    events: [
      { id: 2, tone: "accent", text: "moved to Applied", full: null, when: "2026-11-02 09:05" },
      { id: 1, tone: "neutral", text: "application created", full: null, when: "2026-11-01 09:00" },
    ],
    reviews: [],
    ...over,
  };
}

export function row(over: Partial<ReviewRow> = {}): ReviewRow {
  return {
    id: "name",
    label: "Full name",
    required: true,
    step: 0,
    editable: true,
    kind: "text",
    input: "line",
    options: [],
    placeholder: "Your answer",
    value: "Ada Lovelace",
    shown: "Ada Lovelace",
    tag: { label: "profile", tone: "ok" },
    warning: null,
    flagged: false,
    rememberable: true,
    remember: false,
    ...over,
  };
}

export function idleApply(over: Partial<Apply> = {}): Apply {
  return {
    phase: "idle",
    generation: 0,
    idle: {
      ats: "Greenhouse",
      applied: false,
      applyUrl: "https://boards.example.com/acme/1",
      urlNote: "found in the job description",
      session: "No login needed: runs in a throwaway browser profile.",
      sessionTone: "neutral",
      dryRun: false,
      dryRunForced: false,
      hasProof: false,
      canSubmitManually: true,
    },
    working: null,
    review: null,
    done: null,
    handoff: null,
    error: null,
    canRetry: true,
    busy: null,
    cvFile: "CV - Acme.pdf",
    ...over,
  };
}

export function reviewApply(rows: ReviewRow[], over: Partial<Apply> = {}): Apply {
  return {
    ...idleApply(),
    phase: "review",
    generation: 1,
    idle: null,
    review: {
      title: "Greenhouse - boards.example.com",
      subtitle: `${rows.length} fields on this step, 0 to check`,
      closeLabel: "Discard",
      dryRun: false,
      complete: true,
      missing: 0,
      notice: null,
      notes: [],
      rows,
      attached: "cv.pdf",
      screenshots: 0,
    },
    canRetry: false,
    ...over,
  };
}
