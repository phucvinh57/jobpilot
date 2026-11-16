// @vitest-environment node
// The handshake against a fake app on a fake WebSocket: the extension trusts nothing that answers on
// the port until it proved it holds the token, and only takes a token the user said Pair to.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { appProof, extProof } from "./auth";
import type { Status } from "./connection";

type Message = Record<string, unknown>;

class FakeSocket {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static all: FakeSocket[] = [];
  readyState = FakeSocket.CONNECTING;
  sent: Message[] = [];
  raw: string[] = [];
  onopen: ((event: unknown) => void) | null = null;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onclose: ((event: unknown) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;

  constructor(readonly url: string) {
    FakeSocket.all.push(this);
  }

  send(data: string): void {
    this.raw.push(data);
    this.sent.push(JSON.parse(data) as Message);
  }

  close(): void {
    if (this.readyState === FakeSocket.CLOSED) return;
    this.readyState = FakeSocket.CLOSED;
    this.onclose?.({});
  }

  open(): void {
    this.readyState = FakeSocket.OPEN;
    this.onopen?.({});
  }

  receive(message: Message): void {
    this.onmessage?.({ data: JSON.stringify(message) });
  }
}

function area() {
  const map = new Map<string, unknown>();
  return {
    map,
    get: vi.fn(async (keys: string[] | string) => {
      const out: Record<string, unknown> = {};
      for (const k of Array.isArray(keys) ? keys : [keys]) if (map.has(k)) out[k] = map.get(k);
      return out;
    }),
    set: vi.fn(async (items: Record<string, unknown>) => {
      for (const [k, v] of Object.entries(items)) map.set(k, structuredClone(v));
    }),
    remove: vi.fn(async (key: string) => {
      map.delete(key);
    }),
  };
}

function fakeChrome() {
  return {
    storage: { local: area(), session: area() },
    extension: { isAllowedIncognitoAccess: vi.fn(async () => false) },
    runtime: { getURL: (path: string) => `chrome-extension://gghdiehhgfalihbgeafcjegccbbinodl/${path}` },
    declarativeNetRequest: { updateSessionRules: vi.fn(async () => undefined) },
  };
}

const TOKEN = "ab".repeat(32);
const APP_NONCE = "cd".repeat(32);

let chromeFake: ReturnType<typeof fakeChrome>;
let connection: typeof import("./connection");

/** One turn of Node's event loop (`setImmediate` is not faked; the extension's types do not know it). */
const tick = () =>
  new Promise<void>((resolve) =>
    (globalThis as unknown as { setImmediate: (callback: () => void) => void }).setImmediate(resolve),
  );

/** Let the extension's promises (storage, WebCrypto) run until `check` holds. */
async function until(what: string, check: () => boolean): Promise<void> {
  // Real time (only the timers are fake): WebCrypto answers from Node's thread pool.
  const end = Date.now() + 5000;
  while (Date.now() < end) {
    if (check()) return;
    await tick();
  }
  throw new Error(`timed out waiting for ${what}`);
}

/** Connect and open the socket; returns it with the extension's `hello`. */
async function connected(): Promise<{ ws: FakeSocket; hello: Message }> {
  await connection.connect();
  const ws = FakeSocket.all.at(-1);
  if (!ws) throw new Error("no socket");
  ws.open();
  await until("the hello", () => ws.sent.length > 0);
  const hello = ws.sent[0] as Message;
  return { ws, hello };
}

const status = () => chromeFake.storage.session.map.get("status") as Status | undefined;
const stored = () => chromeFake.storage.local.map.get("token");

beforeEach(async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout", "setInterval", "clearInterval"] });
  FakeSocket.all = [];
  chromeFake = fakeChrome();
  vi.stubGlobal("chrome", chromeFake);
  vi.stubGlobal("WebSocket", FakeSocket);
  vi.stubGlobal(
    "fetch",
    vi.fn(async () => new Response("", { status: 404 })),
  );
  vi.spyOn(console, "info").mockImplementation(() => undefined);
  vi.spyOn(console, "warn").mockImplementation(() => undefined);
  vi.resetModules();
  connection = await import("./connection");
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("a paired extension", () => {
  beforeEach(() => {
    chromeFake.storage.local.map.set("token", TOKEN);
  });

  it("says it is paired and sends a nonce, never the token", async () => {
    const { hello } = await connected();
    expect(hello).toMatchObject({ type: "hello", protocol: 1, paired: true, incognitoAllowed: false });
    expect(hello.nonce).toMatch(/^[0-9a-f]{64}$/);
    expect(hello).not.toHaveProperty("token");
  });

  it("closes on an impostor's wrong proof without sending the token or a proof of its own", async () => {
    const { ws, hello } = await connected();
    ws.receive({ type: "challenge", proof: await appProof("ef".repeat(32), hello.nonce as string), nonce: APP_NONCE });
    await until("the close", () => ws.readyState === FakeSocket.CLOSED);
    expect(ws.sent.map((m) => m.type)).toEqual(["hello"]);
    expect(ws.raw.join("")).not.toContain(TOKEN);
    expect(status()?.state).toBe("impostor");
    expect(stored()).toBe(TOKEN);

    // Calls and a forget from it are not answered; the impostor state stays while reconnecting.
    ws.receive({ type: "forget" });
    expect(stored()).toBe(TOKEN);
    await vi.advanceTimersByTimeAsync(1000);
    expect(FakeSocket.all).toHaveLength(2);
    expect(status()?.state).toBe("impostor");
  });

  it("answers the app's correct proof with its own, then is welcomed", async () => {
    const { ws, hello } = await connected();
    ws.receive({ type: "challenge", proof: await appProof(TOKEN, hello.nonce as string), nonce: APP_NONCE });
    await until("the auth", () => ws.sent.length === 2);
    expect(ws.sent[1]).toEqual({ type: "auth", proof: await extProof(TOKEN, APP_NONCE) });
    ws.receive({ type: "welcome" });
    await until("connected", () => status()?.state === "connected");
    expect(ws.raw.join("")).not.toContain(TOKEN);

    ws.receive({ type: "call", id: 1, method: "tab.bogus", params: {} });
    await until("the answer", () => ws.sent.length === 3);
    expect(ws.sent[2]).toMatchObject({ type: "error", id: 1, error: { code: "bad_params" } });
  });

  it("refuses a welcome that did not follow a proof", async () => {
    const { ws } = await connected();
    ws.receive({ type: "welcome" });
    await until("the close", () => ws.readyState === FakeSocket.CLOSED);
    expect(status()?.state).toBe("impostor");
    expect(ws.sent.map((m) => m.type)).toEqual(["hello"]);
  });

  it("ignores a forget before it was welcomed", async () => {
    const { ws } = await connected();
    ws.receive({ type: "forget" });
    for (let i = 0; i < 20; i++) await tick();
    expect(stored()).toBe(TOKEN);
    expect(ws.readyState).toBe(FakeSocket.OPEN);
  });
});

describe("pairing", () => {
  it("refuses a token the user did not say Pair to", async () => {
    const { ws, hello } = await connected();
    expect(hello).toMatchObject({ paired: false });
    ws.receive({ type: "pair", code: "123456" });
    await until("pairing", () => status()?.state === "pairing");
    expect(status()).toMatchObject({ code: "123456" });
    ws.receive({ type: "paired", token: TOKEN });
    await until("the close", () => ws.readyState === FakeSocket.CLOSED);
    expect(stored()).toBeUndefined();
    expect(status()?.state).toBe("refused");
    expect(ws.sent.map((m) => m.type)).toEqual(["hello"]);
  });

  it("stores the token after the user's consent for that code, then is welcomed", async () => {
    const { ws } = await connected();
    ws.receive({ type: "pair", code: "123456" });
    await until("pairing", () => status()?.state === "pairing");
    expect(connection.consent("654321")).toBe(false);
    expect(connection.consent("123456")).toBe(true);
    expect(ws.sent.at(-1)).toEqual({ type: "consent", code: "123456" });
    expect(status()).toMatchObject({ state: "pairing", code: "123456", consented: true });

    ws.receive({ type: "paired", token: TOKEN });
    ws.receive({ type: "welcome" });
    await until("connected", () => status()?.state === "connected");
    expect(stored()).toBe(TOKEN);
    expect(ws.readyState).toBe(FakeSocket.OPEN);
  });

  it("refuses a token after a new code replaced the one the user said Pair to", async () => {
    const { ws } = await connected();
    ws.receive({ type: "pair", code: "123456" });
    await until("pairing", () => status()?.state === "pairing");
    connection.consent("123456");
    ws.receive({ type: "pair", code: "999999" });
    ws.receive({ type: "paired", token: TOKEN });
    await until("the close", () => ws.readyState === FakeSocket.CLOSED);
    expect(stored()).toBeUndefined();
    expect(status()?.state).toBe("refused");
  });

  it("consents by itself only when the packaged config.json says autoConsent", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async () => new Response(JSON.stringify({ port: 4321, autoConsent: true }), { status: 200 })),
    );
    const { ws } = await connected();
    expect(ws.url).toBe("ws://127.0.0.1:4321/jobpilot");
    ws.receive({ type: "pair", code: "123456" });
    await until("the consent", () => ws.sent.some((m) => m.type === "consent"));
    expect(ws.sent.at(-1)).toEqual({ type: "consent", code: "123456" });
    ws.receive({ type: "paired", token: TOKEN });
    ws.receive({ type: "welcome" });
    await until("connected", () => status()?.state === "connected");
    expect(stored()).toBe(TOKEN);
  });

  it("does not consent by itself because storage says so", async () => {
    chromeFake.storage.local.map.set("autoConsent", true);
    const { ws } = await connected();
    ws.receive({ type: "pair", code: "123456" });
    await until("pairing", () => status()?.state === "pairing");
    for (let i = 0; i < 20; i++) await tick();
    expect(ws.sent.map((m) => m.type)).toEqual(["hello"]);
  });
});
