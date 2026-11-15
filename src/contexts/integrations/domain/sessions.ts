// Browser sessions as the shell reads them, and what the card says about them.

import { ago, type Tone } from "@/shared";

export type SessionStatus = "new" | "ok" | "expired";

/** A managed browser profile. */
export interface Session {
  id: number;
  name: string;
  host: string;
  status: SessionStatus;
  /** RFC 3339, `null` when it was never checked. */
  verifiedAt: string | null;
  /** A login window is open and waits for the user to press Done. */
  loginInProgress: boolean;
}

/** The browser sessions run on. */
export interface Chrome {
  path: string;
  /** `JOBPILOT_CHROME` / `managed` / `system`. */
  source: string;
}

/** Where the browser download stands (also the payload of `integrations:browser-download`). */
export interface Download {
  active: boolean;
  /** Percent done, when the size is known. */
  percent: number | null;
}

export interface Sessions {
  sessions: Session[];
  chrome: Chrome | null;
  download: Download;
}

/** Quick starts for the "New session" form: (name, login URL). */
export const SUGGESTIONS: readonly { name: string; url: string }[] = [
  { name: "LinkedIn", url: "https://www.linkedin.com/login" },
  { name: "Indeed", url: "https://secure.indeed.com/account/login" },
  { name: "Google", url: "https://accounts.google.com/" },
];

/** The first suggestion fills the form when it opens. */
export const FIRST_SUGGESTION = SUGGESTIONS[0] ?? { name: "", url: "" };

/** `busy` key for operations that are not about one existing session (creating a new one). */
export const NEW_KEY = 0;

/** The destructive actions of a row: the first click asks "Sure?", the second runs it. */
export type Destructive = "signOut" | "delete";

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

/** `host - verified 5 min ago`, or `host - never verified`. */
export function sessionLine(session: Pick<Session, "host" | "verifiedAt">, now: Date): string {
  const verified =
    session.verifiedAt === null ? "never verified" : `verified ${ago(now, new Date(session.verifiedAt))}`;
  return `${session.host} - ${verified}`;
}

/** The browser line of the card. */
export function browserLine(chrome: Chrome | null, download: Download): string {
  if (download.active) {
    return download.percent === null ? "Downloading browser..." : `Downloading browser... ${download.percent}%`;
  }
  return chrome ? `Browser: ${chrome.path} (${chrome.source})` : "Browser: not found";
}

/** The label of the download button. */
export function downloadLabel(chrome: Chrome | null): string {
  return chrome ? "Update browser" : "Download browser";
}

/** The tag at the top of the card. */
export function browserTag(chrome: Chrome | null, download: Download): { tone: Tone; text: string } {
  if (chrome) {
    return { tone: "ok", text: "Chrome found" };
  }
  return download.active ? { tone: "neutral", text: "Downloading" } : { tone: "warn", text: "No browser" };
}
