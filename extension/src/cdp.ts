// chrome.debugger, with Chrome's failures as `cdp` errors.

import { BridgeError, type Params } from "./protocol";

export const DEBUGGER_VERSION = "1.3";

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));

/** Chrome's text when the tab has no debugger attached (the user cancelled the bar, or a restart). */
export const isNotAttached = (error: unknown): boolean =>
  /not attached|detached while handling/i.test(error instanceof BridgeError ? error.message : message(error));

export async function attach(tab: number): Promise<void> {
  try {
    await chrome.debugger.attach({ tabId: tab }, DEBUGGER_VERSION);
  } catch (error) {
    throw new BridgeError("cdp", message(error));
  }
}

/** Send one CDP command to the tab, or to a child session of it. */
export async function command(tab: number, method: string, params: Params = {}, session?: string): Promise<Params> {
  const target: chrome.debugger.DebuggerSession = session ? { tabId: tab, sessionId: session } : { tabId: tab };
  try {
    const result = await chrome.debugger.sendCommand(target, method, params);
    return (result ?? {}) as Params;
  } catch (error) {
    throw new BridgeError("cdp", message(error));
  }
}
