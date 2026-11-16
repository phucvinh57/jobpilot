// Chrome's events about owned tabs, turned into protocol events (PROTOCOL.md, "Events"). Listeners are
// registered synchronously at the top level of the service worker so Chrome can wake it for them.

import { dropPaused, onAttachedToTarget, onRequestPaused } from "./guard";
import { forgetTab } from "./methods";
import { emit } from "./outbox";
import { forgetWindow, isOwned } from "./owned";
import type { Params } from "./protocol";

async function onDebuggerEvent(source: chrome.debugger.DebuggerSession, method: string, params?: Params) {
  const tab = source.tabId;
  if (tab === undefined || !(await isOwned(tab))) return;
  const p = params ?? {};
  switch (method) {
    case "Fetch.requestPaused":
      onRequestPaused(tab, source.sessionId, p);
      break;
    case "Target.attachedToTarget":
      await onAttachedToTarget(tab, p);
      break;
    case "Target.detachedFromTarget":
      if (typeof p.sessionId === "string") dropPaused(tab, p.sessionId);
      break;
    case "Page.javascriptDialogOpening":
      if (!source.sessionId) emit("dialog", { tab, kind: String(p.type ?? ""), message: String(p.message ?? "") });
      break;
  }
}

export function registerTabEvents(): void {
  chrome.tabs.onRemoved.addListener((tab) => {
    void forgetTab(tab);
  });
  chrome.windows.onRemoved.addListener((windowId) => {
    void forgetWindow(windowId);
  });
  chrome.debugger.onDetach.addListener((source, reason) => {
    const tab = source.tabId;
    if (tab === undefined) return;
    void isOwned(tab).then((owned) => {
      if (!owned) return;
      dropPaused(tab);
      console.info("jobpilot: debugger detached", tab, reason);
      emit("detached", { tab, reason });
    });
  });
  chrome.debugger.onEvent.addListener((source, method, params) => {
    void onDebuggerEvent(source, method, params as Params | undefined).catch((error) =>
      console.warn("jobpilot: debugger event failed", method, error),
    );
  });
}
