// What Settings > Browser says: the connection to the JobPilot extension in the user's Chrome, how
// to install it, and each site the user is signed in to (a session).

import { ago } from "@/lib/display";
import type { Tone } from "@/lib/tokens";
import type { BrowserStatus, Session, SessionStatus } from "@/types/integrations";

/** Quick starts for the "Add a site" dialog: (name, sign-in URL). */
export const SUGGESTIONS: readonly { name: string; url: string }[] = [
  { name: "LinkedIn", url: "https://www.linkedin.com/login" },
  { name: "Indeed", url: "https://secure.indeed.com/account/login" },
  { name: "Google", url: "https://accounts.google.com/" },
];

/** The first suggestion fills the form when it opens. */
export const FIRST_SUGGESTION = SUGGESTIONS[0] ?? { name: "", url: "" };

/** `busy` key for operations that are not about one existing site (adding one). */
export const NEW_KEY = 0;

export function statusTone(status: SessionStatus): Tone {
  switch (status) {
    case "ok":
      return "ok";
    case "expired":
      return "danger";
    case "new":
      return "neutral";
  }
}

/** What the status tag of a site says. */
export function statusLabel(status: SessionStatus): string {
  switch (status) {
    case "ok":
      return "signed in";
    case "expired":
      return "signed out";
    case "new":
      return "not checked";
  }
}

/** `host - checked 5 min ago`, or `host - never checked`. */
export function sessionLine(session: Pick<Session, "host" | "verifiedAt">, now: Date): string {
  const checked = session.verifiedAt === null ? "never checked" : `checked ${ago(now, new Date(session.verifiedAt))}`;
  return `${session.host} - ${checked}`;
}

/** The tag at the top of the section. */
export function connectionTag(browser: BrowserStatus): { tone: Tone; text: string } {
  if (browser.connected) {
    return { tone: "ok", text: "Connected" };
  }
  if (browser.pairingCode !== null) {
    return { tone: "warn", text: "Waiting for pairing" };
  }
  if (!browser.listening) {
    return { tone: "danger", text: "Off" };
  }
  return { tone: "warn", text: "Not connected" };
}

/** "Chrome 153, JobPilot extension 0.1.0" for a connected browser. */
export function connectionLine(browser: BrowserStatus): string {
  const name = browser.browser ?? "Your browser";
  return browser.extensionVersion ? `${name}, JobPilot extension ${browser.extensionVersion}` : name;
}

/** Whether the extension may open private tabs (forms that need no sign-in open in one). */
export function privateTabsLine(browser: Pick<BrowserStatus, "incognitoAllowed">): string {
  return browser.incognitoAllowed
    ? "Private tabs: allowed"
    : "Private tabs: not allowed (turn on Allow in Incognito for JobPilot on chrome://extensions)";
}

/** Why the browser is not connected, in one line. */
export function disconnectedLine(browser: Pick<BrowserStatus, "listening" | "port" | "paired">): string {
  if (!browser.listening) {
    return `JobPilot cannot wait for the extension: port ${browser.port} is used by another program.`;
  }
  return browser.paired
    ? "The paired browser is not connected: open Chrome with the JobPilot extension."
    : "Install the JobPilot extension in Chrome to let JobPilot open tabs there.";
}
