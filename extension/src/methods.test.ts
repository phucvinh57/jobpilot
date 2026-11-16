// @vitest-environment node
import { beforeEach, describe, expect, it, vi } from "vitest";

import { PRIVATE_HOST_REGEXES } from "./allowlist";
import { failAllPaused, onAttachedToTarget, onRequestPaused, pausedCount } from "./guard";
import { distinctDomains, handleCall } from "./methods";
import { setSink } from "./outbox";
import { resetOwnedCache } from "./owned";
import type { Call, Params } from "./protocol";

type Sent = { tabId?: number; sessionId?: string; method: string; params?: Params };

/** A fake `chrome` with just what the methods use: one existing user tab (id 1) the extension must never touch. */
function fakeChrome() {
  const session = new Map<string, unknown>();
  const area = (map: Map<string, unknown>) => ({
    get: vi.fn(async (keys: string[] | string) => {
      const out: Record<string, unknown> = {};
      for (const k of Array.isArray(keys) ? keys : [keys]) if (map.has(k)) out[k] = map.get(k);
      return out;
    }),
    set: vi.fn(async (items: Record<string, unknown>) => {
      for (const [k, v] of Object.entries(items)) map.set(k, structuredClone(v));
    }),
  });
  let nextTab = 100;
  const tabs = new Map<number, { id: number; windowId: number; url: string; title: string; status: string }>();
  tabs.set(1, { id: 1, windowId: 1, url: "https://mail.example.com/", title: "User's own", status: "complete" });
  const windows = new Map<number, { id: number; incognito: boolean }>([[1, { id: 1, incognito: false }]]);
  const sent: Sent[] = [];
  const responses: Record<string, (s: Sent) => Params> = {};
  let attached = new Set<number>();
  const fake = {
    sent,
    openTabs: tabs,
    responses,
    detach: (tab: number) => attached.delete(tab),
    storage: { session: area(session), local: area(new Map()) },
    extension: { isAllowedIncognitoAccess: vi.fn(async () => true) },
    windows: {
      get: vi.fn(async (id: number) => {
        const w = windows.get(id);
        if (!w) throw new Error(`No window with id: ${id}.`);
        return w;
      }),
      create: vi.fn(async (opts: { incognito?: boolean }) => {
        const id = 10 + windows.size;
        windows.set(id, { id, incognito: !!opts.incognito });
        const tab = { id: nextTab++, windowId: id, url: "about:blank", title: "", status: "complete" };
        tabs.set(tab.id, tab);
        return { id, incognito: !!opts.incognito, tabs: [tab] };
      }),
      update: vi.fn(async () => ({})),
    },
    tabs: {
      create: vi.fn(async (opts: { windowId: number }) => {
        const tab = { id: nextTab++, windowId: opts.windowId, url: "about:blank", title: "", status: "complete" };
        tabs.set(tab.id, tab);
        return tab;
      }),
      get: vi.fn(async (id: number) => {
        const t = tabs.get(id);
        if (!t) throw new Error(`No tab with id: ${id}.`);
        return t;
      }),
      remove: vi.fn(async (id: number) => {
        if (!tabs.delete(id)) throw new Error(`No tab with id: ${id}.`);
      }),
      update: vi.fn(async () => ({})),
      group: vi.fn(async () => 5),
    },
    tabGroups: { query: vi.fn(async () => []), update: vi.fn(async () => ({})) },
    debugger: {
      attach: vi.fn(async ({ tabId }: { tabId: number }) => {
        attached.add(tabId);
      }),
      sendCommand: vi.fn(async (target: { tabId: number; sessionId?: string }, method: string, params?: Params) => {
        if (!attached.has(target.tabId))
          throw new Error(`Debugger is not attached to the tab with id: ${target.tabId}.`);
        const s = { ...target, method, params };
        sent.push(s);
        return responses[method]?.(s) ?? {};
      }),
    },
    declarativeNetRequest: { updateSessionRules: vi.fn(async () => undefined) },
    cookies: {
      getAll: vi.fn(async () => [
        { domain: ".example.com", name: "sid", value: "secret" },
        { domain: "example.com", name: "a", value: "b" },
        { domain: "jobs.example.com", name: "c", value: "d" },
      ]),
      getAllCookieStores: vi.fn(async () => [
        { id: "0", tabIds: [1] },
        { id: "1", tabIds: [] as number[] },
      ]),
    },
    reset: () => {
      attached = new Set();
    },
  };
  return fake;
}

