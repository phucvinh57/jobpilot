// The read models of the shell's `applications` commands (`src-tauri/src/applications/dto.rs`).
// Text that comes from a posting, a CV or an email is plain text here: views show it as text only.

import type { CommandError, Problem } from "@/lib/problem";
import type { Insight } from "@/components/InsightBrief";
import type { Tone } from "@/lib/tokens";

export interface Tag {
  label: string;
  tone: Tone;
}

// ------------------------------------------------------------------ Stages of the Jobs page

/** The application stages of the Jobs page (Leads comes from discovery). */
export type StageKey = "cv" | "applied" | "inProgress" | "done";

/** How a finished application ended: an offer, or a rejection. */
export type Outcome = "success" | "failed";

export interface BoardCard {
  id: number;
  title: string;
  subtitle: string;
  /** The status as a person reads it (`CV ready`). */
  status: string;
  /** What the user does next. */
  next: Tag;
  /** How long ago the card last changed (`today`, `3 d`). */
  age: string;
  /** The interview round the user marked (kept once the application is answered). */
  round: string | null;
  /** `null`: not finished. */
  outcome: Outcome | null;
  /** The stage the card is in. */
  stage: StageKey;
}

export interface BoardStage {
  key: StageKey;
  label: string;
  cards: BoardCard[];
}

export interface ActivityItem {
  id: number;
  appId: number;
  text: string;
  when: string;
  tone: Tone;
}

export interface Board {
  count: number;
  /** Every stage in order, empty ones too. */
  stages: BoardStage[];
  activity: ActivityItem[];
}

// ------------------------------------------------------------------ Application screen

export type PrimaryKey = "generate" | "approveCv" | "reviewAndSubmit" | "continue" | "submit";

/** The one primary button: which action, and whether it can be pressed. */
export interface Primary {
  action: PrimaryKey;
  label: string;
  enabled: boolean;
  loading: boolean;
  /** Why the button is disabled, or what is left to do. */
  hint: string | null;
  /** The Settings section that fixes what blocks the button. */
  settings: string | null;
}

export interface ApplicationHeader {
  id: number;
  company: string;
  role: string;
  status: { key: string; label: string; tone: Tone };
  /** Company, then the location when the analysis found one. */
  subtitle: string;
  hasCv: boolean;
  steps: string[];
  /** The step in progress (`steps.length`: every step is done). */
  stepIndex: number;
  /** The progress line of the running generate / revise job. */
  job: string | null;
  /** The last failure of a job, until the next job starts or it is dismissed. */
  error: CommandError | null;
  /** `null`: read-only (sent, or answered by the company). */
  primary: Primary | null;
  /** A plain "Next job" button replaces the primary one. */
  showNextJob: boolean;
  /** Changes whenever a job rewrote the CV files. */
  cvEpoch: number;
  /** The interview round the user marked; `null`: none marked. */
  round: string | null;
  /** The rounds to pick from (the brief's interview process, or the usual ones). */
  roundOptions: string[];
  /** What the user can mark by hand (the company's answers); empty before it was sent. */
  marks: Mark[];
  /** The stage of the Jobs page the application is in. */
  stage: StageKey;
  outcome: Outcome | null;
}

export type MarkKey = "interview" | "offer" | "rejected";

export interface Mark {
  key: MarkKey;
  label: string;
}

// ------------------------------------------------------------------ JD tab

export interface Jd {
  /** `null`: no JD stored (yet). */
  text: string | null;
  source: string | null;
  sourceIsLink: boolean;
  mustHave: string[];
  niceToHave: string[];
  /** The company brief; `null`: not researched (yet). */
  insight: Insight | null;
}

// ------------------------------------------------------------------ CV tab

export interface Requirement {
  text: string;
  nice: boolean;
  /** Why the profile does not cover it; `null`: covered. */
  note: string | null;
  covered: boolean;
}

