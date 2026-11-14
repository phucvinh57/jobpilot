// What Today shows and does with the queue: the filters, which lead is open, which leads "Prepare
// CV" acts on, the one primary button, and what needs the user.

import type { Problem } from "@/shared";

import { leadTitle } from "./text";
import type { FailedPrepare, Job, Lead, Queue } from "./types";

// ------------------------------------------------------------ filters

/** What the list is narrowed to. The default shows everything. */
export interface QueueFilter {
  /** Only leads found by this source. */
  sourceId: number | null;
  /** Only leads scored at least this (an unscored lead has no score to compare). */
  minScore: number | null;
  /** Only leads whose location contains this text (ignoring case). */
  location: string;
}

export const NO_FILTER: QueueFilter = { sourceId: null, minScore: null, location: "" };

/** Minimum scores the filter offers. */
export const MIN_SCORES: readonly number[] = [50, 70, 85];

export function filterMatches(filter: QueueFilter, lead: Pick<Lead, "sourceId" | "score" | "location">): boolean {
  if (filter.sourceId !== null && lead.sourceId !== filter.sourceId) {
    return false;
  }
  if (filter.minScore !== null && !(lead.score !== null && lead.score >= filter.minScore)) {
    return false;
  }
  const wanted = filter.location.trim().toLowerCase();
  return wanted === "" || lead.location.toLowerCase().includes(wanted);
}

/** The leads that pass `filter`, in their order. */
export function filterItems(items: readonly Lead[], filter: QueueFilter): Lead[] {
  return items.filter((lead) => filterMatches(filter, lead));
}

// ------------------------------------------------------------ selection

/** The lead to show: `requested` when it is listed, else the first one (the best match). */
export function openLeadId(shown: readonly Lead[], requested: number | undefined): number | undefined {
  if (requested !== undefined && shown.some((l) => l.id === requested)) {
    return requested;
  }
  return shown[0]?.id;
}

/** The lead to open once `id` leaves the list: the one after it, else the one before. */
export function neighbourOf(shown: readonly Lead[], id: number): number | undefined {
  const at = shown.findIndex((l) => l.id === id);
  if (at < 0) {
    return undefined;
  }
  return (shown[at + 1] ?? shown[at - 1])?.id;
}

/** The leads "Prepare CV" acts on, in queue order: the ticked ones, else the open one. */
export function targetsOf(shown: readonly Lead[], checked: readonly number[], open: number | undefined): number[] {
  if (checked.length > 0) {
    return shown.map((l) => l.id).filter((id) => checked.includes(id));
  }
  return open !== undefined && shown.some((l) => l.id === open) ? [open] : [];
}

// ------------------------------------------------------------ the primary button

export type QueueAction = "prepareCv" | "searchNow";

export function actionLabel(action: QueueAction): string {
  return action === "prepareCv" ? "Prepare CV" : "Search now";
}

/** What the primary button looks like right now. */
export interface QueuePrimary {
  action: QueueAction;
  enabled: boolean;
  /** Work is running: spinner on the button. */
  loading: boolean;
  /** Why the button is disabled, or what it will do. */
  hint?: string;
}

/** What `queuePrimaryFor` needs to know. */
export interface QueueFacts {
  /** New leads in the list, before the filters. */
  total: number;
  /** Leads that pass the filters. */
  shown: number;
  /** Leads "Prepare CV" would act on: the ticked ones, else the open one. */
  targets: number;
  /** Of those, the ones not already waiting for or getting a CV. */
  idleTargets: number;
  /** A search is running. */
  searching: boolean;
  /** Enabled job sources. */
  sources: number;
}

/** The one primary button of Today: "Prepare CV" while there are leads, "Search now" when the list
 *  is empty. Disabled while the work it starts runs. Every other button on the screen is plain. */
