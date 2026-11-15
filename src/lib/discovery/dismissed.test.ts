import { describe, expect, it } from "vitest";

import { filterDismissed, NO_FILTER } from "./queue";
import { dismissedWhen, isExclusion, restoredNote } from "./text";
import type { DismissedJob } from "@/types/discovery";

function job(id: number, extra: Partial<DismissedJob> = {}): DismissedJob {
  return {
    kind: "lead",
    id,
    company: "Acme",
    role: `Role ${id}`,
    url: `https://a.test/${id}`,
    reason: null,
    dismissedAt: "2026-11-14T09:00:00+00:00",
    ...extra,
  };
}

describe("the dismissed filter", () => {
  it("shows the open leads by default, with no search", () => {
    expect(NO_FILTER.show).toBe("open");
    expect(NO_FILTER.search).toBe("");
  });

  it("searches company and role, ignoring case and keeping the order", () => {
    const items = [
      job(1, { company: "Globex", role: "SRE" }),
      job(2, { company: "Initech", role: "Backend Engineer", kind: "application" }),
      job(3, { company: "Acme", role: "Site reliability" }),
    ];
    const ids = (search: string) => filterDismissed(items, search).map((j) => j.id);
    expect(ids("")).toEqual([1, 2, 3]);
    expect(ids("   ")).toEqual([1, 2, 3]);
    expect(ids("sre")).toEqual([1]);
    expect(ids("INITECH")).toEqual([2]);
    expect(ids(" reliab ")).toEqual([3]);
    expect(ids("engineer")).toEqual([2]);
    expect(ids("nobody")).toEqual([]);
    // A copy: the store's list is never handed out to be changed.
    expect(filterDismissed(items, "")).not.toBe(items);
  });
});

describe("the dismissed list's words", () => {
  it("says where a restored job went", () => {
    expect(restoredNote(job(1))).toBe("Acme — Role 1 is back in Leads.");
    expect(restoredNote(job(2, { kind: "application", reason: "too far" }))).toBe("Acme — Role 2 is back in CVs.");
  });

  it("says that an excluded company stays excluded", () => {
    expect(isExclusion("excluded company: Acme")).toBe(true);
    expect(isExclusion("  Excluded company: Acme")).toBe(true);
    expect(isExclusion("too junior")).toBe(false);
    expect(isExclusion(null)).toBe(false);
    expect(restoredNote(job(1, { reason: "excluded company: Acme" }))).toContain("still excluded in your preferences");
  });

  it("says that a restored lead the preferences filter stays hidden", () => {
    expect(restoredNote(job(1, { reason: "excluded company: Acme" }), { hidden: "excluded company: Acme" })).toBe(
      "Acme — Role 1 is restored, but your preferences hide it (excluded company: Acme). It shows in Leads once they let it pass.",
    );
  });

  it("says when a job was dismissed", () => {
    const now = new Date("2026-11-14T09:05:00+00:00");
    expect(dismissedWhen("2026-11-14T09:00:00+00:00", now)).toBe("5 min ago");
    expect(dismissedWhen("not a date", now)).toBe("earlier");
  });
});
