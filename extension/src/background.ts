// The service worker: wires Chrome's events to the bridge. Every listener is registered synchronously
// here so Chrome can wake the worker for it.

import { ALARM, connect, consent, ensureAlarm, pairAgain, reconnectNow } from "./connection";
import { registerTabEvents } from "./events";

registerTabEvents();

const start = () => {
  void ensureAlarm();
  void connect();
};

chrome.runtime.onStartup.addListener(start);
chrome.runtime.onInstalled.addListener(start);
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === ALARM) void connect();
});

// The popup (the extension's own page; no web page can message the extension): a port was saved
// (reconnect), the user pressed Pair for a code, or asked to pair again.
chrome.runtime.onMessage.addListener((message: unknown, sender) => {
  if (sender.id !== chrome.runtime.id || typeof message !== "object" || message === null) return;
  const { type, code } = message as { type?: unknown; code?: unknown };
  if (type === "reconnect") void reconnectNow();
  // The user's answers come from the popup page only.
  if (!sender.url?.startsWith(chrome.runtime.getURL("popup.html"))) return;
  if (type === "consent" && typeof code === "string") consent(code);
  if (type === "pairAgain") void pairAgain();
});

// A wake-up for any other event (or a restart of the worker) also reconnects.
start();
