// The context's port for browser sessions and the managed browser.

import { invoke, listen, notifyChanged } from "@/shared/bridge";

import type { Download, Sessions } from "../domain/sessions";

/** The sessions, the browser they run on and the state of a browser download. */
export function loadSessions(): Promise<Sessions> {
  return invoke<Sessions>("integrations_sessions");
}

/** A session or the browser changed: the card, and the "No Chrome" state of the shell, follow. */
function sessionsChanged(): void {
  notifyChanged("integrations", "health");
}

async function changing<T>(call: Promise<T>): Promise<T> {
  try {
    return await call;
  } finally {
    // Also after a failure: a check that failed may still have changed what is stored.
    sessionsChanged();
  }
}

/** Add a session and open its browser at the login address; resolves with the new session's id. */
export function createSession(name: string, url: string): Promise<number> {
  return changing(invoke<number>("integrations_create_session", { name, url }));
}

/** Open the login window of a session again. */
export function openSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_open_session", { id }));
}

/** The user is done logging in: save the logins, close the window and check the session. */
export function finishLogin(id: number): Promise<void> {
  return changing(invoke<void>("integrations_finish_login", { id }));
}

/** Check in the background whether the session is still logged in. */
export function checkSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_check_session", { id }));
}

/** Forget the logins of a session (the session stays, as new). */
export function signOutSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_sign_out_session", { id }));
}

export function deleteSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_delete_session", { id }));
}

/** Download (or update) the managed browser. Resolves when done; progress arrives through [`onBrowserDownload`]. */
export function downloadBrowser(): Promise<void> {
  return changing(invoke<void>("integrations_download_browser"));
}

/** The percentage of a running download. Returns the unsubscribe. */
export function onBrowserDownload(handler: (download: Download) => void): () => void {
  return listen<Download>("integrations:browser-download", handler);
}
