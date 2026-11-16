// @vitest-environment node
import { describe, expect, it } from "vitest";

import { DEFAULT_PORT, parsePort } from "./config";
import {
  BridgeError,
  backoffMs,
  boolParam,
  errorReply,
  objectParam,
  optionalStringParam,
  parseIncoming,
  resultReply,
  tabParam,
} from "./protocol";

describe("parseIncoming", () => {
  it("reads the handshake messages", () => {
    expect(parseIncoming('{"type":"welcome"}')).toEqual({ type: "welcome" });
    expect(parseIncoming('{"type":"pair","code":"123456"}')).toEqual({ type: "pair", code: "123456" });
    expect(parseIncoming('{"type":"challenge","proof":"ab","nonce":"cd"}')).toEqual({
      type: "challenge",
      proof: "ab",
      nonce: "cd",
    });
    expect(parseIncoming(`{"type":"paired","token":"${"a".repeat(64)}"}`)).toEqual({
      type: "paired",
      token: "a".repeat(64),
    });
    expect(parseIncoming('{"type":"rejected","reason":"declined"}')).toEqual({ type: "rejected", reason: "declined" });
    expect(parseIncoming('{"type":"rejected"}')).toEqual({ type: "rejected", reason: "" });
    expect(parseIncoming('{"type":"forget"}')).toEqual({ type: "forget" });
    expect(parseIncoming('{"type":"pong"}')).toEqual({ type: "pong" });
  });

  it("reads calls, with empty params when there are none", () => {
    expect(parseIncoming('{"type":"call","id":7,"method":"tab.info","params":{"tab":3}}')).toEqual({
      type: "call",
      id: 7,
      method: "tab.info",
      params: { tab: 3 },
    });
    expect(parseIncoming('{"type":"call","id":8,"method":"tab.closeAll"}')).toEqual({
      type: "call",
      id: 8,
      method: "tab.closeAll",
      params: {},
    });
    expect(parseIncoming('{"type":"call","id":9,"method":"tab.info","params":[1]}')).toMatchObject({ params: null });
  });

  it("ignores unknown types and malformed frames", () => {
    for (const text of [
      "",
      "not json",
      "[]",
      "null",
      '{"type":"hello"}',
      '{"type":"surprise"}',
      '{"type":"pair"}',
      '{"type":"challenge","proof":"ab"}',
      '{"type":"challenge","proof":"ab","nonce":""}',
      '{"type":"challenge","nonce":"cd"}',
      '{"type":"paired","token":""}',
      '{"type":"call","id":"1","method":"tab.info"}',
      '{"type":"call","id":-1,"method":"tab.info"}',
      '{"type":"call","id":1.5,"method":"tab.info"}',
      '{"type":"call","id":1}',
    ]) {
      expect(parseIncoming(text), text).toBeNull();
    }
  });
});

describe("replies", () => {
  it("wraps results and errors", () => {
    expect(resultReply(1, { tab: 2 })).toEqual({ type: "result", id: 1, result: { tab: 2 } });
    expect(errorReply(2, new BridgeError("not_owned", "no"))).toEqual({
      type: "error",
      id: 2,
      error: { code: "not_owned", message: "no" },
    });
    expect(errorReply(3, new Error("boom"))).toEqual({
      type: "error",
      id: 3,
      error: { code: "internal", message: "boom" },
    });
  });
});

describe("params", () => {
  it("checks types", () => {
    expect(tabParam({ tab: 4 })).toBe(4);
    expect(() => tabParam({ tab: "4" })).toThrow(BridgeError);
    expect(() => tabParam({})).toThrow(/tab/);
    expect(boolParam({}, "focus")).toBe(false);
    expect(boolParam({ focus: true }, "focus")).toBe(true);
    expect(() => boolParam({ focus: "yes" }, "focus")).toThrow(BridgeError);
    expect(objectParam({}, "params")).toEqual({});
    expect(() => objectParam({ params: [] }, "params")).toThrow(BridgeError);
    expect(optionalStringParam({}, "session")).toBeUndefined();
    expect(() => optionalStringParam({ session: 1 }, "session")).toThrow(BridgeError);
  });
});

describe("reconnect backoff", () => {
  it("doubles from 1 s up to 30 s, forever", () => {
    const seconds = Array.from({ length: 9 }, (_, attempt) => backoffMs(attempt) / 1000);
    expect(seconds).toEqual([1, 2, 4, 8, 16, 30, 30, 30, 30]);
    expect(backoffMs(1000)).toBe(30_000);
  });
});

describe("port", () => {
  it("accepts a TCP port as a number or a numeric string", () => {
    expect(parsePort(1234)).toBe(1234);
    expect(parsePort("47615")).toBe(DEFAULT_PORT);
    for (const value of [0, 65536, -1, 1.5, "", "12a", null, undefined, {}]) expect(parsePort(value)).toBeNull();
  });
});
