// Typed calls for Settings > Browser: the JobPilot extension in the user's Chrome (pairing) and the
// sites they are signed in to (sessions).

import { invoke, listen, notifyChanged } from "./bridge";

import type { BrowserSection } from "@/types/integrations";

/** The connection to the user's browser and the sites they are signed in to. */
export function loadBrowser(): Promise<BrowserSection> {
  return invoke<BrowserSection>("integrations_browser");
}

/** The connection or a site changed: the section, and the browser state of the shell, follow. */
function browserChanged(): void {
  notifyChanged("integrations", "health");
}

async function changing<T>(call: Promise<T>): Promise<T> {
  try {
    return await call;
  } finally {
    // Also after a failure: a check that failed may still have changed what is stored.
    browserChanged();
  }
}

/** The user checked the code: pair the browser that waits. */
export function approvePairing(): Promise<void> {
  return changing(invoke<void>("integrations_approve_pairing"));
}

/** Refuse the browser that asks to pair. */
export function declinePairing(): Promise<void> {
  return changing(invoke<void>("integrations_decline_pairing"));
}

/** Forget the paired browser: it has to pair again. */
export function forgetBrowser(): Promise<void> {
  return changing(invoke<void>("integrations_forget_browser"));
}

/** Add a site and open its sign-in page in a tab of the user's browser; resolves with its id. */
export function createSession(name: string, url: string): Promise<number> {
  return changing(invoke<number>("integrations_create_session", { name, url }));
}

/** Open the sign-in tab of a site again (or bring it to the front). */
export function openSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_open_session", { id }));
}

/** The user is done signing in: close the sign-in tab and check the site. */
export function finishLogin(id: number): Promise<void> {
  return changing(invoke<void>("integrations_finish_login", { id }));
}

/** Check in a background tab whether the user is still signed in to the site. */
export function checkSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_check_session", { id }));
}

/** Remove a site from the list (the user stays signed in to it in their browser). */
export function deleteSession(id: number): Promise<void> {
  return changing(invoke<void>("integrations_delete_session", { id }));
}

/** The browser connection changed (connected, a pairing code to approve, paired, gone). Returns the
 *  unsubscribe. */
export function onBrowserStatus(handler: () => void): () => void {
  return listen<null>("integrations:browser-status", () => handler());
}
