// Reading the graph runs the Leads stage follows: "Prepare CV" (one thread per lead, `lead:<id>`), the whole
// search (`discovery`) and each source's search (`source_run`).

import type { GraphProgress } from "@/api/events";

export const PREPARE_GRAPH = "prepare_cv";
export const DISCOVERY_GRAPH = "discovery";
export const SOURCE_GRAPH = "source_run";

/** The lead of a `prepare_cv` thread. */
export function parsePrepareThread(thread: string): number | undefined {
  const rest = thread.startsWith("lead:") ? thread.slice("lead:".length) : undefined;
  if (rest === undefined || !/^\d+$/.test(rest)) {
    return undefined;
  }
  return Number(rest);
}

/** The line under a lead whose CV is being written; `undefined` for a step that says nothing new. */
export function prepareLabel(p: GraphProgress): string | undefined {
  if (p.kind === "nodeStarted" && p.node === "promote") {
    return "Creating the application";
  }
  if (p.kind === "nodeStarted" && p.node === "generate") {
    return "Writing the CV";
  }
  if (p.kind === "detail" && p.text !== null) {
    return p.text;
  }
  return undefined;
}

/** Does this step change what the Leads stage shows? A source that ended, paused or failed has saved its
 *  leads by then; a whole search starting or ending moves the "searching" line. */
export function movesTheQueue(p: GraphProgress): boolean {
  if (p.graph === SOURCE_GRAPH) {
    return p.kind === "done" || p.kind === "waiting" || p.kind === "failed";
  }
  if (p.graph === DISCOVERY_GRAPH) {
    return p.kind === "nodeStarted" || p.kind === "done" || p.kind === "failed" || p.kind === "cancelled";
  }
  return false;
}
