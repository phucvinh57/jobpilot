// Where the app listens: `chrome.storage.local.port` (the popup), else `config.json` packaged next
// to the manifest (`{"port": 1234}`, tests write it), else 47615.

export const DEFAULT_PORT = 47615;

/** A valid TCP port, or `null`. Accepts a number or a numeric string. */
export function parsePort(value: unknown): number | null {
  const port = typeof value === "string" && /^\s*\d+\s*$/.test(value) ? Number(value) : value;
  return typeof port === "number" && Number.isInteger(port) && port >= 1 && port <= 65535 ? port : null;
}

async function packagedPort(): Promise<number | null> {
  try {
    const response = await fetch(chrome.runtime.getURL("config.json"));
    if (!response.ok) return null;
    const config: unknown = await response.json();
    return typeof config === "object" && config !== null ? parsePort((config as { port?: unknown }).port) : null;
  } catch {
    return null;
  }
}

export async function resolvePort(): Promise<number> {
  const { port } = await chrome.storage.local.get("port");
  return parsePort(port) ?? (await packagedPort()) ?? DEFAULT_PORT;
}

export const bridgeUrl = (port: number): string => `ws://127.0.0.1:${port}/jobpilot`;
