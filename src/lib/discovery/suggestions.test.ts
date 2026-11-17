import { describe, expect, it } from "vitest";

import type { GraphProgress } from "@/api/events";

import { addedText, fitText, suggestionLine, suggestionName, suggestNote, SUGGEST_GRAPH } from "./suggestions";

const SUGGESTION = {
  id: 1,
  kind: "greenhouse",
  label: "Greenhouse",
  name: "Demo Robotics",
  host: "boards.greenhouse.io",
  jobs: 14,
  matching: 3,
};

function step(extra: Partial<GraphProgress>): GraphProgress {
  return { thread: "source_suggest", graph: SUGGEST_GRAPH, kind: "detail", node: "search", text: null, ...extra };
}

describe("suggested sources", () => {
  it("says how many jobs fit the roles, and only the count when that says enough", () => {
    expect(fitText({ jobs: 14, matching: 3 })).toBe("3 of 14 jobs fit your roles");
    expect(fitText({ jobs: 4, matching: 4 }), "all fit").toBe("4 jobs");
    expect(fitText({ jobs: 6, matching: 0 }), "no roles, or none fits").toBe("6 jobs");
    expect(fitText({ jobs: 1, matching: 1 })).toBe("1 job");
    expect(fitText({ jobs: 2, matching: 1 })).toBe("1 of 2 jobs fit your roles");
  });

  it("names a row by its name, else its host, else its kind", () => {
    expect(suggestionName(SUGGESTION)).toBe("Demo Robotics");
    expect(suggestionName({ ...SUGGESTION, name: "  " })).toBe("boards.greenhouse.io");
    expect(suggestionName({ ...SUGGESTION, name: "", host: "" })).toBe("Greenhouse");
  });

  it("puts the kind, the host and the fit on one line", () => {
    expect(suggestionLine(SUGGESTION)).toBe("Greenhouse · boards.greenhouse.io · 3 of 14 jobs fit your roles");
    expect(suggestionLine({ ...SUGGESTION, host: "", matching: 14 })).toBe("Greenhouse · 14 jobs");
  });

  it("counts the sources added", () => {
    expect(addedText(1)).toBe("1 source added");
    expect(addedText(3)).toBe("3 sources added");
  });

  it("reads the progress line of the search only", () => {
    expect(suggestNote(step({ text: "Searching 3 of 8" }))).toBe("Searching 3 of 8");
    expect(suggestNote(step({ text: "Checking 5 sources", node: "verify" }))).toBe("Checking 5 sources");
    expect(suggestNote(step({ graph: "discovery", text: "Searching 3 of 8" })), "another graph").toBeUndefined();
    expect(suggestNote(step({ kind: "nodeStarted", text: null }))).toBeUndefined();
    expect(suggestNote(step({ text: " " }))).toBeUndefined();
  });
});
