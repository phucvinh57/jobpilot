// What the shell's `discovery_*` commands return (`src-tauri/src/discovery/dto.rs`), plus the
// few shapes the screens keep for themselves. No React, no shell.

import type { CommandError, Problem } from "@/lib/problem";
import type { Insight } from "@/components/InsightBrief";

/** What the user put in the link box (or dropped on the window), classified by the shell. */
export interface LeadInput {
  kind: "url" | "file" | "text";
  value: string;
}

export type CriterionName = "skills" | "domain" | "location" | "salary" | "seniority";

/** One line of "why this score" under a lead's name. */
export interface Reason {
  criterion: CriterionName;
  score: number;
  text: string;
}

/** One criterion of "why it fits". */
export interface Criterion {
  criterion: CriterionName;
  weight: number;
  /** `false` when neither the posting nor the preferences say anything about it. */
  known: boolean;
  score: number;
  reason: string;
}

/** A found job as the queue shows it. */
export interface Lead {
  id: number;
  /** The source that found it; `null` for a job the user added (or a deleted source). */
  sourceId: number | null;
  /** The posting's address; not a web link for a job added from a file or pasted text. */
  url: string;
  company: string;
  role: string;
  location: string;
  /** 0 to 100; `null` when never scored. */
  score: number | null;
  /** RFC 3339. */
  foundAt: string;
  reasons: Reason[];
  /** The five criteria in a fixed order; `null` when the lead has no readable breakdown. */
  criteria: Criterion[] | null;
  insight: Insight | null;
}

/** A job source and how its last run ended. */
export interface Source {
  id: number;
  kind: string;
  /** May be blank. */
  name: string;
  /** The host of its address; the web app never gets the whole address. */
  host: string;
  enabled: boolean;
  sessionId: number | null;
  /** RFC 3339. */
  lastRunAt: string | null;
  /** Free text set by the runner: `ok`, `needs_human`, `failed: ...`. */
  lastStatus: string | null;
}

/** A source paused at a login, captcha or block page. */
export interface Waiting {
  thread: string;
  sourceId: number;
  sourceName: string;
  reason: "login" | "captcha" | "blocked" | "other";
  /** The page to deal with (http(s) only). */
  url: string | null;
}

/** A "Prepare CV" that did not finish. */
export interface FailedPrepare {
  leadId: number;
  company: string;
  role: string;
  message: string;
  problem: Problem;
  /** The application that was already created for the lead. */
  applicationId: number | null;
}

/** What a search found. */
export interface RunSummary {
  /** RFC 3339; blank when unknown. */
  finishedAt: string;
  saved: number;
  waiting: number;
  failed: number;
}

/** Everything the Leads stage shows, read in one go. */
export interface Queue {
  /** Leads waiting for the user, best match first. */
  items: Lead[];
  failed: FailedPrepare[];
  sources: Source[];
  waiting: Waiting[];
  lastRun: RunSummary | null;
  /** A search is running right now (started here or by the daily timer). */
  searching: boolean;
  /** How many jobs the "Dismissed" filter lists. */
  dismissed: number;
}

export interface AddedLead {
  leadId: number;
  state: "new" | "dismissed" | "promoted";
  applicationId: number | null;
}

export type { Dismissed, Restored } from "./dismiss";

/** What a dismissed job was: a lead in the queue, or a job in the CVs stage. */
export type DismissedKind = "lead" | "application";

/** One row of the "Dismissed" list: a lead the user dismissed or the preferences filtered out, or a
 *  job dismissed from the CVs stage. */
export interface DismissedJob {
  kind: DismissedKind;
  /** The lead's or the application's id, after `kind`. */
  id: number;
  company: string;
  role: string;
  /** The posting's address; not a web link for a job added from a file or pasted text. */
  url: string | null;
  /** Why, as the user typed it or the filter said; `null` without a reason. */
  reason: string | null;
  /** RFC 3339. */
  dismissedAt: string;
}

/** How one lead's "Prepare CV" ended. */
export interface PrepareOutcome {
  leadId: number;
  applicationId: number | null;
  /** `Company — Role` of the application. */
  label: string | null;
  error: CommandError | null;
}

/** How a search ended (the daily one reports through an event). */
export interface SearchFinished {
  summary: RunSummary | null;
  error: CommandError | null;
}

export interface SourceKind {
  kind: string;
  label: string;
  /** What to type in the address field. */
  hint: string;
}

/** A job board a careers page points to: offered as a source to add instead (never added by itself). */
export interface BoardHint {
  /** The source kind to add (`greenhouse`, `lever`, ...). */
  kind: string;
  /** The kind's label (`Greenhouse`). */
  label: string;
  /** The board address to add; the shell's check for the kind accepts it. */
  input: string;
}

export interface SessionOption {
  id: number;
  name: string;
}

/** Everything Settings > Job sources shows. */
export interface JobSources {
  sources: Source[];
  /** The sources paused for the user right now. */
  waiting: number[];
  sessions: SessionOption[];
  /** "Search daily while JobPilot is open". */
  daily: boolean;
  kinds: SourceKind[];
}

/** Where a lead stands in "Prepare CV" during this session. */
export type Job = { state: "queued" } | { state: "running"; label: string };
