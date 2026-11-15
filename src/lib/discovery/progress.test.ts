import { describe, expect, it } from "vitest";

import type { GraphProgress } from "@/api/bridge";

import { movesTheQueue, parsePrepareThread, prepareLabel } from "./progress";

const step = (
  graph: string,
  kind: GraphProgress["kind"],
  node: string | null = null,
  text: string | null = null,
): GraphProgress => ({
  thread: "t",
  graph,
  kind,
  node,
  text,
});

describe("graph progress", () => {
  it("names the lead of a prepare-CV thread", () => {
    expect(parsePrepareThread("lead:42")).toBe(42);
    expect(parsePrepareThread("lead:")).toBeUndefined();
    expect(parsePrepareThread("lead:x")).toBeUndefined();
    expect(parsePrepareThread("discover:1:2")).toBeUndefined();
  });

  it("labels the steps of writing a CV", () => {
    expect(prepareLabel(step("prepare_cv", "nodeStarted", "promote"))).toBe("Creating the application");
    expect(prepareLabel(step("prepare_cv", "nodeStarted", "generate"))).toBe("Writing the CV");
    expect(prepareLabel(step("prepare_cv", "detail", "generate", "Tailoring the CV"))).toBe("Tailoring the CV");
    expect(prepareLabel(step("prepare_cv", "nodeFinished", "generate"))).toBeUndefined();
    expect(prepareLabel(step("prepare_cv", "nodeStarted", "other"))).toBeUndefined();
  });

  it("moves the queue when a source ends or a search starts or ends", () => {
    expect(movesTheQueue(step("source_run", "done"))).toBe(true);
    expect(movesTheQueue(step("source_run", "waiting"))).toBe(true);
    expect(movesTheQueue(step("source_run", "failed"))).toBe(true);
    expect(movesTheQueue(step("source_run", "detail", "fetch", "3 of 9"))).toBe(false);
    expect(movesTheQueue(step("discovery", "nodeStarted", "start"))).toBe(true);
    expect(movesTheQueue(step("discovery", "done"))).toBe(true);
    expect(movesTheQueue(step("prepare_cv", "done"))).toBe(false);
    expect(movesTheQueue(step("trends", "done"))).toBe(false);
  });
});
