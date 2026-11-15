import { describe, expect, it } from "vitest";

import { compactAgo, kindLabel, rowAddress, rowName, sourceTag, statusLine, STATUS_CHARS } from "./sources";
import type { SourceKind } from "./types";

const KINDS: SourceKind[] = [
  { kind: "greenhouse", label: "Greenhouse", hint: "" },
  { kind: "yc", label: "Y Combinator", hint: "" },
  { kind: "career_page", label: "Career page", hint: "" },
];

const NOW = new Date("2026-11-14T12:00:00Z");
const hoursAgo = (h: number) => new Date(NOW.getTime() - h * 3_600_000).toISOString();
const source = (lastRunAt: string | null, lastStatus: string | null) => ({ lastRunAt, lastStatus });

describe("a source's status line", () => {
  it("says when and how the last run ended", () => {
    expect(statusLine(source(null, null), NOW)).toEqual({ line: "never run", outcome: "never" });
    expect(statusLine(source(hoursAgo(3), "ok"), NOW)).toEqual({ line: "ok - 3h ago", outcome: "ok" });
    expect(statusLine(source(NOW.toISOString(), "ok"), NOW).line).toBe("ok - just now");
    expect(statusLine(source(hoursAgo(1), "failed: fetching boards.test: HTTP 503"), NOW)).toEqual({
      line: "failed: fetching boards.test: HTTP 503 - 1h ago",
      outcome: "failed",
    });
    expect(statusLine(source(null, "needs_human"), NOW).outcome).toBe("needsYou");
    expect(statusLine(source(hoursAgo(2), "needs_human"), NOW).outcome, "a paused source is not finished").toBe(
      "needsYou",
    );
    expect(statusLine(source(hoursAgo(30), "paused for today: repeated challenges"), NOW).outcome).toBe("other");
    expect(statusLine(source(hoursAgo(2), null), NOW).line).toBe("ran 2h ago");
    expect(statusLine(source(null, "running"), NOW)).toEqual({ line: "running", outcome: "other" });
    const { line } = statusLine(source(hoursAgo(1), `failed: ${"x".repeat(300)}`), NOW);
    expect(Array.from(line).length < STATUS_CHARS + 20 && line.includes("..."), line).toBe(true);
  });

  it("shows a whole 'links to a board' note", () => {
    const note = "No postings on the page. It links to a SmartRecruiters board: add it as a SmartRecruiters source";
    expect(statusLine(source(hoursAgo(1), note), NOW)).toEqual({ line: `${note} - 1h ago`, outcome: "boardHint" });
    expect(sourceTag("boardHint", false)).toEqual({ text: "see note", tone: "accent" });
    expect(sourceTag("boardHint", true).text, "a paused source still needs the user").toBe("needs you");
    expect(statusLine(source(hoursAgo(1), "No postings on this page"), NOW).outcome).toBe("other");
  });

  it("points a paused source to the Jobs page", () => {
    expect(statusLine(source(null, "needs_human"), NOW).line).toContain("Jobs");
  });

  it("ages read compactly", () => {
    expect(compactAgo(NOW, new Date(NOW.getTime() - 20_000))).toBe("just now");
    expect(compactAgo(NOW, new Date(NOW.getTime() - 5 * 60_000))).toBe("5m ago");
    expect(compactAgo(NOW, new Date(NOW.getTime() - 3 * 3_600_000))).toBe("3h ago");
    expect(compactAgo(NOW, new Date(NOW.getTime() - 50 * 3_600_000))).toBe("2d ago");
  });
});

describe("a source row", () => {
  it("kinds have labels and unknown ones show as they are", () => {
    expect(kindLabel(KINDS, "career_page")).toBe("Career page");
    expect(kindLabel(KINDS, "yc")).toBe("Y Combinator");
    expect(kindLabel(KINDS, "linkedin")).toBe("linkedin");
  });

  it("is named by itself or by its kind, with the host under it", () => {
    expect(rowName({ name: " Acme ", kind: "greenhouse" }, KINDS)).toBe("Acme");
    expect(rowName({ name: " ", kind: "greenhouse" }, KINDS)).toBe("Greenhouse");
    expect(rowAddress({ kind: "greenhouse", host: "boards.greenhouse.io" }, KINDS)).toBe(
      "Greenhouse boards.greenhouse.io",
    );
    expect(rowAddress({ kind: "yc", host: "" }, KINDS)).toBe("Y Combinator");
  });

  it("is tagged by how it ended, and a paused source always needs the user", () => {
    expect(sourceTag("ok", false)).toEqual({ text: "ok", tone: "ok" });
    expect(sourceTag("failed", false)).toEqual({ text: "failed", tone: "danger" });
    expect(sourceTag("never", false)).toEqual({ text: "new", tone: "neutral" });
    expect(sourceTag("other", false)).toEqual({ text: "idle", tone: "neutral" });
    expect(sourceTag("needsYou", false)).toEqual({ text: "needs you", tone: "warn" });
    expect(sourceTag("ok", true)).toEqual({ text: "needs you", tone: "warn" });
  });
});
