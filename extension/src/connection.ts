// The WebSocket client to the app (PROTOCOL.md, "Identity and transport", "Handshake and pairing").
//
// The extension does not trust whatever answers on the port. The token (in `chrome.storage.local.token`)
// is never sent after pairing: the app proves it holds it (`challenge`) before the extension proves it
// does (`auth`), and a token is only taken (`paired`) after the user pressed Pair in the popup for the
// code of this connection. Tokens, nonces and proofs are never logged.

import { extProof, newNonce, verifyAppProof } from "./auth";
import { autoConsent, bridgeUrl, resolvePort } from "./config";
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

/**
 * `impostor`: something answered on the port but could not prove it holds the token (not JobPilot).
 * `refused`: the other side sent a token for a pairing the user did not approve in the popup.
 */
export type ConnectionState =
  | "connecting"
  | "pairing"
  | "connected"
  | "disconnected"
  | "rejected"
  | "impostor"
  | "refused";

/**
 * What the popup shows (in `chrome.storage.session.status`). Never a token. `reason` is the app's last
 * rejection, kept until it welcomes or pairs again. `consented`: the user pressed Pair for `code`.
 */
export type Status = { state: ConnectionState; port: number; code?: string; consented?: boolean; reason?: string };

/** States that stay on screen while the extension tries again (until the app is proven or pairs). */
const STICKY: ConnectionState[] = ["impostor", "refused"];

let socket: WebSocket | null = null;
let opening = false;
let attempt = 0;
let retryTimer: ReturnType<typeof setTimeout> | undefined;
let pingTimer: ReturnType<typeof setInterval> | undefined;
let status: Status = { state: "disconnected", port: 0 };
// Frames are handled in order (a `welcome` is in effect before the next call), calls do not wait.
let inbox: Promise<void> = Promise.resolve();

// The handshake of the current socket (reset by `teardown`).
/** The nonce sent in `hello` (the app's proof must be for it). */
let nonce: string | null = null;
/** The app proved it holds the token, and the extension answered with `auth`. */
let verified = false;
/** A token was taken in this connection (after the user's consent). */
let freshlyPaired = false;
let welcomed = false;
/** The code of the app's `pair`, and the code the user said Pair to. */
let pairCode: string | null = null;
let consentedCode: string | null = null;

type StatusExtra = { code?: string; consented?: boolean; reason?: string; port?: number; force?: boolean };

function setStatus(state: ConnectionState, extra: StatusExtra = {}): void {
  // An impostor or refusal stays on screen while reconnecting, until something else happens.
  const kept = !extra.force && STICKY.includes(status.state) && (state === "connecting" || state === "disconnected");
  const next = kept ? status.state : state;
  const reason = next === "connected" || next === "pairing" ? undefined : (extra.reason ?? status.reason);
  status = { state: next, port: extra.port ?? status.port, code: extra.code, consented: extra.consented, reason };
  chrome.storage.session.set({ status }).catch(() => undefined);
}

function send(ws: WebSocket, message: unknown): boolean {
  if (ws.readyState !== WebSocket.OPEN) return false;
  ws.send(JSON.stringify(message));
  return true;
}

async function storedToken(): Promise<string | null> {
  const { token } = await chrome.storage.local.get("token");
  return typeof token === "string" && token !== "" ? token : null;
}

async function hello(ws: WebSocket): Promise<void> {
  const paired = (await storedToken()) !== null;
  const incognitoAllowed = await chrome.extension.isAllowedIncognitoAccess();
  if (socket !== ws) return;
  nonce = newNonce();
  send(ws, {
    type: "hello",
    protocol: PROTOCOL_VERSION,
    extension: EXTENSION_VERSION,
    browser: navigator.userAgent,
    incognitoAllowed,
    paired,
    nonce,
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

/** Close `ws` without another word, show `state` and try again later (backoff). */
function abandon(ws: WebSocket, state: "impostor" | "refused", why: string): void {
  if (socket !== ws) return;
  console.warn(`jobpilot: ${why}; disconnected`);
  teardown();
  setStatus(state);
  scheduleRetry();
}

/** The app's `challenge`: check its proof, then prove the extension holds the token too. */
async function onChallenge(ws: WebSocket, proof: string, appNonce: string): Promise<void> {
  const token = await storedToken();
  const sentNonce = nonce;
  if (token === null || sentNonce === null || verified || freshlyPaired) {
    abandon(ws, "impostor", "an unexpected challenge");
    return;
  }
  if (!(await verifyAppProof(token, sentNonce, proof))) {
    // Not the app this browser was paired with: say nothing more, keep the token.
    abandon(ws, "impostor", "the program on the port could not prove it is JobPilot");
    return;
  }
  if (socket !== ws) return;
  verified = true;
  send(ws, { type: "auth", proof: await extProof(token, appNonce) });
}

async function onPair(ws: WebSocket, code: string): Promise<void> {
  pairCode = code;
  consentedCode = null;
  setStatus("pairing", { code });
  console.info("jobpilot: pairing requested");
  if ((await autoConsent()) && socket === ws) consent(code);
}

async function onPaired(ws: WebSocket, token: string): Promise<void> {
  if (pairCode === null || consentedCode !== pairCode) {
    // The user did not press Pair for this code: the token is dropped.
    abandon(ws, "refused", "a pairing the user did not approve");
    return;
  }
  await chrome.storage.local.set({ token });
  if (socket !== ws) return;
  freshlyPaired = true;
  console.info("jobpilot: paired");
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
    case "challenge":
      await onChallenge(ws, message.proof, message.nonce);
      break;
    case "welcome":
      // Only an app that proved the token, or that just paired with the user's consent.
      if (!verified && !freshlyPaired) {
        abandon(ws, "impostor", "a welcome without a proof");
        break;
      }
      welcomed = true;
      attempt = 0;
      setSink((m) => send(ws, m));
      setStatus("connected", { force: true });
      console.info("jobpilot: connected to the app");
      break;
    case "pair":
      await onPair(ws, message.code);
      break;
    case "paired":
      await onPaired(ws, message.token);
      break;
    case "rejected":
      setStatus("rejected", { reason: message.reason || "rejected", force: true });
      console.warn("jobpilot: rejected by the app:", message.reason);
      break;
    case "forget":
      // Only from the app this browser is connected to (an unproven peer cannot un-pair it).
      if (!welcomed) break;
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
  verified = false;
  freshlyPaired = false;
  nonce = null;
  pairCode = null;
  consentedCode = null;
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
  setStatus("connecting", { force: true });
  await connect();
}

/**
 * The user pressed Pair in the popup for `code`: tell the app (it sends the token once the user also
 * approved there). `false` when no pairing with that code is going on.
 */
export function consent(code: string): boolean {
  const ws = socket;
  if (!ws || pairCode === null || code !== pairCode) return false;
  consentedCode = code;
  send(ws, { type: "consent", code });
  setStatus("pairing", { code, consented: true });
  console.info("jobpilot: pairing approved in the extension");
  return true;
}

/** The user asked to pair again (after JobPilot was paired with another browser): drop the token. */
export async function pairAgain(): Promise<void> {
  await chrome.storage.local.remove("token");
  await reconnectNow();
}

/** The 1-minute tick (and startup): connect when nothing is open or opening. */
export async function ensureAlarm(): Promise<void> {
  if (!(await chrome.alarms.get(ALARM))) await chrome.alarms.create(ALARM, { periodInMinutes: 1 });
}
