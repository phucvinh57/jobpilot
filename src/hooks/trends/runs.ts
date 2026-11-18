// The run in flight, the step it is at and how the last one ended.
//
// A run takes minutes (collect, then the AI provider writes the digest), so it lives in a store of
// its own, not in the screen: leaving Trends and coming back still shows it running (and keeps a
// second one from starting), and its result is a toast wherever the user is by then.

import { create } from "zustand";

import { toCommandError, type CommandError } from "@/lib/problem";
import { useShell } from "@/stores/shell";
import { toast } from "@/stores/toast";
import type { GraphProgress } from "@/api/events";

import * as api from "@/api/trends";
import { resultBody } from "@/lib/trends/format";
import { isTrendsGraph, retryOf, stepLabel, STARTING, type Job } from "@/lib/trends/jobs";

/** A run that did not finish: the Trends screen shows it as a toast once (with Retry, which needs
 *  the screen) and then clears it (`clearFailure`). `retry` carries on where it stopped. */
export type Notice = { kind: "error"; error: CommandError; retry: Job };

export interface RunEnd {
  /** The report the run saved, if any. */
  reportId: number | null;
}

interface RunsState {
  /** The job running now: the actions that would start another are disabled. */
  running: Job | undefined;
  /** The graph step it is at. */
  step: string | undefined;
  notice: Notice | undefined;
  /** Run `job` unless one is running. Resolves when it ended (never rejects), `undefined` when it
   *  did not start. How it ended is said in a toast from here, so it shows on any screen. */
  start: (job: Job) => Promise<RunEnd | undefined>;
  /** A step of a graph run moved. */
  follow: (progress: GraphProgress) => void;
  /** Forget a failed run once it was reported. */
  clearFailure: () => void;
}

export const useRuns = create<RunsState>((set, get) => ({
  running: undefined,
  step: undefined,
  notice: undefined,

  start: async (job) => {
    if (get().running) {
      return undefined;
    }
    set({ running: job, step: STARTING, notice: undefined });
    try {
      const run = await api.runJob(job);
      set({ running: undefined, step: undefined });
      // A run that waits for the user or was cancelled is a note, not a success.
      const say = run.status === "done" ? toast.ok : toast.info;
      say(run.headline, resultBody(run.details));
      return { reportId: run.reportId };
    } catch (thrown) {
      const error = toCommandError(thrown);
      set({ running: undefined, step: undefined, notice: { kind: "error", error, retry: retryOf(job) } });
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

  clearFailure: () => {
    if (get().notice?.kind === "error") {
      set({ notice: undefined });
    }
  },
}));
