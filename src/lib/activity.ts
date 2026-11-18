// The sidebar's list of background jobs: how a graph step changes it. The shell names the jobs
// (`shared_activity`); between two reads, a job's own progress text is shown as it arrives.

import type { GraphProgress } from "@/api/events";
import type { Activity } from "@/types/shell";

export interface ActivityState {
  items: Activity[];
  /** The last progress text of a thread's running step ("Tailoring the CV"), by thread. */
  details: Record<string, string>;
}

export const NO_ACTIVITY: ActivityState = { items: [], details: {} };

/** What a graph step does to the list: the new state, and whether the list must be read again. */
export interface Step {
  state: ActivityState;
  reread: boolean;
}

function without(details: Record<string, string>, thread: string): Record<string, string> {
  if (!(thread in details)) {
    return details;
  }
  const { [thread]: _, ...rest } = details;
  return rest;
}

/** A running job says its own progress text, when it reported one. */
function withDetail(item: Activity, details: Record<string, string>): Activity {
  const text = item.state === "running" ? details[item.id] : undefined;
  return text === undefined || text === item.status ? item : { ...item, status: text };
}

/** A fresh read of the list: jobs keep the progress text of the step they are still at. */
export function loaded(state: ActivityState, items: Activity[]): ActivityState {
  const details: Record<string, string> = {};
  for (const item of items) {
    const text = state.details[item.id];
    if (item.state === "running" && text !== undefined) {
      details[item.id] = text;
    }
  }
  return { items: items.map((item) => withDetail(item, details)), details };
}

/** A graph step:
 *  - a progress text replaces the status line of that thread's job at once;
 *  - a new step, a pause or an end means the list changed: read it again (an ended job leaves the
 *    list at once, so a failed read does not keep it);
 *  - a step that only finished changes nothing (the next one starting, or the end, follows). */
export function onProgress(state: ActivityState, p: GraphProgress): Step {
  switch (p.kind) {
    case "detail": {
      if (p.text === null || p.text.trim() === "") {
        return { state, reread: false };
      }
      const details = { ...state.details, [p.thread]: p.text };
      return { state: { items: state.items.map((item) => withDetail(item, details)), details }, reread: false };
    }
    case "nodeStarted":
    case "waiting":
      return { state: { items: state.items, details: without(state.details, p.thread) }, reread: true };
    case "done":
    case "failed":
    case "cancelled":
      return {
        state: {
          items: state.items.filter((item) => item.id !== p.thread),
          details: without(state.details, p.thread),
        },
        reread: true,
      };
    case "nodeFinished":
    case "nodeFailed":
      return { state, reread: false };
  }
}

/** How many jobs wait for the user. */
export function waitingCount(items: readonly Activity[]): number {
  return items.filter((item) => item.state === "waiting").length;
}

/** The indicator's accessible name: "2 background jobs", "3 background jobs, 1 waiting for you". */
export function activityLabel(items: readonly Activity[]): string {
  const jobs = items.length === 1 ? "1 background job" : `${items.length} background jobs`;
  const waiting = waitingCount(items);
  return waiting > 0 ? `${jobs}, ${waiting} waiting for you` : jobs;
}