let chromeFake: ReturnType<typeof fakeChrome>;
let callId = 0;
const call = (method: string, params: Params | null = {}) =>
  handleCall({ type: "call", id: ++callId, method, params } satisfies Call);

beforeEach(() => {
  chromeFake = fakeChrome();
  vi.stubGlobal("chrome", chromeFake);
  resetOwnedCache();
  setSink(null);
  vi.spyOn(console, "info").mockImplementation(() => undefined);
});

describe("tab.open", () => {
  it("opens a tab in a JobPilot window, attaches, prepares and navigates", async () => {
    const result = await call("tab.open", { url: "https://example.com/", private: false, focus: false, guard: false });
    expect(result).toEqual({ tab: 100, window: 11 });
    expect(chromeFake.windows.create).toHaveBeenCalledWith(
      expect.objectContaining({ url: "about:blank", incognito: false, focused: false, width: 1280, height: 800 }),
    );
    expect(chromeFake.debugger.attach).toHaveBeenCalledWith({ tabId: 100 }, "1.3");
    expect(chromeFake.sent.map((s) => s.method)).toEqual([
      "Emulation.setAutomationOverride",
      "Emulation.setFocusEmulationEnabled",
      "Page.enable",
      "Runtime.enable",
      "Page.navigate",
    ]);
    expect(chromeFake.sent[0]?.params).toEqual({ enabled: true });
    expect(chromeFake.tabs.group).toHaveBeenCalledWith({ tabIds: [100], createProperties: { windowId: 11 } });
    expect(chromeFake.tabGroups.update).toHaveBeenCalledWith(5, { title: "JobPilot", color: "grey" });
  });

  it("reuses the window for the next tab", async () => {
    await call("tab.open", { url: "about:blank" });
    const second = await call("tab.open", { url: "about:blank" });
    expect(second).toEqual({ tab: 101, window: 11 });
    expect(chromeFake.windows.create).toHaveBeenCalledTimes(1);
    expect(chromeFake.tabs.create).toHaveBeenCalledWith({ windowId: 11, url: "about:blank", active: true });
  });

  it("installs the guard before the first navigation", async () => {
    await call("tab.open", { url: "https://example.com/", guard: true });
    const rules = chromeFake.declarativeNetRequest.updateSessionRules;
    expect(rules).toHaveBeenLastCalledWith({
      removeRuleIds: [1, 2, 3],
      addRules: PRIVATE_HOST_REGEXES.map((regexFilter, i) =>
        expect.objectContaining({
          id: i + 1,
          action: { type: "block" },
          condition: expect.objectContaining({
            regexFilter,
            isUrlFilterCaseSensitive: true,
            tabIds: [100],
            resourceTypes: expect.arrayContaining(["main_frame", "sub_frame", "websocket"]),
          }),
        }),
      ),
    });
    const methods = chromeFake.sent.map((s) => s.method);
    expect(methods.slice(4)).toEqual(["Fetch.enable", "Target.setAutoAttach", "Page.navigate"]);
    expect(chromeFake.sent[5]?.params).toEqual({ autoAttach: true, waitForDebuggerOnStart: true, flatten: true });
  });

  it("refuses bad URLs and private tabs without incognito access", async () => {
    await expect(call("tab.open", { url: "file:///etc/passwd" })).rejects.toMatchObject({ code: "bad_params" });
    chromeFake.extension.isAllowedIncognitoAccess.mockResolvedValue(false);
    await expect(call("tab.open", { url: "https://example.com/", private: true })).rejects.toMatchObject({
      code: "incognito_not_allowed",
    });
    expect(chromeFake.windows.create).not.toHaveBeenCalled();
  });

  it("fails with Chrome's errorText and closes the tab it opened", async () => {
    chromeFake.responses["Page.navigate"] = () => ({ errorText: "net::ERR_NAME_NOT_RESOLVED" });
    await expect(call("tab.open", { url: "https://nope.invalid/" })).rejects.toMatchObject({
      code: "cdp",
      message: "net::ERR_NAME_NOT_RESOLVED",
    });
    expect(chromeFake.openTabs.has(100)).toBe(false);
    await expect(call("tab.info", { tab: 100 })).rejects.toMatchObject({ code: "not_owned" });
  });
});

