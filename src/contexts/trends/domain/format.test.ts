import { describe, expect, it } from "vitest";

import { addFailure, chartHint, collectCaption, digestHint, readFailure, reportRowName } from "./format";

describe("sentences", () => {
  it("says when a digest was made and how much it read", () => {
    expect(digestHint("2026-11-14 09:12", 6)).toBe("2026-11-14 09:12 · 6 item(s) read");
    expect(digestHint("2026-11-14 09:12", null)).toBe("2026-11-14 09:12");
  });

  it("explains the chart", () => {
    const hint = chartHint({ fromMonth: "2026-06", toMonth: "2026-11", documents: 398 });
    expect(hint).toContain("2026-06 to 2026-11 (398 read)");
    expect(hint).toContain("Counted by the app, not by the AI");
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
