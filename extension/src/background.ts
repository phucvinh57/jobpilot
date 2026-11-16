// The service worker: wires Chrome's events to the bridge. Every listener is registered synchronously
// here so Chrome can wake the worker for it.

import { ALARM, connect, ensureAlarm, reconnectNow } from "./connection";
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

// The popup saved a port (or cleared it): reconnect at once.
chrome.runtime.onMessage.addListener((message: unknown, sender) => {
  if (sender.id !== chrome.runtime.id) return;
  if (typeof message === "object" && message !== null && (message as { type?: unknown }).type === "reconnect") {
    void reconnectNow();
  }
});

// A wake-up for any other event (or a restart of the worker) also reconnects.
start();
