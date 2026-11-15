import { describe, expect, it } from "vitest";

import { dismissalLine, jobDismissedDetail, jobRestoredDetail } from "./dismiss";

describe("dismissing a job in CVs", () => {
  it("says where the job went and what else went with it", () => {
    expect(jobDismissedDetail({ also: 0, kept: 0 })).toBe("Find it under Leads > Dismissed.");
    expect(jobDismissedDetail({ also: 2, kept: 0 })).toBe(
      "2 leads from the same company went too. Find it under Leads > Dismissed.",
    );
    expect(jobDismissedDetail({ also: 1, kept: 1 })).toContain(
      "1 lead from that company was kept: the CV is being prepared.",
    );
  });

  it("says when a restored job's company is still excluded", () => {
    expect(jobRestoredDetail({ hidden: null, excluded: false })).toBeUndefined();
    expect(jobRestoredDetail({ hidden: null, excluded: true })).toContain("still excluded");
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