describe("owned tabs only", () => {
  it("refuses a tab the extension did not open, for every tab method", async () => {
    for (const method of ["tab.navigate", "tab.info", "tab.focus", "tab.close", "cdp.send"]) {
      await expect(
        call(method, { tab: 1, url: "https://example.com/", method: "Runtime.evaluate" }),
        method,
      ).rejects.toMatchObject({
        code: "not_owned",
      });
    }
    expect(chromeFake.tabs.get).not.toHaveBeenCalledWith(1);
    expect(chromeFake.tabs.remove).not.toHaveBeenCalled();
    expect(chromeFake.debugger.sendCommand).not.toHaveBeenCalled();
  });

  it("survives a service-worker restart (the owned set is in storage.session)", async () => {
    await call("tab.open", { url: "about:blank" });
    resetOwnedCache();
    expect(await call("tab.info", { tab: 100 })).toEqual({ url: "about:blank", title: "", loading: false });
  });

  it("closes a tab, emits tabClosed once, and closing it again is not an error", async () => {
    const events: unknown[] = [];
    setSink((m) => {
      events.push(m);
      return true;
    });
    await call("tab.open", { url: "about:blank" });
    expect(await call("tab.close", { tab: 100 })).toEqual({});
    expect(await call("tab.close", { tab: 100 })).toEqual({});
    expect(events).toEqual([{ type: "event", event: "tabClosed", params: { tab: 100 } }]);
    await expect(call("cdp.send", { tab: 100, method: "Runtime.evaluate" })).rejects.toMatchObject({
      code: "not_owned",
    });
  });

  it("closeAll closes only owned tabs", async () => {
    await call("tab.open", { url: "about:blank" });
    await call("tab.open", { url: "about:blank" });
    await call("tab.closeAll");
    expect([...chromeFake.openTabs.keys()]).toEqual([1]);
  });
});

describe("cdp.send", () => {
  it("passes allowlisted methods, with the child session", async () => {
    await call("tab.open", { url: "about:blank" });
    chromeFake.responses["Runtime.evaluate"] = () => ({ result: { type: "string", value: "Title" } });
    expect(
      await call("cdp.send", { tab: 100, method: "Runtime.evaluate", params: { expression: "document.title" } }),
    ).toEqual({ result: { type: "string", value: "Title" } });
    await call("cdp.send", { tab: 100, method: "Fetch.continueRequest", params: { requestId: "r1" }, session: "S1" });
    expect(chromeFake.debugger.sendCommand).toHaveBeenLastCalledWith(
      { tabId: 100, sessionId: "S1" },
      "Fetch.continueRequest",
      {
        requestId: "r1",
      },
    );
  });

  it("refuses methods outside the allowlist and navigation to other schemes", async () => {
    await call("tab.open", { url: "about:blank" });
    const before = chromeFake.sent.length;
    await expect(call("cdp.send", { tab: 100, method: "Network.getAllCookies" })).rejects.toMatchObject({
      code: "not_allowed",
    });
    await expect(
      call("cdp.send", { tab: 100, method: "Page.navigate", params: { url: "file:///etc/passwd" } }),
    ).rejects.toMatchObject({ code: "bad_params" });
    expect(chromeFake.sent.length).toBe(before);
  });

  it("re-attaches once after the user cancelled debugging", async () => {
    await call("tab.open", { url: "about:blank" });
    chromeFake.detach(100);
    await call("cdp.send", { tab: 100, method: "Runtime.evaluate", params: { expression: "1" } });
    expect(chromeFake.debugger.attach).toHaveBeenCalledTimes(2);
    expect(chromeFake.sent.at(-1)?.method).toBe("Runtime.evaluate");
  });

  it("reports Chrome's failure as cdp", async () => {
    await call("tab.open", { url: "about:blank" });
    chromeFake.responses["DOM.querySelector"] = () => {
      throw new Error('{"code":-32000,"message":"Could not find node with given id"}');
    };
    await expect(call("cdp.send", { tab: 100, method: "DOM.querySelector", params: {} })).rejects.toMatchObject({
      code: "cdp",
      message: expect.stringContaining("Could not find node"),
    });
  });
});

