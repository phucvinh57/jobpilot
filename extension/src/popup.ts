// The toolbar popup: the connection state, the pairing code and the user's Pair, the port, and the
// Incognito hint.

import type { Status } from "./connection";
import { DEFAULT_PORT, parsePort } from "./config";

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function label(status: Status | undefined): string {
  const state = status?.state ?? "disconnected";
  switch (state) {
    case "connecting":
      return "Connecting to the JobPilot app…";
    case "pairing":
      return status?.consented
        ? "Waiting for JobPilot: press Pair there too (Settings, Browser) if it shows the same code"
        : "JobPilot asks to pair with this browser";
    case "connected":
      return "Connected to the JobPilot app";
    case "disconnected":
      return "Not connected: is the JobPilot app running?";
    case "rejected":
      return "The app refused the connection";
    case "impostor":
      return `Another program answered on port ${status?.port || DEFAULT_PORT}, not JobPilot`;
    case "refused":
      return "Pairing refused: the app sent a pairing you did not approve here";
  }
}

function render(status: Status | undefined): void {
  const state = status?.state ?? "disconnected";
  $("state").textContent = label(status);
  $("state").dataset.state = state;
  const pairing = state === "pairing" && !!status?.code;
  $("pairing").hidden = !pairing;
  $("code").textContent = pairing ? (status?.code ?? "") : "";
  $("consent").hidden = !pairing || !!status?.consented;
  $("consent-hint").hidden = !pairing || !!status?.consented;
  $("impostor").hidden = state !== "impostor";
  const reason = $("reason");
  reason.hidden = !status?.reason || state === "connected" || state === "pairing";
  reason.textContent = status?.reason ? `Last refusal: ${status.reason}` : "";
}

async function init(): Promise<void> {
  let current: Status | undefined;
  const show = (status: Status | undefined) => {
    current = status;
    render(status);
  };
  const { status } = await chrome.storage.session.get("status");
  show(status as Status | undefined);
  chrome.storage.onChanged.addListener((changes, area) => {
    if (area === "session" && changes.status) show(changes.status.newValue as Status | undefined);
  });

  // The user's Pair, for the code on screen (the service worker checks it is the current one).
  $("consent").addEventListener("click", () => {
    if (current?.state === "pairing" && current.code) {
      void chrome.runtime.sendMessage({ type: "consent", code: current.code });
    }
  });
  $("pair-again").addEventListener("click", () => {
    void chrome.runtime.sendMessage({ type: "pairAgain" });
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
