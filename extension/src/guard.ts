// The guard of a discovery tab (PROTOCOL.md, "The guard"): a declarativeNetRequest session rule blocks
// literal private hosts in the extension; every other request is paused (Fetch) and the app decides.
// The extension never continues a request on its own: it only fails the paused ones when the app is gone.

import { PRIVATE_HOST_REGEXES } from "./allowlist";
import { command } from "./cdp";
import { emit, linked } from "./outbox";
import { guardedTabs, isGuarded, markGuarded } from "./owned";
import type { Params } from "./protocol";

const FETCH_PATTERNS = { patterns: [{ urlPattern: "*", requestStage: "Request" }] };
const AUTO_ATTACH = { autoAttach: true, waitForDebuggerOnStart: true, flatten: true };

// Every resource type: a rule without `resourceTypes` would skip main frames.
const RESOURCE_TYPES = [
  "main_frame",
  "sub_frame",
  "stylesheet",
  "script",
  "image",
  "font",
  "object",
  "xmlhttprequest",
  "ping",
  "csp_report",
  "media",
  "websocket",
  "webtransport",
  "webbundle",
  "other",
] as const;

const resourceTypes = (): chrome.declarativeNetRequest.ResourceType[] => {
  const known = chrome.declarativeNetRequest.ResourceType;
  return (known ? Object.values(known) : [...RESOURCE_TYPES]) as chrome.declarativeNetRequest.ResourceType[];
};

// The session rules of the guard: one per kind of private host (Chrome caps a compiled regex at 2 KB),
// each scoped to the guarded tabs (`tabIds`); a tab leaves them when it closes.
export const GUARD_RULE_IDS = PRIVATE_HOST_REGEXES.map((_, i) => i + 1);

export function guardRules(tabs: number[]): chrome.declarativeNetRequest.Rule[] {
  if (tabs.length === 0) return [];
  return PRIVATE_HOST_REGEXES.map((regexFilter, i) => ({
    id: i + 1,
    priority: 1,
    action: { type: "block" as chrome.declarativeNetRequest.RuleActionType },
    condition: { regexFilter, isUrlFilterCaseSensitive: true, tabIds: [...tabs], resourceTypes: resourceTypes() },
  }));
}

let rulesQueue: Promise<unknown> = Promise.resolve();

/** Scope the guard rules to the tabs guarded now (none: no rules). */
export function syncGuardRules(): Promise<void> {
  const next = rulesQueue.then(async () => {
    const tabs = await guardedTabs();
    await chrome.declarativeNetRequest.updateSessionRules({
      removeRuleIds: GUARD_RULE_IDS,
      addRules: guardRules(tabs),
    });
  });
  rulesQueue = next.catch(() => undefined);
  return next;
}

/** Pause every request of the tab and of the targets it starts (iframes, workers). */
export async function intercept(tab: number): Promise<void> {
  await command(tab, "Fetch.enable", FETCH_PATTERNS);
  await command(tab, "Target.setAutoAttach", AUTO_ATTACH);
}

/** Install the guard on a fresh tab, before its first navigation. */
export async function installGuard(tab: number): Promise<void> {
  await markGuarded(tab);
  await syncGuardRules();
  await intercept(tab);
}

/** A guarded tab is gone: it leaves the rules. */
export async function removeGuardRule(): Promise<void> {
  try {
    await syncGuardRules();
  } catch (error) {
    console.warn("jobpilot: could not update the guard rules", error);
  }
}

// Requests paused and forwarded to the app, so they can be failed when the socket drops.
type Paused = { tab: number; session?: string; requestId: string };
const paused = new Map<string, Paused>();
const key = (tab: number, session: string | undefined, requestId: string) => `${tab}|${session ?? ""}|${requestId}`;

const fail = (p: Paused) =>
  command(p.tab, "Fetch.failRequest", { requestId: p.requestId, errorReason: "BlockedByClient" }, p.session).catch(
    () => undefined,
  );

/** `Target.attachedToTarget` on a guarded tab: guard the child before it runs. */
export async function onAttachedToTarget(tab: number, params: Params): Promise<void> {
  const session = typeof params.sessionId === "string" ? params.sessionId : undefined;
  if (!session) return;
  if (await isGuarded(tab)) {
    try {
      await command(tab, "Fetch.enable", FETCH_PATTERNS, session);
    } catch (error) {
      console.warn("jobpilot: Fetch.enable failed on a child target", tab, error);
    }
    // Nested out-of-process iframes attach through their parent; a worker has no Target domain.
    await command(tab, "Target.setAutoAttach", AUTO_ATTACH, session).catch(() => undefined);
  }
  await command(tab, "Runtime.runIfWaitingForDebugger", {}, session).catch(() => undefined);
}

/** `Fetch.requestPaused`: the app answers it; without an app it fails at once. */
export function onRequestPaused(tab: number, session: string | undefined, params: Params): void {
  const requestId = typeof params.requestId === "string" ? params.requestId : undefined;
  if (!requestId) return;
  const request = params.request as { url?: unknown } | undefined;
  const url = typeof request?.url === "string" ? request.url : "";
  const p: Paused = { tab, session, requestId };
  if (!linked()) {
    void fail(p);
    return;
  }
  paused.set(key(tab, session, requestId), p);
  const sent = emit("requestPaused", session ? { tab, session, requestId, url } : { tab, requestId, url });
  if (!sent) {
    paused.delete(key(tab, session, requestId));
    void fail(p);
  }
}

/** The app answered a paused request (`Fetch.continueRequest` / `Fetch.failRequest`). */
export function resolved(tab: number, session: string | undefined, params: Params): void {
  if (typeof params.requestId === "string") paused.delete(key(tab, session, params.requestId));
}

/** Forget the paused requests of a tab (closed, detached) or of one of its child sessions. */
export function dropPaused(tab: number, session?: string): void {
  for (const [k, p] of paused) {
    if (p.tab === tab && (session === undefined || p.session === session)) paused.delete(k);
  }
}

/** The socket dropped: every request still waiting for the app fails. */
export async function failAllPaused(): Promise<void> {
  const all = [...paused.values()];
  paused.clear();
  await Promise.all(all.map(fail));
}

export const pausedCount = (): number => paused.size;
