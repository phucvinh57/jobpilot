import { describe, expect, it } from "vitest";

import type { GraphProgress } from "@/api/events";
import type { Activity } from "@/types/shell";
import { activityLabel, loaded, NO_ACTIVITY, onProgress, waitingCount, type ActivityState } from "./activity";

const CV: Activity = {
  id: "lead:4",
  title: "Preparing a CV",
  status: "Writing the CV",
  state: "running",
  screen: "jobs",
};
const SEARCH: Activity = {
  id: "discover:r1",
  title: "Job search",
  status: "Searching the sources",
  state: "running",
  screen: "jobs",
};
const PAUSED: Activity = {
  id: "discover:r1:7",
  title: "Job search: Acme",
  status: "Waiting for you: sign in",
  state: "waiting",
  screen: "jobs",
};

function step(kind: GraphProgress["kind"], thread: string, text: string | null = null): GraphProgress {
  return { thread, graph: "g", kind, node: kind.startsWith("node") || kind === "detail" ? "n" : null, text };
}

const state = (...items: Activity[]): ActivityState => loaded(NO_ACTIVITY, items);

describe("onProgress", () => {
  it("shows a progress text on that thread's job at once, without a read", () => {
    const { state: next, reread } = onProgress(state(CV, SEARCH), step("detail", "lead:4", "Tailoring the CV"));
    expect(reread).toBe(false);
    expect(next.items.map((i) => i.status)).toEqual(["Tailoring the CV", "Searching the sources"]);
  });

  it("ignores an empty progress text, and one for a waiting job", () => {
    const before = state(CV, PAUSED);
    expect(onProgress(before, step("detail", "lead:4", " ")).state).toBe(before);
    expect(onProgress(before, step("detail", "lead:4")).state).toBe(before);
    const next = onProgress(before, step("detail", "discover:r1:7", "Reading")).state;
    expect(next.items[1]).toEqual(PAUSED);
  });

  it("reads the list again when a step starts or a run pauses", () => {
    for (const kind of ["nodeStarted", "waiting"] as const) {
      const before = state(CV);
      const { state: next, reread } = onProgress(before, step(kind, "lead:9"));
      expect(reread).toBe(true);
      expect(next.items).toEqual([CV]);
    }
  });

  it("drops an ended job at once and reads again", () => {
    for (const kind of ["done", "failed", "cancelled"] as const) {
      const { state: next, reread } = onProgress(state(CV, SEARCH), step(kind, "lead:4"));
      expect(reread).toBe(true);
      expect(next.items).toEqual([SEARCH]);
    }
  });

  it("does nothing when a step only finished", () => {
    const before = state(CV);
    for (const kind of ["nodeFinished", "nodeFailed"] as const) {
      expect(onProgress(before, step(kind, "lead:4"))).toEqual({ state: before, reread: false });
    }
  });
});

describe("loaded", () => {
  it("keeps the progress text of a step that still runs", () => {
    const detailed = onProgress(state(CV, SEARCH), step("detail", "lead:4", "Tailoring the CV")).state;
    // Another thread moved: the list is read again while the CV is still being written.
    const next = loaded(detailed, [CV, SEARCH]);
    expect(next.items.map((i) => i.status)).toEqual(["Tailoring the CV", "Searching the sources"]);
  });

  it("forgets the progress text when that thread starts its next step, pauses or leaves", () => {
    const detailed = onProgress(state(CV), step("detail", "lead:4", "Tailoring the CV")).state;
    const started = onProgress(detailed, step("nodeStarted", "lead:4")).state;
    expect(loaded(started, [CV]).items).toEqual([CV]);

    const paused = { ...CV, state: "waiting" as const, status: "Waiting for you" };
    expect(loaded(detailed, [paused]).items).toEqual([paused]);

    const gone = loaded(detailed, [SEARCH]);
    expect(gone.details).toEqual({});
    expect(loaded(gone, [CV, SEARCH]).items).toEqual([CV, SEARCH]);
  });

  it("shows a text that arrived before the job was first listed", () => {
    const early = onProgress(NO_ACTIVITY, step("detail", "lead:4", "Tailoring the CV")).state;
    expect(early.items).toEqual([]);
    expect(loaded(early, [CV]).items[0]?.status).toBe("Tailoring the CV");
  });
});

describe("activityLabel", () => {
  it("counts the jobs and those waiting", () => {
    expect(activityLabel([CV])).toBe("1 background job");
    expect(activityLabel([CV, SEARCH])).toBe("2 background jobs");
    expect(activityLabel([CV, SEARCH, PAUSED])).toBe("3 background jobs, 1 waiting for you");
    expect(waitingCount([CV, PAUSED])).toBe(1);
  });
});
