# JobPilot browser bridge protocol (v1)

The JobPilot desktop app drives the user's own Chrome through the JobPilot extension. The **app is the
server**; the extension's service worker is the only client. The extension is a dumb, allowlisted relay:
every decision (what to open, what to fill, which host is allowed) is made in the app
(`src-tauri/src/engine/browser/bridge/`). This file is the contract between the two sides; change both
together.

## Identity and transport

- Extension id: `gghdiehhgfalihbgeafcjegccbbinodl` (fixed by the public `key` in `manifest.json`; there is
  no private key in the repo, the id only depends on the public one).
- WebSocket `ws://127.0.0.1:<port>/jobpilot`, default port **47615**. The app may be told another port
  with `JOBPILOT_BRIDGE_PORT`; the extension reads its port from, in order: `chrome.storage.local.port`
  (set in the popup), `config.json` packaged next to `manifest.json` (`{"port": 1234}`, tests write
  it), else 47615.
- The app accepts the upgrade only when the `Origin` header is `chrome-extension://<extension id>` and
  the path is `/jobpilot`; anything else gets HTTP 403. One extension connection at a time: a new
  authenticated connection replaces the old one.
- Every frame is one JSON text message with a `type`. Unknown `type`s are ignored by both sides.
- The extension sends `{"type":"ping"}` every 20 s (an active WebSocket keeps an MV3 service worker
  alive); the app answers `{"type":"pong"}`. The extension reconnects with backoff 1 s, 2 s, 4 s … 30 s
  max, forever, and on `chrome.runtime.onStartup` / `onInstalled` and a 1-minute `chrome.alarms` tick.

## Handshake and pairing

1. Extension → app, first message:
   `{"type":"hello","protocol":1,"extension":"0.1.0","browser":"<navigator.userAgent>","incognitoAllowed":true,"token":"<token>"|null}`
   (`incognitoAllowed` = `chrome.extension.isAllowedIncognitoAccess()`).
2. App → extension, one of:
   - `{"type":"welcome"}`: the token matched the one in the app's keyring. Calls may follow.
   - `{"type":"pair","code":"123456"}`: no or unknown token. The extension shows the code in its popup;
     the app shows the same code in Settings and the user approves there.
   - `{"type":"paired","token":"<64 hex chars>"}` (after a `pair`, once the user approved): the
     extension stores it in `chrome.storage.local.token`, then the app sends `welcome`.
   - `{"type":"rejected","reason":"..."}` (wrong protocol version, user declined, pairing timed out):
     the app then closes the socket. On "declined" the extension clears its stored token.
3. A hello whose `protocol` is not 1 is rejected with reason `protocol`.
4. `{"type":"forget"}` (app → extension, any time): the user forgot this browser. The extension clears
   its token, closes every tab it owns and reconnects (and will be asked to pair again).

Token values never appear in logs on either side.

## Calls (app → extension)

`{"type":"call","id":<u64>,"method":"<name>","params":{...}}` answered by
`{"type":"result","id":<u64>,"result":<value>}` or
`{"type":"error","id":<u64>,"error":{"code":"<code>","message":"<text>"}}`.

