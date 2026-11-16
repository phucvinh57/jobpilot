// @vitest-environment node
import { describe, expect, it } from "vitest";

import { CDP_ALLOWLIST, isAllowedMethod, isAllowedUrl, isPrivateUrl, PRIVATE_HOST_REGEXES } from "./allowlist";

describe("CDP allowlist", () => {
  it("passes exactly the methods of PROTOCOL.md", () => {
    expect(CDP_ALLOWLIST.size).toBe(23);
    for (const method of ["Page.navigate", "Runtime.evaluate", "Input.insertText", "Fetch.failRequest"]) {
      expect(isAllowedMethod(method)).toBe(true);
    }
  });

  it("refuses everything else", () => {
    for (const method of [
      "Network.getCookies",
      "Network.getAllCookies",
      "Storage.getCookies",
      "Target.createTarget",
      "Target.setAutoAttach",
      "Browser.close",
      "Page.setDownloadBehavior",
      "Runtime.compileScript",
      "runtime.evaluate",
      "Runtime.evaluate ",
      "",
    ]) {
      expect(isAllowedMethod(method), method).toBe(false);
    }
  });
});

describe("tab URLs", () => {
  it("allows http, https and about:blank", () => {
    expect(isAllowedUrl("https://boards.greenhouse.io/acme/jobs/1")).toBe(true);
    expect(isAllowedUrl("http://example.com")).toBe(true);
    expect(isAllowedUrl("about:blank")).toBe(true);
  });

  it("refuses other schemes and junk", () => {
    for (const url of [
      "file:///etc/passwd",
      "chrome://settings",
      "chrome-extension://gghdiehhgfalihbgeafcjegccbbinodl/popup.html",
      "javascript:alert(1)",
      "data:text/html,hi",
      "about:settings",
      "about:blank#x",
      "ftp://example.com",
      "view-source:https://example.com",
      "example.com",
      "",
    ]) {
      expect(isAllowedUrl(url), url).toBe(false);
    }
  });
});

describe("guard rule (private hosts)", () => {
  const blocked = [
    "http://localhost/",
    "http://localhost:8080/x",
    "http://localhost./",
    "https://api.localhost/",
    "http://printer.local/",
    "https://db.corp.internal:5432/",
    "ws://localhost:9222/devtools",
    "http://user:pass@localhost/",
    "http://0.0.0.0/",
    "http://0.1.2.3/",
    "http://10.0.0.1/",
    "http://10.255.255.255:80/a?b#c",
    "http://100.64.0.1/",
    "http://100.127.255.255/",
    "http://127.0.0.1:47615/jobpilot",
    "http://127.1.2.3",
    "http://169.254.169.254/latest/meta-data",
    "http://172.16.0.1/",
    "http://172.31.255.255/",
    "http://192.168.1.1/",
    "http://198.18.0.1/",
    "http://198.19.255.255/",
    "http://224.0.0.1/",
    "http://239.255.255.250/",
    "http://240.0.0.1/",
    "http://255.255.255.255/",
    "http://[::1]/",
    "http://[::1]:8080/",
    "http://[::]/",
    "http://[::ffff:7f00:1]/",
    "http://[fc00::1]/",
    "http://[fd12:3456:789a::1]/",
    "http://[fe80::1]/",
    "http://[febf::1]/",
    "HTTP://LOCALHOST/",
    "https://local/",
    "https://internal:8443/",
    "wss://LocalHost:9222/",
    // other notations Chrome canonicalizes to a private literal
    "http://0x7f.1/",
    "http://2130706433/",
    "http://0177.0.0.1/",
    "http://[::ffff:127.0.0.1]/",
    "http://[0:0:0:0:0:0:0:1]/",
    "http://[FE80::1]/",
  ];
  const allowed = [
    "https://example.com/",
    "https://localhost.example.com/",
    "https://mylocalhost/",
    "https://internal.example.com/",
    "https://127.0.0.1.nip.io/",
    "https://10.example.com/",
    "http://1.1.1.1/",
    "http://8.8.8.8/",
    "http://11.0.0.1/",
    "http://100.63.255.255/",
    "http://100.128.0.1/",
    "http://126.255.255.255/",
    "http://128.0.0.1/",
    "http://169.253.1.1/",
    "http://169.255.1.1/",
    "http://172.15.255.255/",
    "http://172.32.0.1/",
    "http://192.167.1.1/",
    "http://192.169.1.1/",
    "http://198.17.255.255/",
    "http://198.20.0.1/",
    "http://223.255.255.255/",
    "http://[2001:db8::1]/",
    "http://[2606:4700::1111]/",
    "http://[fc::1]/",
    "http://[fec0::1]/",
    "https://example.com/?next=http://localhost/",
    "https://example.com/redirect/127.0.0.1",
    "https://localhost@example.com/",
  ];

  it.each(blocked)("blocks %s", (url) => expect(isPrivateUrl(url)).toBe(true));
  it.each(allowed)("lets %s through to the app", (url) => expect(isPrivateUrl(url)).toBe(false));

  it("is RE2-compatible (no lookaround, no backreference)", () => {
    for (const source of PRIVATE_HOST_REGEXES) {
      expect(source).not.toMatch(/\(\?[=!<]/);
      expect(source).not.toMatch(/\\[1-9]/);
    }
  });
});
