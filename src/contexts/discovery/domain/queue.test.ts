import { describe, expect, it } from "vitest";

import {
  actionLabel,
  dropUnreported,
  failureRows,
  filterItems,
  jobLabel,
  MIN_SCORES,
  needsYouCount,
  neighbourOf,
  NO_FILTER,
  openLeadId,
  prepareManyLabel,
  primaryPlace,
  queuePrimaryFor,
  STOPPED,
  targetsOf,
  type PlaceFacts,
  type QueueFacts,
  type SessionFailure,
} from "./queue";
import type { FailedPrepare, Job, Lead, Queue } from "./types";

const OTHER = { kind: "other", challenge: null, advice: null, settingsSection: null, retryable: false } as const;

function lead(id: number, extra: Partial<Lead> = {}): Lead {
  return {
    id,
    sourceId: null,
    url: `https://a.test/${id}`,
    company: "Acme",
    role: "Backend Engineer",
    location: "",
    score: null,
    foundAt: "2026-11-14T09:00:00+00:00",
    reasons: [],
    criteria: null,
    insight: null,
    ...extra,
  };
}

function facts(change: Partial<QueueFacts> = {}): QueueFacts {
  return { total: 5, shown: 5, targets: 1, idleTargets: 1, searching: false, sources: 2, ...change };
}

describe("the one primary button", () => {
  it("is Prepare CV while there are leads", () => {
    const p = queuePrimaryFor(facts());
    expect([p.action, p.enabled, p.loading, p.hint]).toEqual(["prepareCv", true, false, undefined]);
    expect(actionLabel(p.action)).toBe("Prepare CV");

    const many = queuePrimaryFor(facts({ targets: 3, idleTargets: 3 }));
    expect(many.enabled && many.hint?.includes("3 leads")).toBe(true);
    // Some targets are already queued: the button still works for the rest.
    expect(queuePrimaryFor(facts({ targets: 3, idleTargets: 1 })).enabled).toBe(true);
    // A search running elsewhere never blocks it.
    expect(queuePrimaryFor(facts({ searching: true })).enabled).toBe(true);
  });

  it("waits for a selection and is disabled while the work runs", () => {
    const none = queuePrimaryFor(facts({ targets: 0, idleTargets: 0 }));
    expect([none.action, none.enabled]).toEqual(["prepareCv", false]);
    expect(none.hint).toContain("Select a lead");

    const hidden = queuePrimaryFor(facts({ shown: 0, targets: 0, idleTargets: 0 }));
    expect(!hidden.enabled && hidden.hint?.includes("filters")).toBe(true);

    const running = queuePrimaryFor(facts({ idleTargets: 0 }));
    expect([running.action, running.enabled, running.loading]).toEqual(["prepareCv", false, true]);
  });

  it("is Search now when the list is empty", () => {
    const empty: Partial<QueueFacts> = { total: 0, shown: 0, targets: 0, idleTargets: 0 };
    const p = queuePrimaryFor(facts(empty));
    expect([p.action, p.enabled, p.loading]).toEqual(["searchNow", true, false]);
    expect(actionLabel(p.action)).toBe("Search now");

    const searching = queuePrimaryFor(facts({ ...empty, searching: true }));
    expect([searching.action, searching.enabled, searching.loading]).toEqual(["searchNow", false, true]);

    const noSources = queuePrimaryFor(facts({ ...empty, sources: 0 }));
    expect([noSources.action, noSources.enabled]).toEqual(["searchNow", false]);
    expect(noSources.hint).toContain("Settings");
  });
});

describe("where the one primary button sits", () => {
  const place = (f: Partial<PlaceFacts>) => primaryPlace({ drawerOpen: false, checked: 0, empty: false, ...f });

  it("is the lead drawer while it is open, whatever is ticked", () => {
    expect(place({ drawerOpen: true })).toBe("drawer");
    expect(place({ drawerOpen: true, checked: 2 })).toBe("drawer");
  });

  it("is the bulk bar over ticked leads while the drawer is closed", () => {
    expect(place({ checked: 1 })).toBe("bulk");
    expect(place({ checked: 3, empty: true })).toBe("bulk");
  });

  it("is Search now on an empty list, and nowhere on a list with nothing open or ticked", () => {
    expect(place({ empty: true })).toBe("search");
    expect(place({})).toBe("none");
  });

  it("is nowhere here while another drawer owns it", () => {
    expect(place({ otherDrawerOpen: true, empty: true })).toBe("none");
    expect(place({ otherDrawerOpen: true, checked: 2 })).toBe("none");
  });

  it("names the bulk action by its count", () => {
    expect(prepareManyLabel(1)).toBe("Prepare 1 CV");
    expect(prepareManyLabel(3)).toBe("Prepare 3 CVs");
  });
});

describe("jobs that never reported", () => {
  it("are cleared and can be retried", () => {
    const jobs: Record<number, Job> = {
      2: { state: "queued" },
      3: { state: "queued" },
      9: { state: "running", label: "other batch" },
    };
    const next = dropUnreported(jobs, {}, [2, 3], OTHER);
    expect(Object.keys(next.jobs), "only this batch's jobs are cleared").toEqual(["9"]);
    expect(Object.keys(next.failed).sort()).toEqual(["2", "3"]);
    expect(next.failed[2]?.message).toContain("Try again");
    expect(next.failed[2]?.message).toBe(STOPPED);
    // Nothing to clear twice.
    const twice = dropUnreported(next.jobs, next.failed, [2], OTHER);
    expect(Object.keys(twice.failed)).toHaveLength(2);
  });
});

