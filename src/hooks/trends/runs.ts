// The run in flight, the step it is at and how the last one ended.
//
// A run takes minutes (collect, then the AI provider writes the digest), so it lives in a store of
// its own, not in the screen: leaving Trends and coming back still shows it running (and keeps a
// second one from starting), and its result is a toast wherever the user is by then.

import { create } from "zustand";

import { toCommandError } from "@/lib/problem";
import { useShell } from "@/stores/shell";
import { toast } from "@/stores/toast";
import type { GraphProgress } from "@/api/events";

import * as api from "@/api/trends";
import { resultBody } from "@/lib/trends/format";
import { isTrendsGraph, retryOf, stepLabel, STARTING, type Job } from "@/lib/trends/jobs";

export interface RunEnd {
  /** The report the run saved, if any. */
  reportId: number | null;
}

/** A run that saved a report: the Trends screen, when it shows the report this one replaces,
 *  switches to it (however the run was started: a button of the screen, or Retry in a toast). */
export interface Saved {
  period: string;
  reportId: number;
}

/** A run that did not finish is an error toast with Retry (carrying on where it stopped), raised
 *  here and nowhere else, so it shows once and on any screen. */
const FAILED = "The run did not finish";

interface RunsState {
  /** The job running now: the actions that would start another are disabled. */
  running: Job | undefined;
  /** The graph step it is at. */
  step: string | undefined;
  /** The report the last finished run saved. */
  saved: Saved | undefined;
  /** Run `job` unless one is running. Resolves when it ended (never rejects), `undefined` when it
   *  did not start. How it ended is said in a toast from here (a failure too, with Retry), so it
   *  shows on any screen. */
  start: (job: Job) => Promise<RunEnd | undefined>;
  /** A step of a graph run moved. */
  follow: (progress: GraphProgress) => void;
}

export const useRuns = create<RunsState>((set, get) => ({
  running: undefined,
  step: undefined,
  saved: undefined,

  start: async (job) => {
    if (get().running) {
      return undefined;
    }
    set({ running: job, step: STARTING });
    try {
      const run = await api.runJob(job);
      const saved = run.reportId == null ? undefined : { period: job.period, reportId: run.reportId };
      set({ running: undefined, step: undefined, saved });
      // A run that waits for the user or was cancelled is a note, not a success.
      const say = run.status === "done" ? toast.ok : toast.info;
      say(run.headline, resultBody(run.details));
      return { reportId: run.reportId };
    } catch (thrown) {
      const error = toCommandError(thrown);
      set({ running: undefined, step: undefined });
      toast.error(error, { title: FAILED, onRetry: () => void get().start(retryOf(job)) });
      // A missing AI provider: let the setup banner know.
      if (error.problem.kind === "noProvider") {
        useShell.getState().refresh();
      }
      return { reportId: null };
    }
  },

  follow: (progress) => {
    if (get().running && isTrendsGraph(progress.graph) && progress.kind === "nodeStarted" && progress.node) {
      set({ step: stepLabel(progress.node) });
    }
  },
}));
