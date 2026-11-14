import { describe, expect, it } from "vitest";

import { DEFAULT_TAB, primaryClick, TABS, type Tab } from "./tabs";

describe("the tabs", () => {
  it("are JD, CV, Apply and Timeline, opening on the CV", () => {
    expect(TABS.map((t) => t.label)).toEqual(["JD", "CV", "Apply", "Timeline"]);
    expect(DEFAULT_TAB).toBe("cv");
  });
});

describe("the primary click", () => {
  const all: Tab[] = ["jd", "cv", "apply", "timeline"];

  it("generates and approves where the user is", () => {
    for (const tab of all) {
      expect(primaryClick("generate", tab)).toEqual({ tab, run: true });
      expect(primaryClick("approveCv", tab)).toEqual({ tab, run: true });
    }
  });

  it("opens the Apply tab and starts for Review & Submit, from any tab", () => {
    for (const tab of all) {
      expect(primaryClick("reviewAndSubmit", tab)).toEqual({ tab: "apply", run: true });
    }
  });

  it("only brings the review into view for Continue and Submit from another tab", () => {
    for (const action of ["continue", "submit"] as const) {
      expect(primaryClick(action, "cv")).toEqual({ tab: "apply", run: false });
      expect(primaryClick(action, "timeline")).toEqual({ tab: "apply", run: false });
      expect(primaryClick(action, "apply")).toEqual({ tab: "apply", run: true });
    }
  });
});
