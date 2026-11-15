import { describe, expect, it } from "vitest";

import { ago, scoreLabel, scoreTone } from "@/shared/ui/rules";

import {
  capitalize,
  dismissedNote,
  isWebLink,
  lastSearchLine,
  leadTitle,
  metaLine,
  reasonsLine,
  shorten,
  sourceName,
  sourceNeedsAttention,
  sourceStatus,
  summaryLine,
  todayLabel,
} from "./text";

describe("the search summary", () => {
  it("reads as a sentence", () => {
    expect(summaryLine({ saved: 3, waiting: 0, failed: 0 })).toBe("Found 3 new leads");
    expect(summaryLine({ saved: 1, waiting: 1, failed: 2 })).toBe(
      "Found 1 new lead, 1 source waiting for you, 2 sources failed",
    );
    expect(summaryLine({ saved: 0, waiting: 0, failed: 0 })).toBe("Found 0 new leads");
  });

  it("says when the last search was", () => {
    const now = new Date("2026-11-14T12:00:00Z");
    const run = { finishedAt: "2026-11-14T11:55:00Z", saved: 2, waiting: 0, failed: 0 };
    expect(lastSearchLine(run, now)).toBe("Last search 5 min ago: Found 2 new leads");
    expect(lastSearchLine({ ...run, finishedAt: "" }, now)).toBe("Last search earlier: Found 2 new leads");
  });
});

describe("the dismiss note", () => {
  it("says what else went and what was kept", () => {
    const note = (also: number, kept: number) => dismissedNote({ also, kept, notExcluded: null });
    expect(note(0, 0)).toBe("Lead dismissed.");
    expect(note(1, 0)).toBe("Lead dismissed, and 1 more job from the same company.");
    expect(note(3, 0)).toBe("Lead dismissed, and 3 more jobs from the same company.");
    expect(note(0, 1)).toBe("Lead dismissed. 1 more job from that company was kept: the CV is being prepared.");
    expect(dismissedNote({ also: 0, kept: 0, notExcluded: "disk full" })).toBe(
      "Lead dismissed, but the company could not be excluded: disk full",
    );
    expect(note(2, 2)).toBe(
      "Lead dismissed, and 2 more jobs from the same company. 2 more jobs from that company were kept: the CV is being prepared.",
    );
  });
});

describe("scores and times", () => {
  // These rules live in the design kit (the Pipeline and the Application screens use them too); the
  // legacy Today tests are kept here so that Today's expectations stay pinned.
  it("scores and tones", () => {
    expect(scoreLabel(87.6)).toBe("88");
    expect(scoreLabel(41.4)).toBe("41");
    expect(scoreLabel(null)).toBe("–");
    expect(scoreTone(75)).toBe("ok");
    expect(scoreTone(74.9)).toBe("warn");
    expect(scoreTone(50)).toBe("warn");
    expect(scoreTone(10)).toBe("danger");
    expect(scoreTone(null)).toBe("neutral");
  });

  it("times read naturally", () => {
    const now = new Date("2026-11-14T12:00:00Z");
    const before = (minutes: number) => new Date(now.getTime() - minutes * 60_000);
    expect(ago(now, before(1 / 3))).toBe("just now");
    expect(ago(now, before(5))).toBe("5 min ago");
    expect(ago(now, before(130))).toBe("2 h ago");
    expect(ago(now, before(3 * 24 * 60))).toBe("3 d ago");
    expect(ago(now, before(-5)), "a clock a little ahead is not the past").toBe("just now");
  });

  it("the date above the title", () => {
    expect(todayLabel(new Date(2026, 10, 14))).toBe("Saturday, 14 November");
    expect(todayLabel(new Date(2026, 0, 5))).toBe("Monday, 5 January");
  });
});

describe("sources on Today", () => {
  const source = (name: string, status: string | null, ran: boolean) => ({
    kind: "greenhouse",
    name,
    lastStatus: status,
    lastRunAt: ran ? "2026-11-14T11:00:00Z" : null,
  });
  const now = new Date("2026-11-14T12:00:00Z");

  it("a source line says how the last search went", () => {
    expect(sourceName(source("Acme careers", null, false))).toBe("Acme careers");
    expect(sourceName(source("  ", null, false))).toBe("greenhouse");
    expect(sourceStatus(source("a", null, false), now)).toBe("not searched yet");
    expect(sourceStatus(source("a", "ok", true), now)).toBe("ok, 1 h ago");
    expect(sourceStatus(source("a", "needs_human", false), now)).toBe("needs you");
    expect(sourceStatus(source("a", "paused for today: repeated challenges", true), now)).toBe(
      "paused for today: repeated challenges, 1 h ago",
    );
    expect(sourceStatus(source("a", "failed_to_fetch", false), now), "underscores read as spaces").toBe(
      "failed to fetch",
    );
    expect(sourceNeedsAttention(source("a", "needs_human", false))).toBe(true);
    expect(sourceNeedsAttention(source("a", "failed: boom", true))).toBe(true);
    expect(sourceNeedsAttention(source("a", "ok", true))).toBe(false);
    expect(sourceNeedsAttention(source("a", null, false))).toBe(false);
  });
});

describe("links and meta lines", () => {
  it("only http(s) links are opened", () => {
    expect(isWebLink("https://a.test/jobs/1") && isWebLink(" HTTP://a.test")).toBe(true);
    expect(
      isWebLink("file:///etc/passwd") || isWebLink("javascript:alert(1)") || isWebLink("manual:abc") || isWebLink(""),
    ).toBe(false);
  });

  it("the line under a lead's name says where", () => {
    expect(metaLine("Singapore", "Acme careers")).toBe("Singapore · Acme careers");
    expect(metaLine("", "HN")).toBe("HN");
    expect(metaLine(" Remote ", null)).toBe("Remote");
    expect(metaLine("", null)).toBe("location not stated");
  });

  it("names a lead and lists its reasons", () => {
    expect(leadTitle("Acme", "Engineer")).toBe("Acme — Engineer");
    expect(
      reasonsLine([
        { criterion: "skills", score: 90, text: "Rust matches" },
        { criterion: "location", score: 100, text: "remote" },
      ]),
    ).toBe("skills 90: Rust matches · location 100: remote");
    expect(reasonsLine([])).toBe("");
  });
});

describe("text helpers", () => {
  it("long text is cut with an ellipsis", () => {
    expect(shorten("short", 10)).toBe("short");
    expect(shorten("abcdefghij", 5)).toBe("abcd…");
    expect(shorten("abcde", 5)).toBe("abcde");
  });

  it("criteria are capitalised", () => {
    expect(capitalize("skills")).toBe("Skills");
    expect(capitalize("")).toBe("");
  });
});
