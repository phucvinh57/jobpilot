// Typed calls for Gmail. The OAuth client secret travels one way: from the form to the shell,
// which keeps it in the OS keyring. Sign-in happens in the system browser, driven by the shell.

import { invoke, listen, notifyChanged } from "./bridge";

import type { Gmail } from "@/types/integrations";

/** Gmail settings and sync status (never a secret). */
export function loadGmail(): Promise<Gmail> {
  return invoke<Gmail>("integrations_gmail");
}

/** Save the OAuth client (an empty secret keeps the saved one) and sign in through the system
 *  browser. Resolves with the account address; rejects when the user cancels or it takes too long. */
export async function connect(clientId: string, clientSecret: string | null): Promise<string> {
  const account = await invoke<string>("integrations_gmail_connect", { clientId, clientSecret });
  notifyChanged("integrations");
  return account;
}

/** Abort a sign-in that waits for the browser. */
export function cancelConnect(): Promise<void> {
  return invoke<void>("integrations_gmail_cancel_connect");
}

/** Check the mailbox now. The outcome (or the error) is saved with the sync: read it with [`loadGmail`]. */
export async function syncNow(): Promise<void> {
  await invoke<void>("integrations_gmail_sync");
  notifyChanged("integrations");
}

/** Revoke the access and forget the account (the OAuth client stays). */
export async function disconnect(): Promise<void> {
  await invoke<void>("integrations_gmail_disconnect");
  notifyChanged("integrations");
}

/** A sync stored mail or changed a status (also in the background): the "last sync" line is stale.
 *  Returns the unsubscribe. */
export function onGmailChanged(handler: () => void): () => void {
  return listen<null>("integrations:gmail-changed", handler);
}
