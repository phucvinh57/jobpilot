// Where the app listens: `chrome.storage.local.port` (the popup), else `config.json` packaged next
// to the manifest (`{"port": 1234}`, tests write it), else 47615.
//
// `config.json` may also say `"autoConsent": true` (tests only: the extension then says Pair to a
// pairing code by itself, as the user would in the popup). That knob is read from the packaged file
// only, never from storage or the app; the shipped extension has no config.json.

export const DEFAULT_PORT = 47615;

/** A valid TCP port, or `null`. Accepts a number or a numeric string. */
export function parsePort(value: unknown): number | null {
  const port = typeof value === "string" && /^\s*\d+\s*$/.test(value) ? Number(value) : value;
  return typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65535 ? port : null;
}

type Packaged = { port: number | null; autoConsent: boolean };

/** What `config.json` (packaged with the extension) says; read once per service-worker start. */
let packaged: Promise<Packaged> | undefined;

async function readPackaged(): Promise<Packaged> {
  try {
    const response = await fetch(chrome.runtime.getURL("config.json"));
    if (!response.ok) return { port: null, autoConsent: false };
    const config: unknown = await response.json();
    if (typeof config !== "object" || config === null) return { port: null, autoConsent: false };
    const { port, autoConsent } = config as { port?: unknown; autoConsent?: unknown };
    return { port: parsePort(port), autoConsent: autoConsent === true };
  } catch {
    return { port: null, autoConsent: false };
  }
}

const packagedConfig = (): Promise<Packaged> => {
  packaged ??= readPackaged();
  return packaged;
};

export async function resolvePort(): Promise<number> {
  const { port } = await chrome.storage.local.get("port");
  return parsePort(port) ?? (await packagedConfig()).port ?? DEFAULT_PORT;
}

/** Tests only: consent to pairing codes without the popup (from the packaged config.json only). */
export async function autoConsent(): Promise<boolean> {
  return (await packagedConfig()).autoConsent;
}

export const bridgeUrl = (port: number): string => `ws://127.0.0.1:${port}/jobpilot`;
