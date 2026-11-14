import { describe, expect, it } from "vitest";

import { suggestButtonId, suggestionText, suggestTarget } from "./boards";

describe("the board behind a careers page", () => {
  it("is only looked for on a careers page with an address", () => {
    expect(suggestTarget("career_page", " acme.com/careers ")).toBe("acme.com/careers");
    expect(suggestTarget("greenhouse", "acme.com/careers")).toBeUndefined();
    expect(suggestTarget("career_page", "")).toBeUndefined();
    expect(suggestTarget("career_page", "acme")).toBeUndefined();
    expect(suggestTarget("career_page", "a.b")).toBeUndefined();
  });

  it("is offered in words and by a button with a stable id", () => {
    const hint = { kind: "greenhouse", label: "Greenhouse", input: "boards.greenhouse.io/acme" };
    expect(suggestionText(hint)).toBe("This page lists its jobs on Greenhouse (boards.greenhouse.io/acme). Add it as a Greenhouse source instead?");
    expect(suggestButtonId(hint)).toBe("suggest-board-greenhouse");
    expect(suggestButtonId({ kind: "a b<c>" })).toBe("suggest-board-abc");
  });
});
