import { describe, expect, it } from "vitest";

import { browserLine, browserTag, downloadLabel, FIRST_SUGGESTION, sessionLine, statusTone, SUGGESTIONS, type Chrome } from "./sessions";

const CHROME: Chrome = { path: "/usr/bin/chromium", source: "system" };

describe("sessions", () => {
  it("shows the tone of a login state", () => {
    expect(statusTone("ok")).toBe("ok");
    expect(statusTone("expired")).toBe("danger");
    expect(statusTone("new")).toBe("neutral");
  });

  it("says when a session was last verified", () => {
    const now = new Date("2026-11-14T12:00:00Z");
    expect(sessionLine({ host: "www.linkedin.com", verifiedAt: null }, now)).toBe("www.linkedin.com - never verified");
    expect(sessionLine({ host: "www.linkedin.com", verifiedAt: "2026-11-14T11:55:00Z" }, now)).toBe("www.linkedin.com - verified 5 min ago");
    expect(sessionLine({ host: "indeed.com", verifiedAt: "2026-11-12T12:00:00Z" }, now)).toBe("indeed.com - verified 2 d ago");
  });

  it("offers quick starts, the first one filling the form", () => {
    expect(SUGGESTIONS.map((s) => s.name)).toEqual(["LinkedIn", "Indeed", "Google"]);
    expect(SUGGESTIONS.every((s) => s.url.startsWith("https://"))).toBe(true);
    expect(FIRST_SUGGESTION.name).toBe("LinkedIn");
  });
});

describe("the browser line", () => {
  it("names the browser, or says there is none", () => {
    expect(browserLine(CHROME, { active: false, percent: null })).toBe("Browser: /usr/bin/chromium (system)");
    expect(browserLine(null, { active: false, percent: null })).toBe("Browser: not found");
  });

  it("follows the download", () => {
    expect(browserLine(null, { active: true, percent: null })).toBe("Downloading browser...");
    expect(browserLine(null, { active: true, percent: 42 })).toBe("Downloading browser... 42%");
    expect(browserLine(CHROME, { active: true, percent: 7 })).toBe("Downloading browser... 7%");
  });

  it("labels the button and the tag", () => {
    expect(downloadLabel(CHROME)).toBe("Update browser");
    expect(downloadLabel(null)).toBe("Download browser");
    expect(browserTag(CHROME, { active: false, percent: null })).toEqual({ tone: "ok", text: "Chrome found" });
    expect(browserTag(null, { active: true, percent: 3 })).toEqual({ tone: "neutral", text: "Downloading" });
    expect(browserTag(null, { active: false, percent: null })).toEqual({ tone: "warn", text: "No browser" });
  });
});