describe("the filters", () => {
  const items = [
    lead(1, { sourceId: 1, location: "Singapore", score: 90 }),
    lead(2, { sourceId: 1, location: "Remote - EU", score: 55 }),
    lead(3, { sourceId: 2, location: "ho chi minh city, Vietnam", score: 70 }),
    lead(4, { sourceId: 2, location: "Remote", score: null }),
    lead(5, { sourceId: null, location: "", score: 95 }),
  ];
  const shown = (filter: Partial<typeof NO_FILTER>) => filterItems(items, { ...NO_FILTER, ...filter }).map((l) => l.id);

  it("narrow by source, minimum score and location", () => {
    expect(shown({}), "no filter keeps everything, in order").toEqual([1, 2, 3, 4, 5]);
    expect(shown({ sourceId: 1 })).toEqual([1, 2]);
    expect(shown({ sourceId: 2 })).toEqual([3, 4]);
    expect(shown({ minScore: 70 }), "70 counts; unscored never passes").toEqual([1, 3, 5]);
    expect(shown({ location: " REMOTE " }), "ignoring case and spaces").toEqual([2, 4]);
    expect(shown({ location: "vietnam" })).toEqual([3]);
    expect(shown({ sourceId: 1, minScore: 60, location: "sing" })).toEqual([1]);
    expect(shown({ sourceId: 999 })).toEqual([]);
    expect(MIN_SCORES).toEqual([50, 70, 85]);
  });
});

describe("which lead is open", () => {
  const shown = [lead(10), lead(11), lead(12)];

  it('is the one asked for when it is listed, the best for "best", else none', () => {
    expect(openLeadId(shown, 11)).toBe(11);
    expect(openLeadId(shown, 99), "not listed: the drawer stays closed").toBeUndefined();
    expect(openLeadId(shown, "best"), "Next job opens the best one").toBe(10);
    expect(openLeadId(shown, undefined), "the drawer is closed: no fallback").toBeUndefined();
    expect(openLeadId([], 11)).toBeUndefined();
    expect(openLeadId([], "best")).toBeUndefined();
  });

  it("moves to the neighbour once a lead leaves: the one after it, else the one before", () => {
    expect(neighbourOf(shown, 10)).toBe(11);
    expect(neighbourOf(shown, 11)).toBe(12);
    expect(neighbourOf(shown, 12)).toBe(11);
    expect(neighbourOf([lead(1)], 1)).toBeUndefined();
    expect(neighbourOf(shown, 99)).toBeUndefined();
  });

  it("Prepare CV acts on the ticked leads in queue order, else the open one", () => {
    expect(targetsOf(shown, [12, 10], 11)).toEqual([10, 12]);
    expect(targetsOf(shown, [], 11)).toEqual([11]);
    expect(targetsOf(shown, [], undefined)).toEqual([]);
    expect(targetsOf(shown, [], 99), "a lead that is not listed is not a target").toEqual([]);
    expect(targetsOf(shown, [99], 11), "a ticked lead that left the list is ignored").toEqual([]);
  });
});

describe("what needs the user", () => {
  const stored = (leadId: number, extra: Partial<FailedPrepare> = {}): FailedPrepare => ({
    leadId,
    company: "Acme",
    role: "Engineer",
    message: "boom",
    problem: OTHER,
    applicationId: 7,
    ...extra,
  });

  it("lists the failures the core remembers, then those of this session, by lead", () => {
    const session: Record<number, SessionFailure> = {
      3: { message: "no AI provider", problem: OTHER },
      1: { message: "dup", problem: OTHER },
    };
    const rows = failureRows([stored(1), stored(5)], [lead(3, { company: "Beta", role: "SRE" })], {}, session);
    expect(rows.map((r) => r.leadId)).toEqual([1, 3, 5]);
    expect(rows[0]?.message, "the stored failure wins over a session one of the same lead").toBe("boom");
    expect(rows[0]?.title).toBe("Acme — Engineer");
    expect(rows[1]?.title).toBe("Beta — SRE");
    expect(rows[1]?.applicationId).toBeNull();
    expect(
      failureRows([], [], {}, { 9: { message: "x", problem: OTHER } })[0]?.title,
      "a lead that is not listed",
    ).toBe("Lead 9");
  });

  it("hides the failure of a lead that is being prepared again", () => {
    const rows = failureRows([stored(1), stored(2)], [], { 1: { state: "queued" } }, {});
    expect(rows.map((r) => r.leadId)).toEqual([2]);
  });

  it("counts sources waiting, failed CVs and CVs to read", () => {
    const queue = {
      items: [],
      failed: [],
      sources: [],
      waiting: [{}, {}],
      lastRun: null,
      searching: false,
    } as unknown as Queue;
    const rows = failureRows([stored(1)], [], {}, {});
    expect(needsYouCount(queue, rows, 3)).toBe(2 + 1 + 3);
    expect(needsYouCount(undefined, [], 0)).toBe(0);
  });

  it("says where a lead being prepared stands", () => {
    expect(jobLabel({ state: "queued" })).toBe("Waiting for its turn");
    expect(jobLabel({ state: "running", label: "Writing the CV" })).toBe("Writing the CV");
  });
});
