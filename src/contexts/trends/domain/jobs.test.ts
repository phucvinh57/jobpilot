import { describe, expect, it } from "vitest";

import { isTrendsGraph, retryOf, stepLabel, type Job } from "./jobs";

const PERIOD = "2026-W46";

describe("retry", () => {
  it("carries on where the job stopped", () => {
    expect(retryOf({ kind: "collect", period: PERIOD })).toEqual({ kind: "resume", period: PERIOD });
    expect(retryOf({ kind: "resume", period: PERIOD })).toEqual({ kind: "resume", period: PERIOD });
    expect(retryOf({ kind: "regenerate", period: PERIOD })).toEqual({ kind: "resumeDigest", period: PERIOD });
    expect(retryOf({ kind: "resumeDigest", period: PERIOD })).toEqual({ kind: "resumeDigest", period: PERIOD });
  });

  it("keeps the period of the job that failed", () => {
    const job: Job = { kind: "collect", period: "2026-Q4" };
    expect(retryOf(job).period).toBe("2026-Q4");
  });
});

describe("step labels", () => {
  it("names every graph node of a trends run", () => {
    for (const node of [
      "plan",
      "hackernews",
      "yc",
      "feeds",
      "report_search",
      "store_items",
      "wait_human",
      "extract",
      "count",
      "synthesize",
      "enforce_citations",
      "save_report",
    ]) {
      expect(stepLabel(node), node).not.toBe("Working");
    }
    expect(stepLabel("something else")).toBe("Working");
  });

  it("follows the trends graphs only", () => {
    expect(isTrendsGraph("trends")).toBe(true);
    expect(isTrendsGraph("trends_digest")).toBe(true);
    expect(isTrendsGraph("discovery")).toBe(false);
    expect(isTrendsGraph("prefs_chat")).toBe(false);
  });
});
