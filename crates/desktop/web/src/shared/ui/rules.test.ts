import { describe, expect, it } from "vitest";

import { ago, barFraction, scoreLabel, scoreTone, stepState } from "./rules";

describe("kit rules", () => {
  it("steps before the current are done", () => {
    expect(stepState(0, 2)).toBe("done");
    expect(stepState(1, 2)).toBe("done");
    expect(stepState(2, 2)).toBe("current");
    expect(stepState(3, 2)).toBe("todo");
  });

  it("a current past the end means all done", () => {
    expect([0, 1, 2, 3, 4].every((i) => stepState(i, 5) === "done")).toBe(true);
    expect(stepState(0, 0)).toBe("current");
  });

  it("bars are clamped", () => {
    expect(barFraction(50, 100)).toBe(0.5);
    expect(barFraction(150, 100)).toBe(1);
    expect(barFraction(-5, 100)).toBe(0);
    expect(barFraction(3, 0)).toBe(0);
    expect(barFraction(Number.NaN, 10)).toBe(0);
  });

  it("scores map to tones", () => {
    expect(scoreTone(90)).toBe("ok");
    expect(scoreTone(75)).toBe("ok");
    expect(scoreTone(60)).toBe("warn");
    expect(scoreTone(10)).toBe("danger");
    expect(scoreTone(null)).toBe("neutral");
  });

  it("a missing score is a dash", () => {
    expect(scoreLabel(82.4)).toBe("82");
    expect(scoreLabel(null)).toBe("–");
  });

  it("times read as how long ago", () => {
    const now = new Date("2026-01-10T12:00:00Z");
    const before = (minutes: number) => new Date(now.getTime() - minutes * 60_000);
    expect(ago(now, before(0))).toBe("just now");
    expect(ago(now, before(5))).toBe("5 min ago");
    expect(ago(now, before(185))).toBe("3 h ago");
    expect(ago(now, before(60 * 24 * 2 + 5))).toBe("2 d ago");
  });
});