export interface Cv {
  hasCv: boolean;
  yaml: string | null;
  pageCount: number;
  /** `cv.yaml` is newer than its previews (or they are missing): render it once. */
  stale: boolean;
  /** The fact guard ran on this CV and found nothing. */
  guardClean: boolean;
  requirements: Requirement[];
  /** The tailoring step reported gaps: offer to add to the profile (Profile > About you). */
  hasGaps: boolean;
}

export interface Rendered {
  pageCount: number;
  /** Fact-guard findings (the edit is kept, the user decides). */
  warnings: string[];
}

// ------------------------------------------------------------------ Timeline tab

export interface TimelineEvent {
  id: number;
  tone: Tone;
  text: string;
  /** The whole event when the row leaves something out. */
  full: string | null;
  when: string;
}

export interface EmailReview {
  id: string;
  title: string;
  body: string;
}

export interface Timeline {
  events: TimelineEvent[];
  reviews: EmailReview[];
}

// ------------------------------------------------------------------ Apply tab

export type ApplyPhase = "idle" | "preparing" | "review" | "submitting" | "done" | "handoff";

export interface ApplyIdle {
  ats: string;
  applied: boolean;
  applyUrl: string | null;
  urlNote: string | null;
  session: string;
  sessionTone: Tone;
  dryRun: boolean;
  dryRunForced: boolean;
  hasProof: boolean;
}

export interface ApplyWorking {
  title: string;
  lines: string[];
}

export interface ApplyNotice {
  tone: Tone;
  title: string;
  body: string;
}

export interface ApplyNote {
  text: string;
  problem: Problem;
}

export type RowInput = "readonly" | "checkbox" | "chips" | "multiChips" | "area" | "line";

export interface ReviewRow {
  id: string;
  label: string;
  required: boolean;
  /** The step of the form the field is on (0-based). */
  step: number;
  editable: boolean;
  /** `text` / `email` / `file` / ... */
  kind: string;
  input: RowInput;
  options: string[];
  placeholder: string;
  /** What the field has now. */
  value: string;
  /** The file name for a file field, else the value. */
  shown: string;
  tag: Tag;
  warning: string | null;
  /** "Needs input" or a warning: highlighted. */
  flagged: boolean;
  /** The answer can be remembered for the next applications. */
  rememberable: boolean;
  remember: boolean;
}

export interface ApplyReview {
  title: string;
  subtitle: string;
  closeLabel: string;
  dryRun: boolean;
  /** `false`: more steps to go (Continue instead of Submit). */
  complete: boolean;
  missing: number;
  notice: ApplyNotice | null;
  notes: ApplyNote[];
  rows: ReviewRow[];
  attached: string | null;
  /** Screenshots of the filled form. */
  screenshots: number;
}

export interface ApplyDone {
  body: string;
  hasProof: boolean;
}

export interface ApplyHandoff {
  title: string;
  reason: string;
  captcha: boolean;
  /** The visible browser is open. */
  headed: boolean;
  hasScreenshot: boolean;
}

export interface Apply {
  phase: ApplyPhase;
  /** Changes when a new review arrives (the editors start from it once). */
  generation: number;
  idle: ApplyIdle | null;
  working: ApplyWorking | null;
  review: ApplyReview | null;
  done: ApplyDone | null;
  handoff: ApplyHandoff | null;
  /** The failure of the last action. */
  error: CommandError | null;
  /** Retry (open and fill the form again) makes sense for `error`. */
  canRetry: boolean;
  /** A short action is running (`Closing the browser...`). */
  busy: string | null;
}

/** Which screenshot of a run: the confirmation of a sent application, the filled form, the page a
 *  handoff stopped at, or the proof of the submit that just happened. */
export type ShotKind = "proof" | "filled" | "handoff" | "done";

/** What the user typed in a review, by field id, and which answers go to the profile. */
export interface ReviewEdits {
  values: Record<string, string>;
  remember: string[];
}

// ------------------------------------------------------------------ events

/** `applications:progress`: the generate / revise job of `appId` moved. */
export interface JobProgress {
  appId: number;
  label: string;
}

/** `applications:apply-progress`: the steps of the form run of `appId` so far. */
export interface ApplyProgress {
  appId: number;
  lines: string[];
}
