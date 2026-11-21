import { describe, expect, it } from "vitest";

import type { SuggestTerms } from "@/types/discovery";

import { canSearch, isPicked, pickedCount, searchNote, toggleTerm } from "./suggestTerms";

function terms(over: Partial<SuggestTerms> = {}): SuggestTerms {
  return { roles: [], domains: [], skills: [], places: [], remote: false, ...over };
}

describe("toggleTerm", () => {
  it("adds a value that is not picked and removes one that is", () => {
    const start = terms({ domains: ["fintech"] });
    const added = toggleTerm(start, "domains", "climate");
    expect(added.domains).toEqual(["fintech", "climate"]);
    expect(toggleTerm(added, "domains", "fintech").domains).toEqual(["climate"]);
  });

  it("leaves the other lists and the input alone", () => {
    const start = terms({ roles: ["Rust Engineer"], places: ["Berlin"], remote: true });
    const next = toggleTerm(start, "roles", "Rust Engineer");
    expect(next).toEqual(terms({ places: ["Berlin"], remote: true }));
    expect(start.roles).toEqual(["Rust Engineer"]);
    expect(next.places).toBe(start.places);
  });
});

describe("isPicked", () => {
  it("looks in the list of that kind only", () => {
    const t = terms({ skills: ["Rust"] });
    expect(isPicked(t, "skills", "Rust")).toBe(true);
    expect(isPicked(t, "roles", "Rust")).toBe(false);
  });
});

describe("pickedCount and canSearch", () => {
  it("counts roles, domains and skills, not places or Remote", () => {
    expect(pickedCount(terms({ places: ["Berlin"], remote: true }))).toBe(0);
    expect(canSearch(terms({ places: ["Berlin"], remote: true }))).toBe(false);
    const t = terms({ roles: ["A"], domains: ["b", "c"], skills: ["D"] });
    expect(pickedCount(t)).toBe(4);
    expect(canSearch(t)).toBe(true);
  });
});

describe("searchNote", () => {
  it("asks for a term when none is picked", () => {
    expect(searchNote(terms({ places: ["Berlin"] }))).toBe("Pick at least one role, domain or skill.");
  });

  it("counts 4 searches per term without places", () => {
    expect(searchNote(terms({ domains: ["fintech"] }))).toBe("1 term picked: up to 4 searches.");
  });

  it("combines each term with up to 2 places, Remote counting as one", () => {
    expect(searchNote(terms({ roles: ["A"], places: ["Berlin"] }))).toBe("1 term picked: up to 4 searches.");
    expect(searchNote(terms({ roles: ["A"], places: ["Berlin"], remote: true }))).toBe(
      "1 term picked: up to 8 searches.",
    );
    expect(searchNote(terms({ roles: ["A"], places: ["Berlin", "Paris", "Rome"] }))).toBe(
      "1 term picked: up to 8 searches.",
    );
  });

  it("says the 8 searches are shared when the terms need more", () => {
    expect(searchNote(terms({ roles: ["A"], skills: ["B"] }))).toBe("2 terms picked: up to 8 searches.");
    expect(searchNote(terms({ roles: ["A"], domains: ["b", "c", "d"], skills: ["E"] }))).toBe(
      "5 terms picked: up to 8 searches, shared between them.",
    );
    expect(searchNote(terms({ roles: ["A", "B"], remote: true, places: ["Berlin"] }))).toBe(
      "2 terms picked: up to 8 searches, shared between them.",
    );
  });
});
