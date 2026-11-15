// AI providers, Gmail and browser sessions as the shell reads them (`integrations_*` commands).
// An API key is never part of what is read: only "a key is saved" comes back.

export interface SyncReport {
  fetched: number;
  updated: number;
  needsReview: number;
  error: string | null;
}

/** Gmail settings and sync status. The client secret and the refresh token stay in the OS keyring:
 *  only whether a client secret is saved comes back. */
export interface Gmail {
  clientId: string;
  hasClientSecret: boolean;
  /** The connected account's address. */
  account: string | null;
  /** RFC 3339. */
  lastSync: string | null;
  lastReport: SyncReport | null;
  /** Minutes between polls while the app is open. */
  pollMinutes: number;
}

export type ProviderKind = "openai" | "anthropic";

export interface Provider {
  id: number;
  name: string;
  kind: ProviderKind;
  kindLabel: string;
  baseUrl: string;
  model: string;
  /** The endpoint lacks `json_schema` support: ask for a JSON object instead. */
  jsonObject: boolean;
  isDefault: boolean;
}

/** What a kind of provider starts with in the form. */
export interface KindDefaults {
  kind: ProviderKind;
  label: string;
  name: string;
  baseUrl: string;
  model: string;
}

export interface Providers {
  providers: Provider[];
  kinds: KindDefaults[];
}

/** The form, sent to save, test or list models. `apiKey` is typed text: empty keeps the saved key. */
export interface ProviderInput {
  id: number;
  kind: ProviderKind;
  name: string;
  baseUrl: string;
  model: string;
  jsonObject: boolean;
  apiKey: string | null;
}

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
