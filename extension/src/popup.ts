// The toolbar popup: the connection state, the pairing code, the port, and the Incognito hint.

import type { Status } from "./connection";
import { DEFAULT_PORT, parsePort } from "./config";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const LABELS: Record<Status["state"], string> = {
  connecting: "Connecting to the JobPilot app…",
  pairing: "Pairing: approve this code in JobPilot (Settings)",
  connected: "Connected to the JobPilot app",
  disconnected: "Not connected: is the JobPilot app running?",
  rejected: "The app refused the connection",
};

function render(status: Status | undefined): void {
  const state = status?.state ?? "disconnected";
  $("state").textContent = LABELS[state];
  $("state").dataset.state = state;
  const code = $("code");
  code.hidden = state !== "pairing" || !status?.code;
  code.textContent = status?.code ?? "";
  const reason = $("reason");
  reason.hidden = !status?.reason || state === "connected" || state === "pairing";
  reason.textContent = status?.reason ? `Last refusal: ${status.reason}` : "";
}

async function init(): Promise<void> {
  const { status } = await chrome.storage.session.get("status");
  render(status as Status | undefined);
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "session" && changes.status) render(changes.status.newValue as Status | undefined);
  });

  const { port } = await chrome.storage.local.get("port");
  const input = $<HTMLInputElement>("port");
  input.placeholder = String(DEFAULT_PORT);
  input.value = parsePort(port)?.toString() ?? "";

  $<HTMLFormElement>("port-form").addEventListener("submit", (event) => {
    event.preventDefault();
    const message = $("port-message");
    if (input.value.trim() === "") {
      void chrome.storage.local.remove("port").then(() => chrome.runtime.sendMessage({ type: "reconnect" }));
      message.textContent = "Using the default port.";
      return;
    }
    const next = parsePort(input.value);
    if (next === null) {
      message.textContent = "A port is a number from 1 to 65535.";
      return;
    }
    void chrome.storage.local.set({ port: next }).then(() => chrome.runtime.sendMessage({ type: "reconnect" }));
    message.textContent = `Saved: port ${next}.`;
  });

  $("incognito").hidden = await chrome.extension.isAllowedIncognitoAccess();
}

void init();
