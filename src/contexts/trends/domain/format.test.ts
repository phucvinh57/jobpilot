import { describe, expect, it } from "vitest";

import {
  addFailure,
  chartHint,
  collectCaption,
  digestHint,
  pickScope,
  readFailure,
  reportRowName,
  skillRowName,
  termsLine,
} from "./format";
import type { ChartScope } from "./types";

function scope(key: "local" | "global"): ChartScope {
  return {
    key,
    label: key === "local" ? "Local" : "Global",
    covers: key === "local" ? "Singapore or remote" : "every posting",
    documents: key === "local" ? 120 : 900,
    months: [],
    skills: [],
    maxPercent: 1,
    note: null,
  };
}

describe("sentences", () => {
  it("says when a digest was made and how much it read", () => {
    expect(digestHint("2026-11-14 09:12", 6)).toBe("2026-11-14 09:12 · 6 item(s) read");
    expect(digestHint("2026-11-14 09:12", null)).toBe("2026-11-14 09:12");
  });

  it("explains the chart", () => {
    // A report saved before the app knew the user's place: postings and leads together.
    const old = chartHint(
      { fromMonth: "2026-06", toMonth: "2026-11", marks: false },
      { ...scope("global"), documents: 398 },
    );
    expect(old).toContain("2026-06 to 2026-11 (398 read)");
    expect(old).toContain("Counted by the app, not by the AI");

    const local = chartHint({ fromMonth: "2026-06", toMonth: "2026-11", marks: true }, scope("local"));
    expect(local).toMatch(/^Local market: Singapore or remote, 120 hiring postings, 2026-06 to 2026-11\./);
    expect(local).toContain("% of postings naming the skill, counted by the app, not by the AI");
  });

  it("shows the scope picked, else the gap's, else the first", () => {
    const chart = { scopes: [scope("local"), scope("global")], scope: "local" as const };
    expect(pickScope(chart, undefined)?.key).toBe("local");
    expect(pickScope(chart, "global")?.key).toBe("global");
    expect(pickScope({ scopes: [scope("global")], scope: "local" }, "local")?.key).toBe("global");
    expect(pickScope({ scopes: [], scope: "global" }, undefined)).toBeUndefined();
  });

  it("names a skill row with its share, direction and marks", () => {
    const base = {
      name: "Rust",
      share: "9.4%",
      description: "56 in total",
      direction: null,
      inProfile: false,
      wished: false,
    };
    expect(skillRowName(base)).toBe("Rust: 9.4% · 56 in total");
    expect(skillRowName({ ...base, direction: { label: "rising" }, inProfile: true, wished: true })).toBe(
      "Rust: 9.4% · rising · in your profile · in your wishes · 56 in total",
    );
  });

  it("lists the terms the skill list does not know", () => {
    expect(
      termsLine([
        { term: "Temporal", items: 5 },
        { term: "dbt", items: 1 },
      ]),
    ).toBe("Temporal (5 items), dbt (1 item)");
    expect(termsLine([])).toBe("");
  });

  it("names the period a run covers", () => {
    expect(collectCaption("2026-W46")).toMatch(/^2026-W46: collects from your sources/);
  });

  it("names a row of the history and the failures", () => {
    expect(reportRowName("Week 46", "2026-11-14 09:12")).toBe("Week 46, 2026-11-14 09:12");
    expect(readFailure("disk full")).toBe("Could not read the saved data: disk full");
    expect(addFailure("Rust", "locked")).toBe('Could not add "Rust" to your preferences: locked');
  });
});
