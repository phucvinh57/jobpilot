// Realistic sample data for the Settings > Browser tests (not shipped: only the tests import it).

import type { BrowserSection, BrowserStatus, Session } from "@/types/integrations";

/** A paired Chrome with the extension connected, allowed in Incognito. */
export const CONNECTED: BrowserStatus = {
  listening: true,
  port: 47615,
  connected: true,
  browser: "Chrome 153",
  extensionVersion: "0.1.0",
  incognitoAllowed: true,
  pairingCode: null,
  paired: true,
  extensionPath: "/opt/jobpilot/extension",
};

/** No extension has connected yet. */
export const DISCONNECTED: BrowserStatus = {
  ...CONNECTED,
  connected: false,
  browser: null,
  extensionVersion: null,
  incognitoAllowed: false,
  paired: false,
};

/** An extension waits for the user to approve its code. */
export const PAIRING: BrowserStatus = {
  ...DISCONNECTED,
  browser: "Chrome 153",
  extensionVersion: "0.1.0",
  pairingCode: "482913",
};

export function site(patch: Partial<Session> = {}): Session {
  return {
    id: 1,
    name: "LinkedIn",
    host: "www.linkedin.com",
    status: "ok",
    verifiedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
    loginInProgress: false,
    ...patch,
  };
}

/** The section with two sites: one signed in, one signed out and never checked. */
export function browserSection(patch: Partial<BrowserSection> = {}): BrowserSection {
  return {
    browser: CONNECTED,
    sessions: [site(), site({ id: 2, name: "Indeed", host: "secure.indeed.com", status: "expired", verifiedAt: null })],
    ...patch,
  };
}