describe("other calls", () => {
  it("refuses unknown methods and non-object params", async () => {
    await expect(call("tabs.query")).rejects.toMatchObject({ code: "bad_params" });
    await expect(call("toString")).rejects.toMatchObject({ code: "bad_params" });
    await expect(call("tab.info", null)).rejects.toMatchObject({ code: "bad_params" });
  });

  it("cookies.domains returns distinct domains only, never names or values", async () => {
    const result = await call("cookies.domains", { domain: "example.com", private: false });
    expect(result).toEqual({ domains: ["example.com", "jobs.example.com"] });
    expect(JSON.stringify(result)).not.toContain("secret");
    expect(chromeFake.cookies.getAll).toHaveBeenCalledWith({ domain: "example.com" });
  });

  it("cookies.domains in private mode reads the JobPilot incognito store", async () => {
    expect(await call("cookies.domains", { domain: "example.com", private: true })).toEqual({ domains: [] });
    await call("tab.open", { url: "about:blank", private: true });
    chromeFake.cookies.getAllCookieStores.mockResolvedValue([
      { id: "0", tabIds: [1] },
      { id: "1", tabIds: [100] },
    ]);
    await call("cookies.domains", { domain: "example.com", private: true });
    expect(chromeFake.cookies.getAll).toHaveBeenLastCalledWith({ domain: "example.com", storeId: "1" });
  });

  it("distinctDomains strips the leading dot and sorts", () => {
    expect(distinctDomains([".b.com", "a.com", "b.com", ".a.com", ""])).toEqual(["a.com", "b.com"]);
  });
});

describe("guard", () => {
  it("scopes the rules to the guarded tabs and drops them with the last one", async () => {
    await call("tab.open", { url: "about:blank", guard: true });
    await call("tab.open", { url: "about:blank", guard: false });
    await call("tab.open", { url: "about:blank", guard: true });
    const rules = chromeFake.declarativeNetRequest.updateSessionRules;
    const lastTabs = () =>
      (rules.mock.lastCall as unknown as [{ addRules: chrome.declarativeNetRequest.Rule[] }])[0].addRules.map(
        (r) => r.condition.tabIds,
      );
    expect(lastTabs()).toEqual([
      [100, 102],
      [100, 102],
      [100, 102],
    ]);
    await call("tab.close", { tab: 100 });
    expect(lastTabs()).toEqual([[102], [102], [102]]);
    await call("tab.close", { tab: 101 });
    expect(rules).toHaveBeenCalledTimes(3);
    await call("tab.close", { tab: 102 });
    expect(rules).toHaveBeenLastCalledWith({ removeRuleIds: [1, 2, 3], addRules: [] });
  });

  const linkTo = (events: unknown[]) =>
    setSink((m) => {
      events.push(m);
      return true;
    });

  it("forwards paused requests to the app, and fails them when the socket drops", async () => {
    const events: unknown[] = [];
    linkTo(events);
    await call("tab.open", { url: "about:blank", guard: true });
    onRequestPaused(100, undefined, { requestId: "r1", request: { url: "https://example.com/" } });
    onRequestPaused(100, "S1", { requestId: "r2", request: { url: "https://cdn.example.com/a.js" } });
    expect(events).toEqual([
      { type: "event", event: "requestPaused", params: { tab: 100, requestId: "r1", url: "https://example.com/" } },
      {
        type: "event",
        event: "requestPaused",
        params: { tab: 100, session: "S1", requestId: "r2", url: "https://cdn.example.com/a.js" },
      },
    ]);
    await call("cdp.send", { tab: 100, method: "Fetch.continueRequest", params: { requestId: "r1" } });
    expect(pausedCount()).toBe(1);
    setSink(null);
    await failAllPaused();
    expect(pausedCount()).toBe(0);
    expect(chromeFake.sent.at(-1)).toEqual({
      tabId: 100,
      sessionId: "S1",
      method: "Fetch.failRequest",
      params: { requestId: "r2", errorReason: "BlockedByClient" },
    });
  });

  it("fails a paused request at once when the app is not connected", async () => {
    await call("tab.open", { url: "about:blank", guard: true });
    onRequestPaused(100, undefined, { requestId: "r9", request: { url: "https://example.com/" } });
    await vi.waitFor(() => expect(chromeFake.sent.at(-1)?.method).toBe("Fetch.failRequest"));
    expect(pausedCount()).toBe(0);
  });

  it("guards a child target before it runs", async () => {
    await call("tab.open", { url: "about:blank", guard: true });
    const before = chromeFake.sent.length;
    await onAttachedToTarget(100, { sessionId: "C1", waitingForDebugger: true });
    expect(chromeFake.sent.slice(before).map((s) => [s.sessionId, s.method])).toEqual([
      ["C1", "Fetch.enable"],
      ["C1", "Target.setAutoAttach"],
      ["C1", "Runtime.runIfWaitingForDebugger"],
    ]);
  });
});
