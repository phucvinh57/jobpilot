// The calls the app may make (PROTOCOL.md, "Calls"). Every tab argument must be a tab this extension
// opened; nothing here lists, reads or touches any other tab of the user.

import { isAllowedMethod, isAllowedUrl } from "./allowlist";
import { dropPaused, installGuard, removeGuardRule, resolved } from "./guard";
import { emit } from "./outbox";
import {
  addOwned,
  isOwned,
  jobPilotWindow,
  ownedTabs,
  removeOwned,
  setJobPilotWindow,
  type WindowKind,
  wasClosed,
} from "./owned";
import {
  BridgeError,
  boolParam,
  type Call,
  objectParam,
  optionalStringParam,
  type Params,
  stringParam,
  tabParam,
} from "./protocol";
import { prepare, send } from "./session";

const GROUP_TITLE = "JobPilot";

async function ownedTab(params: Params): Promise<number> {
  const tab = tabParam(params);
  if (!(await isOwned(tab))) throw new BridgeError("not_owned", `tab ${tab} was not opened by JobPilot`);
  return tab;
}

function urlParam(params: Params): string {
  const url = stringParam(params, "url");
  if (!isAllowedUrl(url)) throw new BridgeError("bad_params", "`url` must be http:, https: or about:blank");
  return url;
}

/** `Page.navigate`; Chrome's `errorText` (blocked, DNS, ...) fails the call with code `cdp`. */
async function navigate(tab: number, url: string): Promise<void> {
  const result = await send(tab, "Page.navigate", { url });
  if (typeof result.errorText === "string" && result.errorText !== "") throw new BridgeError("cdp", result.errorText);
}

// One window of each kind; concurrent opens wait for the same window.
const windowLocks: Partial<Record<WindowKind, Promise<unknown>>> = {};

async function existingWindow(kind: WindowKind): Promise<number | undefined> {
  const id = await jobPilotWindow(kind);
  if (id === undefined) return undefined;
  try {
    const win = await chrome.windows.get(id);
    if (win.incognito === (kind === "private")) return id;
  } catch {
    // closed meanwhile
  }
  await setJobPilotWindow(kind, undefined);
  return undefined;
}

/**
 * The JobPilot window of that kind, created when there is none. A new window comes with its own
 * `about:blank` tab, which the extension created: it is returned so the caller uses it as the new tab.
 */
function ensureWindow(kind: WindowKind, focused: boolean): Promise<{ windowId: number; blankTab?: number }> {
  const run = async () => {
    const existing = await existingWindow(kind);
    if (existing !== undefined) return { windowId: existing };
    const win = await chrome.windows.create({
      url: "about:blank",
      incognito: kind === "private",
      focused,
      width: 1280,
      height: 800,
    });
    if (win?.id === undefined) throw new BridgeError("internal", "Chrome did not create the window");
    await setJobPilotWindow(kind, win.id);
    return { windowId: win.id, blankTab: win.tabs?.[0]?.id };
  };
  const next = (windowLocks[kind] ?? Promise.resolve()).catch(() => undefined).then(run);
  windowLocks[kind] = next;
  return next;
}

/** Put the tab in the window's grey "JobPilot" group (best effort: a group is only a label). */
async function group(tab: number, windowId: number): Promise<void> {
  try {
    const [existing] = await chrome.tabGroups.query({ windowId, title: GROUP_TITLE });
    const groupId = existing
      ? await chrome.tabs.group({ tabIds: [tab], groupId: existing.id })
      : await chrome.tabs.group({ tabIds: [tab], createProperties: { windowId } });
    await chrome.tabGroups.update(groupId, { title: GROUP_TITLE, color: "grey" });
  } catch (error) {
    console.warn("jobpilot: could not group the tab", tab, error);
  }
}

async function focusTab(tab: number): Promise<void> {
  const info = await chrome.tabs.get(tab);
  await chrome.windows.update(info.windowId, { focused: true, state: "normal" });
  await chrome.tabs.update(tab, { active: true });
}

async function tabOpen(params: Params) {
  const url = urlParam(params);
  const isPrivate = boolParam(params, "private");
  const focus = boolParam(params, "focus");
  const guard = boolParam(params, "guard");
  if (isPrivate && !(await chrome.extension.isAllowedIncognitoAccess())) {
    throw new BridgeError("incognito_not_allowed", "JobPilot is not allowed in Incognito (chrome://extensions)");
  }
  const kind: WindowKind = isPrivate ? "private" : "normal";
  const { windowId, blankTab } = await ensureWindow(kind, focus);
  let tab = blankTab;
  if (tab === undefined) {
    const created = await chrome.tabs.create({ windowId, url: "about:blank", active: true });
    tab = created.id;
  }
  if (tab === undefined) throw new BridgeError("internal", "Chrome did not create the tab");
  await addOwned(tab);
  try {
    await group(tab, windowId);
    await prepare(tab);
    if (guard) await installGuard(tab);
    if (url !== "about:blank") await navigate(tab, url);
    if (focus) await focusTab(tab);
  } catch (error) {
    // The app never learns this tab: do not leave it behind.
    await closeTab(tab).catch(() => undefined);
    throw error;
  }
  console.info(`jobpilot: opened tab ${tab} (${isPrivate ? "private" : "normal"}${guard ? ", guarded" : ""})`);
  return { tab, window: windowId };
}

