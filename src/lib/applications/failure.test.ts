import { describe, expect, it } from "vitest";

import { CommandError, type Problem } from "@/lib/problem";

import { sameFailure } from "./failure";

const network: Problem = {
  kind: "network",
  challenge: null,
  advice: "Check your network.",
  settingsSection: null,
  retryable: true,
};

describe("the same failure", () => {
  it("is the same kind and message, read again", () => {
    expect(sameFailure(new CommandError("timed out", network), new CommandError("timed out", network))).toBe(true);
  });

  it("is another failure with another message or kind", () => {
    expect(sameFailure(new CommandError("timed out", network), new CommandError("refused", network))).toBe(false);
    expect(sameFailure(new CommandError("timed out", network), new CommandError("timed out"))).toBe(false);
  });

  it("is none on both sides, never one", () => {
    expect(sameFailure(null, undefined)).toBe(true);
    expect(sameFailure(null, new CommandError("timed out"))).toBe(false);
    expect(sameFailure(new CommandError("timed out"), undefined)).toBe(false);
  });
});
