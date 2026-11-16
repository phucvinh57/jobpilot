// The WebSocket client to the app (PROTOCOL.md, "Identity and transport", "Handshake and pairing").
// The token is stored in `chrome.storage.local.token` and never logged.

import { bridgeUrl, resolvePort } from "./config";
import { failAllPaused } from "./guard";
import { closeAll, handleCall } from "./methods";
import { setSink } from "./outbox";
import {
  backoffMs,
  type Call,
  EXTENSION_VERSION,
  errorReply,
  parseIncoming,
  PROTOCOL_VERSION,
  resultReply,
} from "./protocol";

const PING_MS = 20_000;
export const ALARM = "jobpilot-reconnect";

export type ConnectionState = "connecting" | "pairing" | "connected" | "disconnected" | "rejected";

/**
 * What the popup shows (in `chrome.storage.session.status`). Never a token. `reason` is the app's last
 * rejection, kept until it welcomes or pairs again.
 */
export type Status = { state: ConnectionState; port: number; code?: string; reason?: string };

let socket: WebSocket | null = null;
let opening = false;
let welcomed = false;
let attempt = 0;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let pingTimer: ReturnType<typeof setInterval> | undefined;
let status: Status = { state: "disconnected", port: 0 };
// Frames are handled in order (a `welcome` is in effect before the next call), calls do not wait.
let inbox: Promise<void> = Promise.resolve();

function setStatus(state: ConnectionState, extra: { code?: string; reason?: string; port?: number } = {}): void {
  const reason = state === "connected" || state === "pairing" ? undefined : (extra.reason ?? status.reason);
  status = { state, port: extra.port ?? status.port, code: extra.code, reason };
  chrome.storage.session.set({ status }).catch(() => undefined);
}

function send(ws: WebSocket, message: unknown): boolean {
  if (ws.readyState !== WebSocket.OPEN) return false;
  ws.send(JSON.stringify(message));
  return true;
}

async function hello(ws: WebSocket): Promise<void> {
  const { token } = await chrome.storage.local.get("token");
  send(ws, {
    type: "hello",
    protocol: PROTOCOL_VERSION,
    extension: EXTENSION_VERSION,
    browser: navigator.userAgent,
    incognitoAllowed: await chrome.extension.isAllowedIncognitoAccess(),
    token: typeof token === "string" && token !== "" ? token : null,
  });
}

async function answer(ws: WebSocket, call: Call): Promise<void> {
  if (!welcomed) {
    send(ws, errorReply(call.id, new Error("not paired")));
    return;
  }
  try {
    send(ws, resultReply(call.id, (await handleCall(call)) ?? {}));
  } catch (error) {
    send(ws, errorReply(call.id, error));
  }
}

async function onMessage(ws: WebSocket, data: unknown): Promise<void> {
  if (socket !== ws || typeof data !== "string") return;
  const message = parseIncoming(data);
  if (!message) return;
  switch (message.type) {
    case "call":
      // Calls run concurrently (a guarded `tab.open` waits for the app to answer paused requests).
      void answer(ws, message);
      break;
    case "welcome":
      welcomed = true;
      attempt = 0;
      setSink((m) => send(ws, m));
      setStatus("connected");
      console.info("jobpilot: connected to the app");
      break;
    case "pair":
      setStatus("pairing", { code: message.code });
      console.info("jobpilot: pairing requested");
      break;
    case "paired":
      await chrome.storage.local.set({ token: message.token });
      console.info("jobpilot: paired");
      break;
    case "rejected":
      setStatus("rejected", { reason: message.reason || "rejected" });
      console.warn("jobpilot: rejected by the app:", message.reason);
      if (message.reason === "declined") await chrome.storage.local.remove("token");
      break;
    case "forget":
      console.info("jobpilot: the app forgot this browser");
      await chrome.storage.local.remove("token");
      await closeAll();
      await reconnectNow();
      break;
    case "pong":
      break;
  }
}

/** Drop the current socket (if any): no more events, paused guard requests fail. */
function teardown(): void {
  const ws = socket;
  socket = null;
  welcomed = false;
  setSink(null);
  clearInterval(pingTimer);
  pingTimer = undefined;
  void failAllPaused();
  if (ws && ws.readyState !== WebSocket.CLOSED && ws.readyState !== WebSocket.CLOSING) ws.close();
}

function scheduleRetry(): void {
  clearTimeout(retryTimer);
  const delay = backoffMs(attempt);
  attempt += 1;
  retryTimer = setTimeout(() => void connect(), delay);
}

/** Open the socket unless one is open or opening. */
export async function connect(): Promise<void> {
  if (socket || opening) return;
  opening = true;
  clearTimeout(retryTimer);
  try {
    const port = await resolvePort();
    setStatus("connecting", { port });
    const ws = new WebSocket(bridgeUrl(port));
    socket = ws;
    ws.onopen = () => {
      if (socket !== ws) return;
      pingTimer = setInterval(() => send(ws, { type: "ping" }), PING_MS);
      void hello(ws).catch((error) => console.warn("jobpilot: hello failed", error));
    };
    ws.onmessage = (event) => {
      inbox = inbox
        .then(() => onMessage(ws, event.data))
        .catch((error) => console.warn("jobpilot: message failed", error));
    };
    ws.onclose = () => {
      if (socket !== ws) return;
      teardown();
      setStatus("disconnected");
      scheduleRetry();
    };
    // An error is always followed by `close`.
    ws.onerror = () => undefined;
  } catch (error) {
    console.warn("jobpilot: could not connect", error);
    socket = null;
    scheduleRetry();
  } finally {
    opening = false;
  }
}

/** Reconnect at once (port changed, forgotten, the popup asked): the backoff starts again. */
export async function reconnectNow(): Promise<void> {
  teardown();
  attempt = 0;
  setStatus("connecting");
  await connect();
}

/** The 1-minute tick (and startup): connect when nothing is open or opening. */
export async function ensureAlarm(): Promise<void> {
  if (!(await chrome.alarms.get(ALARM))) await chrome.alarms.create(ALARM, { periodInMinutes: 1 });
}
