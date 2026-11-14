import { describe, expect, it } from "vitest";

import { board } from "./fixtures";
import { activityLabel, appStages, cardLabel, cardTag, emptyHint, shownActivity, stageCards, stageLabel, stageOf } from "./pipeline";
import type { ActivityItem, BoardCard } from "./types";

const card: BoardCard = {
  id: 1,
  title: "Backend Engineer",
  subtitle: "Acme",
  status: "CV ready",
  next: { label: "Review & submit", tone: "accent" },
  age: "today",
  round: null,
  outcome: null,
  stage: "cv",
};

describe("the application stages", () => {
  it("counts the cards of every stage, in order", () => {
    expect(appStages(board())).toEqual([
      { key: "cv", label: "CVs", count: 2 },
      { key: "applied", label: "Applied", count: 1 },
      { key: "inProgress", label: "In progress", count: 1 },
      { key: "done", label: "Done", count: 2 },
    ]);
  });

  it("shows every stage with 0 before the board is read", () => {
    expect(appStages(undefined).map((s) => [s.key, s.count])).toEqual([
      ["cv", 0],
      ["applied", 0],
      ["inProgress", 0],
      ["done", 0],
    ]);
    expect(stageLabel("inProgress")).toBe("In progress");
  });

  it("lists the cards of a stage", () => {
    expect(stageCards(board(), "applied").map((c) => c.id)).toEqual([3]);
    expect(stageCards({ count: 0, stages: [], activity: [] }, "done")).toEqual([]);
    expect(stageOf(board(), 4)).toBe("inProgress");
    expect(stageOf(board(), 99)).toBeUndefined();
  });

  it("says how an application ended, else what happens next", () => {
    expect(cardTag({ ...card, outcome: "success", stage: "done" })).toEqual({ label: "Offer", tone: "ok" });
    expect(cardTag({ ...card, outcome: "failed", stage: "done" })).toEqual({ label: "Rejected", tone: "danger" });
    expect(cardTag(card)).toEqual({ label: "Review & submit", tone: "accent" });
    const interview = { ...card, stage: "inProgress" as const, round: "System design", next: { label: "System design", tone: "warn" as const } };
    expect(cardTag(interview)).toEqual({ label: "System design", tone: "warn" });
  });

  it("explains every empty stage", () => {
    for (const key of ["cv", "applied", "inProgress", "done"] as const) {
      expect(emptyHint(key).length).toBeGreaterThan(10);
    }
  });

  it("shows the first rows of the activity until all are asked for", () => {
    const items: ActivityItem[] = Array.from({ length: 10 }, (_, i) => ({ id: i, appId: 1, text: `e${i}`, when: "09:00", tone: "neutral" }));
    expect(shownActivity(items, false)).toHaveLength(8);
    expect(shownActivity(items, true)).toHaveLength(10);
  });
});

describe("the names of rows", () => {
  it("names a card by company, role, next action and status", () => {
    expect(cardLabel(card)).toBe("Acme, Backend Engineer: Review & submit, CV ready");
    expect(cardLabel({ ...card, subtitle: "" })).toBe("Backend Engineer: Review & submit, CV ready");
  });

  it("names an activity row", () => {
    expect(activityLabel({ id: 1, appId: 1, text: "Acme: moved to Applied", when: "09:05", tone: "accent" })).toBe("Acme: moved to Applied, 09:05");
  });
});
