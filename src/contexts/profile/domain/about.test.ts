import { describe, expect, it } from "vitest";

import {
  educationLine,
  noteProblem,
  roleLine,
  sizeLabel,
  sourceLine,
  statusLine,
  totalChars,
  truncatedNote,
  type About,
} from "./about";

function about(patch: Partial<About> = {}): About {
  return {
    sources: [{ id: 1, name: "cv.pdf", kind: "file", chars: 4200, truncated: false }],
    status: "ready",
    summary: null,
    leftOut: [],
    builtAt: "2026-11-15T10:00:00Z",
    limits: { maxChars: 20000, maxTotalChars: 60000 },
    ...patch,
  };
}

const NOW = new Date("2026-11-15T10:05:00Z");

describe("sizes", () => {
  it("are short and round", () => {
    expect(sizeLabel(1)).toBe("1 character");
    expect(sizeLabel(820)).toBe("820 characters");
    expect(sizeLabel(4200)).toBe("4.2k characters");
    expect(sizeLabel(20000)).toBe("20k characters");
    expect(sizeLabel(12345)).toBe("12k characters");
  });

  it("describe a document and a cut one", () => {
    expect(sourceLine({ id: 1, name: "cv.pdf", kind: "file", chars: 4200, truncated: false })).toBe(
      "File · 4.2k characters",
    );
    expect(sourceLine({ id: 2, name: "On-call", kind: "text", chars: 300, truncated: false })).toBe(
      "Note · 300 characters",
    );
    expect(truncatedNote(20000)).toBe("Only the first 20k characters were kept.");
    expect(totalChars(about().sources)).toBe(4200);
  });
});

describe("the status line", () => {
  it("says a read is running and offers no Re-read meanwhile", () => {
    expect(statusLine(about(), true, NOW)).toEqual({ text: "Reading your documents…", tone: "muted", reread: false });
  });

  it("follows where the reading stands", () => {
    expect(statusLine(about({ status: "empty", sources: [] }), false, NOW)).toMatchObject({ reread: false });
    expect(statusLine(about({ status: "unread" }), false, NOW)).toEqual({
      text: "Not read yet.",
      tone: "warn",
      reread: true,
    });
    expect(statusLine(about({ status: "stale" }), false, NOW).text).toMatch(/^Out of date/);
    expect(statusLine(about(), false, NOW)).toEqual({ text: "Read 5 min ago.", tone: "ok", reread: true });
    expect(statusLine(about({ builtAt: "garbage" }), false, NOW).text).toBe("Read.");
  });
});

describe("the summary lines", () => {
  it("join what is there", () => {
    expect(roleLine({ position: "Engineer", company: "Acme", dates: "2021 – 2024" })).toBe(
      "Engineer — Acme · 2021 – 2024",
    );
    expect(roleLine({ position: "Engineer", company: "", dates: "" })).toBe("Engineer");
    expect(roleLine({ position: "", company: "Acme", dates: "2020" })).toBe("Acme · 2020");
    expect(educationLine({ degree: "BSc", institution: "HCMUT", dates: "2015 – 2019" })).toBe(
      "BSc — HCMUT · 2015 – 2019",
    );
  });
});

describe("a note", () => {
  it("needs text and fits the limits", () => {
    expect(noteProblem("   ", about())).toMatch(/Write something/);
    expect(noteProblem("I ran the on-call rota.", about())).toBeNull();
    expect(noteProblem("x".repeat(20001), about())).toMatch(/under 20k characters/);
    const full = about({ sources: [{ id: 1, name: "a", kind: "text", chars: 59990, truncated: false }] });
    expect(noteProblem("x".repeat(11), full)).toMatch(/remove one first/);
  });
});
