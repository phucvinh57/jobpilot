import { describe, expect, it } from "vitest";

import { disconnectLabel, formatSyncTime, lastSyncLine, pollLine, secretLabel, syncResult } from "./gmail";

describe("the Gmail card", () => {
  it("formats the last sync in local time", () => {
    // Built from local parts, so the test does not depend on the time zone it runs in.
    expect(formatSyncTime(new Date(2026, 10, 14, 9, 5))).toBe("2026-11-14 09:05");
    expect(formatSyncTime(new Date(2026, 0, 2, 23, 59))).toBe("2026-01-02 23:59");
    expect(lastSyncLine(null)).toBe("Not synced yet");
    expect(lastSyncLine(new Date(2026, 10, 14, 9, 5).toISOString())).toBe("Last sync: 2026-11-14 09:05");
  });

  it("says what the last sync did", () => {
    expect(syncResult(null)).toEqual({ text: "", ok: true });
    expect(syncResult({ fetched: 4, updated: 1, needsReview: 0, error: null })).toEqual({
      text: "fetched 4, updated 1",
      ok: true,
    });
    expect(syncResult({ fetched: 4, updated: 1, needsReview: 1, error: null }).text).toBe(
      "fetched 4, updated 1, 1 needs review",
    );
    expect(syncResult({ fetched: 4, updated: 1, needsReview: 3, error: null }).text).toBe(
      "fetched 4, updated 1, 3 need review",
    );
    expect(syncResult({ fetched: 0, updated: 0, needsReview: 0, error: "Gmail is not connected" })).toEqual({
      text: "Sync problem: Gmail is not connected",
      ok: false,
    });
  });

  it("states how often mail is checked", () => {
    expect(pollLine(10)).toBe("New mail is checked every 10 min while JobPilot is open.");
  });

  it("says when a saved secret stays and arms the disconnect in two steps", () => {
    expect(secretLabel(false)).toBe("Client secret");
    expect(secretLabel(true)).toBe("Client secret (leave empty to keep the saved one)");
    expect(disconnectLabel(false)).toBe("Disconnect");
    expect(disconnectLabel(true)).toBe("Click again to disconnect");
  });
});