Error codes: `not_owned` (a tab the extension did not open, or one already closed), `not_allowed` (a CDP
method outside the allowlist), `incognito_not_allowed`, `bad_params`, `cdp` (Chrome refused; `message`
is Chrome's text), `internal`.

Tabs are numbers (`chrome.tabs` ids). **The extension acts only on tabs it opened itself** (an
owned-tab set, kept in `chrome.storage.session` so it survives a service-worker restart). It never
reads, lists or touches any other tab of the user.

| method | params | result |
|---|---|---|
| `tab.open` | `{"url": string, "private": bool, "focus": bool, "guard": bool}` | `{"tab": number, "window": number}` |
| `tab.navigate` | `{"tab", "url"}` | `{}` (or error `cdp` with Chrome's `errorText`) |
| `tab.info` | `{"tab"}` | `{"url": string, "title": string, "loading": bool}` |
| `tab.focus` | `{"tab"}` | `{}` |
| `tab.close` | `{"tab"}` | `{}` (closing a tab that is already gone is not an error) |
| `tab.closeAll` | `{}` | `{}` |
| `cdp.send` | `{"tab", "method": string, "params": object, "session"?: string}` | Chrome's result object |
| `cookies.domains` | `{"domain": string, "private": bool}` | `{"domains": [string]}` |

### `tab.open`

1. Ensure the JobPilot window of that kind exists (one per kind: `private` = an incognito window,
   else a normal window): created with `chrome.windows.create({url:"about:blank", incognito, focused:
   focus, width:1280, height:800})` and remembered in `chrome.storage.session`. If `private` and the
   extension is not allowed in incognito: error `incognito_not_allowed`.
2. Create the tab `about:blank` in that window (`active: true`), add it to a tab group titled
   "JobPilot" (grey) in that window, add it to the owned set.
3. `chrome.debugger.attach({tabId}, "1.3")`, then send:
   `Emulation.setAutomationOverride {enabled:true}` (sites can see the page is automated:
   `navigator.webdriver` is true; JobPilot never hides automation),
   `Emulation.setFocusEmulationEnabled {enabled:true}` (a background tab keeps rendering),
   `Page.enable`, `Runtime.enable`.
4. If `guard`: install the guard (below) before the first navigation.
5. Navigate with `Page.navigate {url}`; if Chrome returns `errorText`, the call fails with code `cdp`.
6. If `focus`: focus the window and the tab. Return without waiting for the load; the app polls
   `document.readyState` with `Runtime.evaluate`.

`url` must be `http:`, `https:` or `about:blank`; anything else is `bad_params`.

### `tab.focus`

`chrome.windows.update(windowId, {focused:true, state:"normal"})` and `chrome.tabs.update(tab,
{active:true})`. Used for login windows and hand-offs ("the user deals with it in their browser").

### `cdp.send` allowlist

Only these methods pass; anything else is `not_allowed`:

```text
Page.navigate Page.enable Page.captureScreenshot Page.getFrameTree Page.getLayoutMetrics
Page.handleJavaScriptDialog Runtime.evaluate Runtime.enable Runtime.runIfWaitingForDebugger
DOM.getDocument DOM.querySelector DOM.getBoxModel DOM.scrollIntoViewIfNeeded DOM.describeNode
DOM.resolveNode Input.dispatchMouseEvent Input.dispatchKeyEvent Input.insertText
Fetch.enable Fetch.continueRequest Fetch.failRequest Emulation.setAutomationOverride
Emulation.setFocusEmulationEnabled
```

`session` (optional) is a CDP child-session id (from a `Target.attachedToTarget` event, see the guard)
and is passed as `{tabId, sessionId}` to `chrome.debugger.sendCommand` (Chrome 125+).

### `cookies.domains`

`chrome.cookies.getAll({domain})` in the normal cookie store, or the incognito store when `private`
(`chrome.cookies.getAllCookieStores` → the store of the JobPilot incognito window). Returns the distinct
`cookie.domain` values with the leading dot removed, sorted. **Cookie names and values never leave
the browser.**

## Events (extension → app)

`{"type":"event","event":"<name>","params":{...}}`

| event | params | when |
|---|---|---|
| `tabClosed` | `{"tab"}` | an owned tab was closed (by the user or by `tab.close`); it leaves the owned set |
| `detached` | `{"tab","reason"}` | `chrome.debugger.onDetach` for an owned tab (the user pressed Cancel on Chrome's "started debugging" bar, or the tab closed) |
| `requestPaused` | `{"tab","session"?: string,"requestId","url"}` | a guarded tab's request is paused (see the guard); the app must answer it |
| `dialog` | `{"tab","kind","message"}` | `Page.javascriptDialogOpening` on an owned tab |

## The guard (discovery tabs: keep pages off the machine and the private network)

A per-tab proxy is impossible from an extension, so for a `guard: true` tab:

1. **Literal hosts, in the extension.** A `declarativeNetRequest` *session* rule scoped to `tabIds:
   [tab]` blocks every request whose host is `localhost`, `*.localhost`, `*.local`, `*.internal`, an
   IPv4 literal in `0.0.0.0/8 10/8 100.64/10 127/8 169.254/16 172.16/12 192.168/16 198.18/15 224/4
   240/4` or an IPv6 literal (`[::1]`, `[fc..]`, `[fd..]`, `[fe80..]`, `[::ffff:..]`, `[::]`). The rule
   is removed when the tab closes.
2. **Every other request, in the app.** `Fetch.enable {patterns:[{urlPattern:"*",requestStage:"Request"}]}`
   on the tab, and `Target.setAutoAttach {autoAttach:true, waitForDebuggerOnStart:true, flatten:true}`
   so iframes in other processes and workers are attached too; on `Target.attachedToTarget` the
   extension sends `Fetch.enable` (same patterns) to the child session, then
   `Runtime.runIfWaitingForDebugger`. Each `Fetch.requestPaused` becomes a `requestPaused` event; the
   app resolves the host with the `net::http` public-host rules and answers with
   `cdp.send Fetch.continueRequest {requestId}` or `Fetch.failRequest {requestId,
   errorReason:"BlockedByClient"}` on the same `session`. The extension sends nothing on its own:
   an unanswered request simply stays paused.

The guard is weaker than the old forward proxy (Chrome resolves the name again after the app checked
it, and a service worker that is not attached is not covered); the literal rule and Chrome's own
Private Network Access checks are the second line.

## Lifetime

- An owned tab is closed by `tab.close`, `tab.closeAll` (the app's exit), `forget`, or the user.
- When the socket drops, the extension keeps the tabs open (an apply waiting for the user must
  survive an app restart) but answers every still-paused guard request with `Fetch.failRequest`.
- The JobPilot windows are reused; when the last owned tab in one closes, the extension closes that
  window only if it holds no other tab.
