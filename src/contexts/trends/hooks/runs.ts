// The run in flight, the step it is at and the line the screen shows under the controls.
//
// A run takes minutes (collect, then the AI provider writes the digest), so it lives in a store of
// its own, not in the screen: leaving Trends and coming back still shows it running (and keeps a
// second one from starting), and its result is there when the user returns.

import { create } from "zustand";

import { toCommandError, useShell, type CommandError, type GraphProgress } from "@/shared";

import * as api from "../api";
import { isTrendsGraph, retryOf, stepLabel, STARTING, type Job } from "../domain/jobs";

/** The line under the controls. */
export type Notice =
  | { kind: "info"; text: string; details: string[] }
  /** A run that did not finish; `retry` carries on where it stopped. */
  | { kind: "error"; error: CommandError; retry: Job };

export interface RunEnd {
  /** The report the run saved, if any. */
  reportId: number | null;
}

interface RunsState {
  /** The job running now: the primary button and the actions that would start another are disabled. */
  running: Job | undefined;
  /** The graph step it is at. */
  step: string | undefined;
  notice: Notice | undefined;
  /** Run `job` unless one is running. Resolves when it ended (never rejects), `undefined` when it did not start. */
  start: (job: Job) => Promise<RunEnd | undefined>;
  /** A step of a graph run moved. */
  follow: (progress: GraphProgress) => void;
  say: (text: string) => void;
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
      set({ running: undefined, step: undefined, notice: { kind: "info", text: run.headline, details: run.details } });
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

  say: (text) => set({ notice: { kind: "info", text, details: [] } }),
}));
