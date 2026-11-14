import { describe, expect, it } from "vitest";

import { activityLabel, cardLabel, columnLabel, pipelineSummary } from "./pipeline";
import type { BoardCard } from "./types";

const card: BoardCard = {
  id: 1,
  title: "Backend Engineer",
  subtitle: "Acme",
  status: "CV ready",
  next: { label: "Review & submit", tone: "accent" },
  age: "today",
};

describe("the Pipeline wording", () => {
  it("counts the applications", () => {
    expect(pipelineSummary(0)).toBe("No applications yet.");
    expect(pipelineSummary(1)).toBe("1 application. Stages update from your Gmail replies.");
    expect(pipelineSummary(4)).toBe("4 applications. Stages update from your Gmail replies.");
  });

  it("names a card by company, role, next action and status", () => {
    expect(cardLabel(card)).toBe("Acme, Backend Engineer: Review & submit, CV ready");
    expect(cardLabel({ ...card, subtitle: "" })).toBe("Backend Engineer: Review & submit, CV ready");
  });

  it("names a column and an activity row", () => {
    expect(columnLabel("Submitted", 3)).toBe("Submitted, 3");
    expect(activityLabel({ id: 1, appId: 1, text: "Acme: moved to Applied", when: "09:05", tone: "accent" })).toBe(
      "Acme: moved to Applied, 09:05",
    );
  });
});