export function queuePrimaryFor(f: QueueFacts): QueuePrimary {
  const ready = (action: QueueAction, hint?: string): QueuePrimary => ({ action, enabled: true, loading: false, hint });
  const blocked = (action: QueueAction, hint: string): QueuePrimary => ({ action, enabled: false, loading: false, hint });
  if (f.total === 0) {
    if (f.searching) {
      return { action: "searchNow", enabled: false, loading: true, hint: "Searching your sources" };
    }
    if (f.sources === 0) {
      return blocked("searchNow", "No job source yet: add one in Settings, then search.");
    }
    return ready("searchNow", "No jobs yet.");
  }
  if (f.targets === 0) {
    return blocked("prepareCv", f.shown === 0 ? "No lead matches the filters." : "Select a lead to prepare its CV.");
  }
  if (f.idleTargets === 0) {
    return { action: "prepareCv", enabled: false, loading: true };
  }
  return ready("prepareCv", f.targets > 1 ? `${f.targets} leads selected, their CVs are written one after the other` : undefined);
}

// ------------------------------------------------------------ what needs the user

/** One "Prepare CV" that did not finish, as the banners show it. */
export interface FailureRow {
  leadId: number;
  title: string;
  message: string;
  problem: Problem;
  /** The application that was already created for the lead. */
  applicationId: number | null;
}

/** A failure of this session that left no thread behind (no provider, ...). */
export interface SessionFailure {
  message: string;
  problem: Problem;
}

/** The failures to show, by lead: those the core remembers (not while the lead is being prepared
 *  again), then the ones of this session it does not, by lead id. */
export function failureRows(
  stored: readonly FailedPrepare[],
  items: readonly Lead[],
  jobs: Readonly<Record<number, Job>>,
  session: Readonly<Record<number, SessionFailure>>,
): FailureRow[] {
  const rows: FailureRow[] = stored
    .filter((f) => jobs[f.leadId] === undefined)
    .map((f) => ({
      leadId: f.leadId,
      title: leadTitle(f.company, f.role),
      message: f.message,
      problem: f.problem,
      applicationId: f.applicationId,
    }));
  for (const [key, failure] of Object.entries(session)) {
    const leadId = Number(key);
    if (rows.some((r) => r.leadId === leadId)) {
      continue;
    }
    const lead = items.find((l) => l.id === leadId);
    rows.push({
      leadId,
      title: lead ? leadTitle(lead.company, lead.role) : `Lead ${leadId}`,
      message: failure.message,
      problem: failure.problem,
      applicationId: null,
    });
  }
  return rows.sort((a, b) => a.leadId - b.leadId);
}

/** What the "NEEDS YOU" header counts: sources waiting at a login, failed CVs and CVs to read. The
 *  sidebar badge shows this same number. */
export function needsYouCount(queue: Queue | undefined, rows: readonly FailureRow[], prepared: number): number {
  return (queue?.waiting.length ?? 0) + rows.length + prepared;
}

/** A "Prepare CV" batch whose results stopped coming (the task ended early): its jobs that never
 *  reported leave the running ones and become failures the user can retry. */
export function dropUnreported(
  jobs: Readonly<Record<number, Job>>,
  failed: Readonly<Record<number, SessionFailure>>,
  unreported: Iterable<number>,
  problem: Problem,
): { jobs: Record<number, Job>; failed: Record<number, SessionFailure> } {
  const nextJobs = { ...jobs };
  const nextFailed = { ...failed };
  for (const id of unreported) {
    if (nextJobs[id] !== undefined) {
      delete nextJobs[id];
      nextFailed[id] = { message: STOPPED, problem };
    }
  }
  return { jobs: nextJobs, failed: nextFailed };
}

/** What a CV that never reported says. */
export const STOPPED = "Preparing the CV stopped before it finished. Try again.";

/** The line under a lead being prepared. */
export function jobLabel(job: Job): string {
  return job.state === "queued" ? "Waiting for its turn" : job.label;
}
