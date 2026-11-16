// What the app may ask for (extension/PROTOCOL.md): the CDP allowlist, the URLs a tab may open, and the
// private-host rule of the guard. Pure, no `chrome`.

/** The only CDP methods `cdp.send` passes; anything else is `not_allowed`. */
export const CDP_ALLOWLIST: ReadonlySet<string> = new Set([
  "Page.navigate",
  "Page.enable",
  "Page.captureScreenshot",
  "Page.getFrameTree",
  "Page.getLayoutMetrics",
  "Page.handleJavaScriptDialog",
  "Runtime.evaluate",
  "Runtime.enable",
  "Runtime.runIfWaitingForDebugger",
  "DOM.getDocument",
  "DOM.querySelector",
  "DOM.getBoxModel",
  "DOM.scrollIntoViewIfNeeded",
  "DOM.describeNode",
  "DOM.resolveNode",
  "Input.dispatchMouseEvent",
  "Input.dispatchKeyEvent",
  "Input.insertText",
  "Fetch.enable",
  "Fetch.continueRequest",
  "Fetch.failRequest",
  "Emulation.setAutomationOverride",
  "Emulation.setFocusEmulationEnabled",
]);

export const isAllowedMethod = (method: string): boolean => CDP_ALLOWLIST.has(method);

/** A tab may open `http:`, `https:` or exactly `about:blank`. */
export function isAllowedUrl(url: string): boolean {
  if (url === "about:blank") return true;
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  return (parsed.protocol === "http:" || parsed.protocol === "https:") && parsed.hostname !== "";
}

// The guard's literal-host rule as RE2 regexes over the request URL (declarativeNetRequest
// `regexFilter`, case-sensitive). Chrome matches them against the canonical URL: scheme and host are
// lower-case, an IPv4 address in any notation is dotted decimal, an IPv6 address is compressed
// lower-case hex in brackets. Chrome caps a compiled regex at 2 KB, so there is one regex per kind of host.
const start = "^[a-z]+://([^/?#@]*@)?";
const end = "\\.?(:[0-9]*)?([/?#]|$)";
const n = "[0-9]+";
const ipv4 = [
  `(0|10|127)\\.${n}`, //                              0.0.0.0/8, 10/8, 127/8
  "100\\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])", //  100.64/10
  "169\\.254", //                                     169.254/16
  "172\\.(1[6-9]|2[0-9]|3[01])", //                   172.16/12
  "192\\.168", //                                     192.168/16
  "198\\.1[89]", //                                   198.18/15
  `(22[4-9]|2[3-5][0-9])\\.${n}`, //                  224/4 and 240/4 (224-255)
].join("|");
const ipv6 = [
  "::", //                       ::, ::1, ::ffff:.. (IPv4-mapped) and the rest of ::/96
  "f[cd][0-9a-f]{2}:", //        fc00::/7 (unique local)
  "fe[89ab][0-9a-f]:", //        fe80::/10 (link local)
].join("|");

/** Block a URL whose host is machine-local or private (PROTOCOL.md, "The guard"); one rule each. */
export const PRIVATE_HOST_REGEXES: readonly string[] = [
  `${start}([^/?#@:\\[\\]]*\\.)?(localhost|local|internal)${end}`,
  `${start}(${ipv4})\\.${n}\\.${n}${end}`,
  `${start}\\[(${ipv6})[^/?#\\]]*\\](:[0-9]*)?([/?#]|$)`,
];

const privateHosts = PRIVATE_HOST_REGEXES.map((source) => new RegExp(source));

/**
 * Whether the guard's rules block this URL: the same regexes, run in JavaScript on the URL in its
 * canonical form (WHATWG, as Chrome sees it). Tests and diagnostics.
 */
export function isPrivateUrl(url: string): boolean {
  let canonical: string;
  try {
    canonical = new URL(url).href;
  } catch {
    return false;
  }
  return privateHosts.some((re) => re.test(canonical));
}
