import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import * as events from "@/api/events";
import type { GraphProgress } from "@/api/events";
import * as shell from "@/api/shell";
import { NO_ACTIVITY } from "@/lib/activity";
import type { Activity } from "@/types/shell";
import { REREAD_MS, startActivity, useActivity } from "./activity";

vi.mock("@/api/shell");
vi.mock("@/api/events");

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

let emit: (p: GraphProgress) => void = () => {};
const stopped = vi.fn();

function step(kind: GraphProgress["kind"], thread: string, text: string | null = null): GraphProgress {
  return { thread, graph: "g", kind, node: null, text };
}

/** Let the pending read settle. */
const settle = () => vi.advanceTimersByTimeAsync(0);
const statuses = () => useActivity.getState().items.map((i) => `${i.title}: ${i.status}`);

beforeEach(() => {
  vi.useFakeTimers();
  vi.resetAllMocks();
  useActivity.setState(NO_ACTIVITY);
  vi.mocked(events.onGraphProgress).mockImplementation((handler) => {
    emit = handler;
    return stopped;
  });
  vi.spyOn(console, "warn").mockImplementation(() => {});
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("startActivity", () => {
  it("reads the jobs once at the start", async () => {
    vi.mocked(shell.activity).mockResolvedValue([CV]);
    const stop = startActivity();
    await settle();
    expect(shell.activity).toHaveBeenCalledTimes(1);
    expect(statuses()).toEqual(["Preparing a CV: Writing the CV"]);
    stop();
    expect(stopped).toHaveBeenCalledTimes(1);
  });

  it("reads again once after a burst of steps, and shows a progress text without reading", async () => {
    vi.mocked(shell.activity).mockResolvedValueOnce([CV]).mockResolvedValue([CV, SEARCH]);
    const stop = startActivity();
    await settle();

    emit(step("detail", "lead:4", "Tailoring the CV"));
    expect(statuses()).toEqual(["Preparing a CV: Tailoring the CV"]);
    await vi.advanceTimersByTimeAsync(REREAD_MS * 2);
    expect(shell.activity).toHaveBeenCalledTimes(1);

    emit(step("nodeStarted", "discover:r1"));
    await vi.advanceTimersByTimeAsync(REREAD_MS / 2);
    emit(step("nodeStarted", "discover:r1"));
    await vi.advanceTimersByTimeAsync(REREAD_MS / 2 - 1);
    expect(shell.activity).toHaveBeenCalledTimes(1);
    // The read is due one interval after the first step of the burst, whatever came after it.
    await vi.advanceTimersByTimeAsync(1);
    expect(shell.activity).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(REREAD_MS * 2);
    expect(shell.activity).toHaveBeenCalledTimes(2);
    // The CV is still at the step that reported: its text stays.
    expect(statuses()).toEqual(["Preparing a CV: Tailoring the CV", "Job search: Searching the sources"]);
    stop();
  });

  it("keeps reading under a steady stream of steps: once per interval, and once after the last", async () => {
    vi.mocked(shell.activity).mockResolvedValue([CV]);
    const stop = startActivity();
    await settle();
    expect(shell.activity).toHaveBeenCalledTimes(1);

    // A step every fifth of the interval, for four intervals: a read at the end of each.
    const gap = REREAD_MS / 5;
    for (let n = 0; n < 20; n++) {
      emit(step("nodeStarted", "lead:4"));
      await vi.advanceTimersByTimeAsync(gap);
    }
    expect(shell.activity).toHaveBeenCalledTimes(5);

    // The last step of a stream is read too.
    emit(step("nodeStarted", "lead:4"));
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(shell.activity).toHaveBeenCalledTimes(6);
    await vi.advanceTimersByTimeAsync(REREAD_MS * 2);
    expect(shell.activity).toHaveBeenCalledTimes(6);
    stop();
  });

  it("keeps the list when a read fails, except a job that ended", async () => {
    vi.mocked(shell.activity).mockResolvedValueOnce([CV, SEARCH]).mockRejectedValue(new Error("no shell"));
    const stop = startActivity();
    await settle();

    emit(step("nodeStarted", "lead:4"));
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(shell.activity).toHaveBeenCalledTimes(2);
    expect(useActivity.getState().items).toEqual([CV, SEARCH]);

    emit(step("done", "lead:4"));
    expect(useActivity.getState().items).toEqual([SEARCH]);
    await vi.advanceTimersByTimeAsync(REREAD_MS);
    expect(useActivity.getState().items).toEqual([SEARCH]);
    stop();
  });

  it("stops reading after the teardown", async () => {
    vi.mocked(shell.activity).mockResolvedValue([CV]);
    const stop = startActivity();
    emit(step("nodeStarted", "lead:4"));
    stop();
    await vi.advanceTimersByTimeAsync(REREAD_MS * 2);
    expect(shell.activity).toHaveBeenCalledTimes(1);
    expect(useActivity.getState().items).toEqual([]);
  });
});
