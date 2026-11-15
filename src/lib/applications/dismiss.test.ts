import { describe, expect, it } from "vitest";

import { dismissalLine, jobDismissedNote } from "./dismiss";

describe("dismissing a job in CVs", () => {
  it("says where the job went and what else went with it", () => {
    expect(jobDismissedNote({ also: 0, kept: 0, notExcluded: null })).toBe(
      "Job dismissed. Find it under Leads > Dismissed.",
    );
    expect(jobDismissedNote({ also: 2, kept: 0, notExcluded: null })).toBe(
      "Job dismissed, and 2 leads from the same company. Find them under Leads > Dismissed.",
    );
    expect(jobDismissedNote({ also: 1, kept: 1, notExcluded: null })).toContain(
      "1 lead from that company was kept: the CV is being prepared.",
    );
    expect(jobDismissedNote({ also: 0, kept: 0, notExcluded: "disk full" })).toBe(
      "Job dismissed, but the company could not be excluded: disk full",
    );
  });

  it("says when and why a job was dismissed", () => {
    const now = new Date("2026-11-14T11:30:00+00:00");
    expect(dismissalLine({ reason: "too far away", at: "2026-11-14T09:30:00+00:00" }, now)).toBe(
      "Dismissed 2 h ago: too far away",
    );
    expect(dismissalLine({ reason: null, at: null }, now)).toBe("Dismissed, no reason given.");
    expect(dismissalLine({ reason: null, at: "garbage" }, now)).toBe("Dismissed, no reason given.");
  });
});
