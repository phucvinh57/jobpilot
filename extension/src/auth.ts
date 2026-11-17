// The handshake's proofs (PROTOCOL.md, "Handshake and pairing"): HMAC-SHA256 keyed by the token (its
// characters) over a prefixed nonce, as lower-case hex. The app answers the extension's nonce with
// `jobpilot-app:<nonce>`, the extension answers the app's with `jobpilot-ext:<nonce>`, so one side's
// answer is never valid as the other's. Same as `crates/engine/src/browser/bridge/pairing.rs`.
// Tokens, nonces and proofs are never logged.

const APP = "jobpilot-app:";
const EXT = "jobpilot-ext:";
const encoder = new TextEncoder();

const toHex = (bytes: Uint8Array): string => Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");

function fromHex(text: string): Uint8Array<ArrayBuffer> | null {
  if (text.length % 2 !== 0 || !/^[0-9a-fA-F]*$/.test(text)) return null;
  const bytes = new Uint8Array(text.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = Number.parseInt(text.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

const key = (token: string, usage: "sign" | "verify"): Promise<CryptoKey> =>
  crypto.subtle.importKey("raw", encoder.encode(token), { name: "HMAC", hash: "SHA-256" }, false, [usage]);

/** 32 random bytes as 64 lower-case hex characters. */
export function newNonce(): string {
  return toHex(crypto.getRandomValues(new Uint8Array(32)));
}

async function sign(token: string, prefix: string, nonce: string): Promise<string> {
  const mac = await crypto.subtle.sign("HMAC", await key(token, "sign"), encoder.encode(prefix + nonce));
  return toHex(new Uint8Array(mac));
}

/** The app's answer to the extension's nonce (the extension only checks it; tests use this). */
export const appProof = (token: string, extNonce: string): Promise<string> => sign(token, APP, extNonce);

/** The extension's answer to the app's nonce. */
export const extProof = (token: string, appNonce: string): Promise<string> => sign(token, EXT, appNonce);

/** Is `proof` the app's answer to `extNonce` for `token`? Constant time (WebCrypto's verify). */
export async function verifyAppProof(token: string, extNonce: string, proof: string): Promise<boolean> {
  const bytes = fromHex(proof);
  if (token === "" || bytes === null || bytes.length !== 32) return false;
  return crypto.subtle.verify("HMAC", await key(token, "verify"), bytes, encoder.encode(APP + extNonce));
}
