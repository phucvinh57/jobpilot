// @vitest-environment node
import { describe, expect, it } from "vitest";

import { appProof, extProof, newNonce, verifyAppProof } from "./auth";

// The same vector is checked by the app (`crates/engine/src/browser/bridge/pairing.rs`).
const TOKEN = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const NONCE = "0f".repeat(32);
const APP_PROOF = "f121701b6248be0864763d4ba74c37535456af3125ef7e9d700c0a4eb862dbd0";
const EXT_PROOF = "2cb63fe35b8ba5cc761110c06bef34353c149c6eebd169371c6cdcd15eb28f74";

describe("proofs", () => {
  it("match the test vector the app checks too", async () => {
    expect(await appProof(TOKEN, NONCE)).toBe(APP_PROOF);
    expect(await extProof(TOKEN, NONCE)).toBe(EXT_PROOF);
  });

  it("verify the app's proof only for the same token, nonce and side", async () => {
    expect(await verifyAppProof(TOKEN, NONCE, APP_PROOF)).toBe(true);
    expect(await verifyAppProof(TOKEN, NONCE, APP_PROOF.toUpperCase())).toBe(true);
    expect(await verifyAppProof(TOKEN, NONCE, EXT_PROOF)).toBe(false);
    expect(await verifyAppProof(TOKEN, "0e".repeat(32), APP_PROOF)).toBe(false);
    expect(await verifyAppProof("1".repeat(64), NONCE, APP_PROOF)).toBe(false);
    expect(await verifyAppProof(TOKEN, NONCE, APP_PROOF.slice(0, 62))).toBe(false);
    expect(await verifyAppProof(TOKEN, NONCE, "zz".repeat(32))).toBe(false);
    expect(await verifyAppProof(TOKEN, NONCE, "")).toBe(false);
    expect(await verifyAppProof("", NONCE, APP_PROOF)).toBe(false);
  });

  it("makes random 64-hex nonces", () => {
    const [a, b] = [newNonce(), newNonce()];
    expect(a).toMatch(/^[0-9a-f]{64}$/);
    expect(a).not.toBe(b);
  });
});
