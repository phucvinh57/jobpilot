// What a background run does, what Retry repeats, and what the screen says while it runs.

/** A run the user starts. */
export type JobKind =
  /** Collect, then build the digest. */
  | "collect"
  /** The digest alone, from the stored items. */
  | "regenerate"
  /** Carry on a paused or failed collection ("Continue", "Retry"). */
  | "resume"
  /** Carry on a failed digest-only run ("Retry"). */
  | "resumeDigest";

export interface Job {
  kind: JobKind;
  /** The period label (`2026-W46`) the run covers. */
  period: string;
}

/** What to run when the user presses Retry after `job` failed: carry on where it stopped. */
export function retryOf(job: Job): Job {
  switch (job.kind) {
    case "collect":
    case "resume":
      return { kind: "resume", period: job.period };
    case "regenerate":
    case "resumeDigest":
      return { kind: "resumeDigest", period: job.period };
  }
}

/** The step label before the first graph node reports. */
export const STARTING = "Starting";

/** Label of the graph node that just started. */
export function stepLabel(node: string): string {
  switch (node) {
    case "plan":
      return "Planning the sources";
    case "hackernews":
    case "yc":
    case "feeds":
    case "report_search":
      return "Reading the sources";
    case "store_items":
      return "Storing the items";
    case "wait_human":
      return "Checking for sources that need you";
    case "extract":
      return "Reading the items with your AI provider";
    case "count":
      return "Counting skills";
    case "synthesize":
      return "Writing the digest";
    case "enforce_citations":
      return "Checking the citations";
    case "save_report":
      return "Saving the digest";
    default:
      return "Working";
  }
}

/** Does a graph step belong to a trends run (`trends`, `trends_digest`)? */
export function isTrendsGraph(graph: string): boolean {
  return graph.startsWith("trends");
}
