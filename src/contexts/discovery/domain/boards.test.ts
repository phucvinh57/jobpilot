import { describe, expect, it } from "vitest";

import { MAX_SUGGESTIONS, shownSuggestions, suggestionText, suggestTarget } from "./boards";

describe("the board behind a careers page", () => {
  it("is only looked for on a careers page with an address", () => {
    expect(suggestTarget("career_page", " acme.com/careers ")).toBe("acme.com/careers");
    expect(suggestTarget("greenhouse", "acme.com/careers")).toBeUndefined();
    expect(suggestTarget("career_page", "")).toBeUndefined();
    expect(suggestTarget("career_page", "acme")).toBeUndefined();
    expect(suggestTarget("career_page", "a.b")).toBeUndefined();
    expect(suggestTarget("career_page", "acme.")).toBeUndefined();
    expect(suggestTarget("career_page", "acme com/careers")).toBeUndefined();
    expect(suggestTarget("career_page", "ftp://acme.com/careers")).toBeUndefined();
    expect(suggestTarget("career_page", "https://jobs.acme.co/open")).toBe("https://jobs.acme.co/open");
  });

  it("is offered in words, at most three at a time, with ids unique per suggestion", () => {
    const hint = { kind: "greenhouse", label: "Greenhouse", input: "boards.greenhouse.io/acme" };
    expect(suggestionText(hint)).toBe("This page lists its jobs on Greenhouse (boards.greenhouse.io/acme). Add it as a Greenhouse source instead?");
    const two = shownSuggestions([hint, { ...hint, input: "boards.greenhouse.io/beta" }, { kind: "a b<c>", label: "X", input: "x" }]);
    expect(two.map((s) => [s.bannerId, s.buttonId])).toEqual([
      ["board-suggestion-greenhouse-0", "suggest-board-greenhouse-0"],
      ["board-suggestion-greenhouse-1", "suggest-board-greenhouse-1"],
      ["board-suggestion-abc-2", "suggest-board-abc-2"],
    ]);
    expect(shownSuggestions(Array.from({ length: 5 }, () => hint))).toHaveLength(MAX_SUGGESTIONS);
  });
});
