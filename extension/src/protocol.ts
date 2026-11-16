// The wire format of the bridge (extension/PROTOCOL.md): pure, no `chrome`.

export const PROTOCOL_VERSION = 1;
export const EXTENSION_VERSION = "0.1.0";

export type ErrorCode = "not_owned" | "not_allowed" | "incognito_not_allowed" | "bad_params" | "cdp" | "internal";

/** A failure that becomes an `error` reply with this code. */
export class BridgeError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "BridgeError";
  }
}

export type Params = Record<string, unknown>;

/** A call from the app; `params` is `null` when it was sent but is not an object (`bad_params`). */
export type Call = { type: "call"; id: number; method: string; params: Params | null };

/** What the app may send. Anything else (unknown `type`, bad JSON) parses to `null` and is ignored. */
export type Incoming =
  | { type: "welcome" }
  | { type: "pair"; code: string }
  | { type: "paired"; token: string }
  | { type: "rejected"; reason: string }
  | { type: "forget" }
  | { type: "pong" }
  | Call;

export type Reply =
  | { type: "result"; id: number; result: unknown }
  | { type: "error"; id: number; error: { code: ErrorCode; message: string } };

const isObject = (value: unknown): value is Params =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export function parseIncoming(text: string): Incoming | null {
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (!isObject(value)) return null;
  switch (value.type) {
    case "welcome":
    case "forget":
    case "pong":
      return { type: value.type };
    case "pair":
      return typeof value.code === "string" ? { type: "pair", code: value.code } : null;
    case "paired":
      return typeof value.token === "string" && value.token.length > 0 ? { type: "paired", token: value.token } : null;
    case "rejected":
      return { type: "rejected", reason: typeof value.reason === "string" ? value.reason : "" };
    case "call": {
      const { id, method } = value;
      if (typeof id !== "number" || !Number.isSafeInteger(id) || id < 0 || typeof method !== "string") return null;
      const params = value.params === undefined || value.params === null ? {} : value.params;
      return { type: "call", id, method, params: isObject(params) ? params : null };
    }
    default:
      return null;
  }
}

export const resultReply = (id: number, result: unknown): Reply => ({ type: "result", id, result });

export function errorReply(id: number, error: unknown): Reply {
  if (error instanceof BridgeError) return { type: "error", id, error: { code: error.code, message: error.message } };
  const message = error instanceof Error ? error.message : String(error);
  return { type: "error", id, error: { code: "internal", message } };
}

/** Reconnect delay after `attempt` failed attempts (0-based): 1 s, 2 s, 4 s ... 30 s max. */
export function backoffMs(attempt: number): number {
  const seconds = 2 ** Math.min(Math.max(attempt, 0), 5);
  return Math.min(seconds, 30) * 1000;
}

// Typed readers for call params: a wrong type is `bad_params`.

export function tabParam(params: Params): number {
  const tab = params.tab;
  if (typeof tab !== "number" || !Number.isSafeInteger(tab) || tab < 0) {
    throw new BridgeError("bad_params", "`tab` must be a tab id");
  }
  return tab;
}

export function stringParam(params: Params, name: string): string {
  const value = params[name];
  if (typeof value !== "string") throw new BridgeError("bad_params", `\`${name}\` must be a string`);
  return value;
}

export function optionalStringParam(params: Params, name: string): string | undefined {
  const value = params[name];
  if (value === undefined || value === null) return undefined;
  if (typeof value !== "string") throw new BridgeError("bad_params", `\`${name}\` must be a string`);
  return value;
}

export function boolParam(params: Params, name: string): boolean {
  const value = params[name];
  if (value === undefined || value === null) return false;
  if (typeof value !== "boolean") throw new BridgeError("bad_params", `\`${name}\` must be a boolean`);
  return value;
}

export function objectParam(params: Params, name: string): Params {
  const value = params[name];
  if (value === undefined || value === null) return {};
  if (!isObject(value)) throw new BridgeError("bad_params", `\`${name}\` must be an object`);
  return value;
}
