import { describe, expect, it } from "vitest";

import { CommandError, needsRecheck, OTHER, problemActions, toCommandError, type Problem } from "./problem";

const problem = (over: Partial<Problem>): Problem => ({ ...OTHER, ...over });

describe("toCommandError", () => {
  it("keeps the shell's message and problem", () => {
    const e = toCommandError({ message: "connection refused", problem: problem({ kind: "network", retryable: true }) });
    expect(e).toBeInstanceOf(CommandError);
    expect(e.message).toBe("connection refused");
    expect(e.problem.kind).toBe("network");
  });

  it("falls back to a plain error", () => {
    expect(toCommandError("boom").message).toBe("boom");
    expect(toCommandError("boom").problem).toEqual(OTHER);
    expect(toCommandError(undefined).problem.kind).toBe("other");
    expect(toCommandError({ message: "x", problem: 3 }).problem).toEqual(OTHER);
  });
});

describe("problemActions", () => {
  it("offers the Settings shortcut for a problem Settings fixes", () => {
    expect(problemActions(problem({ kind: "noProvider", settingsSection: "AI providers" }), true)).toEqual([
      "settings",
    ]);
  });

  it("sends a missing profile to Profile > About you", () => {
    expect(problemActions(problem({ kind: "noProfile", settingsSection: "About you" }), true)).toEqual(["settings"]);
  });

  it("offers Check again for a missing typst", () => {
    expect(problemActions(problem({ kind: "noTypst" }), false)).toEqual(["recheck"]);
  });

  it("offers Open browser when a site needs the user", () => {
    expect(problemActions(problem({ kind: "needsHuman", challenge: "captcha" }), false)).toEqual(["openBrowser"]);
  });

  it("offers Retry only for a retryable problem with something to retry", () => {
    const network = problem({ kind: "network", retryable: true });
    expect(problemActions(network, true)).toEqual(["retry"]);
    expect(problemActions(network, false)).toEqual([]);
    expect(problemActions(OTHER, true)).toEqual([]);
  });
});

describe("needsRecheck", () => {
  it("is true for a missing prerequisite", () => {
    expect(needsRecheck(problem({ kind: "noProvider" }))).toBe(true);
    expect(needsRecheck(problem({ kind: "noChrome" }))).toBe(true);
    expect(needsRecheck(problem({ kind: "noProfile" }))).toBe(true);
    expect(needsRecheck(problem({ kind: "network" }))).toBe(false);
  });
});
