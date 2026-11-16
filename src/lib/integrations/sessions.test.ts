import { describe, expect, it } from "vitest";

import { CONNECTED, DISCONNECTED, PAIRING } from "@/test/fixtures/integrations";
import {
  connectionLine,
  connectionTag,
  disconnectedLine,
  FIRST_SUGGESTION,
  privateTabsLine,
  sessionLine,
  statusLabel,
  statusTone,
  SUGGESTIONS,
} from "./sessions";

describe("sites", () => {
  it("shows the tone and the words of a sign-in state", () => {
    expect(statusTone("ok")).toBe("ok");
    expect(statusTone("expired")).toBe("danger");
    expect(statusTone("new")).toBe("neutral");
    expect([statusLabel("ok"), statusLabel("expired"), statusLabel("new")]).toEqual([
      "signed in",
      "signed out",
      "not checked",
    ]);
  });

  it("says when a site was last checked", () => {
    const now = new Date("2026-11-14T12:00:00Z");
    expect(sessionLine({ host: "www.linkedin.com", verifiedAt: null }, now)).toBe("www.linkedin.com - never checked");
    expect(sessionLine({ host: "www.linkedin.com", verifiedAt: "2026-11-14T11:55:00Z" }, now)).toBe(
      "www.linkedin.com - checked 5 min ago",
    );
    expect(sessionLine({ host: "indeed.com", verifiedAt: "2026-11-12T12:00:00Z" }, now)).toBe(
      "indeed.com - checked 2 d ago",
    );
  });

  it("offers quick starts, the first one filling the form", () => {
    expect(SUGGESTIONS.map((s) => s.name)).toEqual(["LinkedIn", "Indeed", "Google"]);
    expect(SUGGESTIONS.every((s) => s.url.startsWith("https://"))).toBe(true);
    expect(FIRST_SUGGESTION.name).toBe("LinkedIn");
  });
});

describe("the connection", () => {
  it("tags the state of the extension", () => {
    expect(connectionTag(CONNECTED)).toEqual({ tone: "ok", text: "Connected" });
    expect(connectionTag(PAIRING)).toEqual({ tone: "warn", text: "Waiting for pairing" });
    expect(connectionTag({ ...PAIRING, pairingWaitingForExtension: true })).toEqual({
      tone: "warn",
      text: "Waiting for the extension",
    });
    expect(connectionTag(DISCONNECTED)).toEqual({ tone: "warn", text: "Not connected" });
    expect(connectionTag({ ...DISCONNECTED, listening: false })).toEqual({ tone: "danger", text: "Off" });
  });

  it("names the browser and the extension", () => {
    expect(connectionLine(CONNECTED)).toBe("Chrome 153, JobPilot extension 0.1.0");
    expect(connectionLine({ ...CONNECTED, browser: null, extensionVersion: null })).toBe("Your browser");
  });

  it("says whether private tabs work and how to allow them", () => {
    expect(privateTabsLine({ incognitoAllowed: true })).toBe("Private tabs: allowed");
    expect(privateTabsLine({ incognitoAllowed: false })).toContain("Allow in Incognito");
    expect(privateTabsLine({ incognitoAllowed: false })).toContain("chrome://extensions");
  });

  it("says why no browser is connected", () => {
    expect(disconnectedLine(DISCONNECTED)).toMatch(/^Install the JobPilot extension/);
    expect(disconnectedLine({ ...DISCONNECTED, paired: true })).toMatch(/^The paired browser is not connected/);
    expect(disconnectedLine({ ...DISCONNECTED, listening: false })).toContain("port 47615");
  });
});