/** Forget a tab that is gone: its owned entry, its guard rule; tell the app once. */
export async function forgetTab(tab: number): Promise<void> {
  const { owned, guarded } = await removeOwned(tab);
  if (!owned) return;
  dropPaused(tab);
  if (guarded) await removeGuardRule();
  emit("tabClosed", { tab });
}

async function closeTab(tab: number): Promise<void> {
  try {
    await chrome.tabs.remove(tab);
  } catch {
    // already gone (closed while the service worker slept): forget it here
  }
  await forgetTab(tab);
}

export async function closeAll(): Promise<void> {
  await Promise.all((await ownedTabs()).map(closeTab));
}

async function cookieDomains(params: Params) {
  const domain = stringParam(params, "domain").trim();
  if (domain === "") throw new BridgeError("bad_params", "`domain` must not be empty");
  const isPrivate = boolParam(params, "private");
  let storeId: string | undefined;
  if (isPrivate) {
    if (!(await chrome.extension.isAllowedIncognitoAccess())) {
      throw new BridgeError("incognito_not_allowed", "JobPilot is not allowed in Incognito (chrome://extensions)");
    }
    storeId = await incognitoStore();
    if (storeId === undefined) return { domains: [] };
  }
  const cookies = await chrome.cookies.getAll(storeId ? { domain, storeId } : { domain });
  return { domains: distinctDomains(cookies.map((c) => c.domain)) };
}

/** The cookie store of the JobPilot incognito window (none while it is closed). */
async function incognitoStore(): Promise<string | undefined> {
  const windowId = await jobPilotWindow("private");
  if (windowId === undefined) return undefined;
  const mine = new Set<number>();
  for (const tab of await ownedTabs()) {
    try {
      if ((await chrome.tabs.get(tab)).windowId === windowId) mine.add(tab);
    } catch {
      // gone
    }
  }
  const stores = await chrome.cookies.getAllCookieStores();
  return stores.find((store) => store.tabIds.some((id) => mine.has(id)))?.id;
}

/** Distinct cookie domains, leading dot removed, sorted. Names and values never leave the browser. */
export function distinctDomains(domains: string[]): string[] {
  return [...new Set(domains.map((d) => d.replace(/^\./, "")).filter((d) => d !== ""))].sort();
}

const methods: Record<string, (params: Params) => Promise<unknown>> = {
  "tab.open": tabOpen,
  "tab.navigate": async (params) => {
    const tab = await ownedTab(params);
    await navigate(tab, urlParam(params));
    return {};
  },
  "tab.info": async (params) => {
    const tab = await ownedTab(params);
    let info: chrome.tabs.Tab;
    try {
      info = await chrome.tabs.get(tab);
    } catch {
      await forgetTab(tab);
      throw new BridgeError("not_owned", `tab ${tab} is closed`);
    }
    return { url: info.url ?? info.pendingUrl ?? "", title: info.title ?? "", loading: info.status === "loading" };
  },
  "tab.focus": async (params) => {
    await focusTab(await ownedTab(params));
    return {};
  },
  "tab.close": async (params) => {
    const tab = tabParam(params);
    if (await isOwned(tab)) await closeTab(tab);
    else if (!(await wasClosed(tab))) throw new BridgeError("not_owned", `tab ${tab} was not opened by JobPilot`);
    return {};
  },
  "tab.closeAll": async () => {
    await closeAll();
    return {};
  },
  "cdp.send": async (params) => {
    const tab = await ownedTab(params);
    const method = stringParam(params, "method");
    if (!isAllowedMethod(method)) throw new BridgeError("not_allowed", `${method} is not allowed`);
    const cdpParams = objectParam(params, "params");
    const session = optionalStringParam(params, "session");
    // The URL rule of `tab.open` holds for every navigation (no file:, chrome:, javascript: ...).
    if (method === "Page.navigate" && !(typeof cdpParams.url === "string" && isAllowedUrl(cdpParams.url))) {
      throw new BridgeError("bad_params", "`url` must be http:, https: or about:blank");
    }
    try {
      return await send(tab, method, cdpParams, session);
    } finally {
      if (method === "Fetch.continueRequest" || method === "Fetch.failRequest") resolved(tab, session, cdpParams);
    }
  },
  "cookies.domains": cookieDomains,
};

/** Run one call; the result, or a `BridgeError` / other error for the `error` reply. */
export async function handleCall(call: Call): Promise<unknown> {
  const run = methods[call.method];
  if (!run || !Object.hasOwn(methods, call.method)) {
    throw new BridgeError("bad_params", `unknown method ${call.method}`);
  }
  if (call.params === null) throw new BridgeError("bad_params", "`params` must be an object");
  return run(call.params);
}
