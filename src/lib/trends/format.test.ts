import { describe, expect, it } from "vitest";

import {
  addFailure,
  chartHint,
  collectCaption,
  digestHint,
  gapSkillName,
  levelLine,
  pickScope,
  noReportsCaption,
  resultBody,
  reportFileName,
  reportName,
  skillRowName,
  termsLine,
} from "./format";
import type { ChartScope } from "@/types/trends";

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
      now: null,
      range: null,
      description: "56 in total",
      direction: null,
      inProfile: false,
      wished: false,
    };
    // A report saved before the tests has no level: the name is what it always was.
    expect(skillRowName(base)).toBe("Rust: 9.4% · 56 in total");
    expect(skillRowName({ ...base, direction: { label: "rising" }, inProfile: true, wished: true })).toBe(
      "Rust: 9.4% · rising · in your profile · in your wishes · 56 in total",
    );
  });

  it("names a skill row with the level of the period and its range when the report has them", () => {
    const row = {
      name: "Rust",
      share: "9%",
      now: "9.4%",
      range: "7.1% to 12.3%",
      description: "56 in total",
      direction: { label: "unclear" },
      inProfile: true,
      wished: false,
    };
    expect(skillRowName(row)).toBe(
      "Rust: 9% · this period 9.4% (7.1% to 12.3%) · unclear · in your profile · 56 in total",
    );
    expect(levelLine(row)).toBe("this period 9.4% (7.1% to 12.3%)");
    expect(levelLine({ now: null, range: null })).toBeUndefined();
    expect(levelLine({ now: "9.4%", range: null })).toBeUndefined();
  });

  it("names a skill of the gap with its range when the report has one", () => {
    const rising = { label: "rising" };
    expect(gapSkillName({ skill: "Rust", share: "9.4%", now: null, range: null, direction: rising })).toBe(
      "Rust: 9.4% of postings, rising",
    );
    expect(
      gapSkillName({ skill: "Rust", share: "9.4%", now: "13.0%", range: "9.0% to 22.0%", direction: rising }),
    ).toBe("Rust: 9.4% of postings, this period 13.0% (9.0% to 22.0%), rising");
  });

  it("lists the terms the skill list does not know, by the kind of item they were found in", () => {
    expect(
      termsLine([
        { term: "Temporal", items: 11, kind: "hiring postings", rising: true },
        { term: "dbt", items: 1, kind: "hiring postings", rising: false },
        { term: "vector databases", items: 14, kind: "Hacker News stories", rising: false },
        { term: "agent harness", items: 6, kind: "feeds", rising: true },
        { term: "evals", items: 3, kind: "hiring postings", rising: false },
      ]),
    ).toBe(
      "hiring postings: Temporal (11 items, rising), dbt (1 item), evals (3 items); " +
        "Hacker News stories: vector databases (14 items); feeds: agent harness (6 items, rising)",
    );
    expect(termsLine([])).toBe("");
  });

  it("lists the terms of a report saved before the kinds were told apart as it always did", () => {
    expect(
      termsLine([
        { term: "Temporal", items: 5, kind: "", rising: false },
        { term: "dbt", items: 1, kind: "", rising: false },
      ]),
    ).toBe("Temporal (5 items), dbt (1 item)");
  });

  it("names the period a run covers", () => {
    expect(collectCaption("2026-W46")).toMatch(/^2026-W46: collects from your sources/);
  });

  it("names a saved report by its period", () => {
    expect(reportName({ periodTitle: "November 2026", period: "2026-M11" })).toBe("November 2026 (2026-M11)");
    expect(reportName({ periodTitle: "2026", period: "2026" })).toBe("2026");
    expect(noReportsCaption("monthly")).toBe("No monthly reports yet. Press New research and choose Monthly.");
  });

  it("offers a report's PDF under a safe file name", () => {
    expect(reportFileName("2026-M11")).toBe("jobpilot-market-insight-2026-M11.pdf");
    expect(reportFileName("2026-W46")).toBe("jobpilot-market-insight-2026-W46.pdf");
    expect(reportFileName("2026")).toBe("jobpilot-market-insight-2026.pdf");
    expect(reportFileName("../a b/c")).toBe("jobpilot-market-insight-a-b-c.pdf");
    expect(reportFileName("")).toBe("jobpilot-market-insight-report.pdf");
  });

  it("names the failures", () => {
    expect(addFailure("Rust", "locked")).toBe('Could not add "Rust" to your preferences: locked');
  });

  it("puts the lines of a run's result on one line, and nothing when there are none", () => {
    expect(resultBody(["Hacker News: 4 new item(s)", "YC companies: timed out"])).toBe(
      "Hacker News: 4 new item(s) · YC companies: timed out",
    );
    expect(resultBody([])).toBeUndefined();
  });
});
