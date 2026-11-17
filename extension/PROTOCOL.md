# JobPilot browser bridge protocol (v1)

The JobPilot desktop app drives the user's own Chrome through the JobPilot extension. The **app is the
server**; the extension's service worker is the only client. The extension is a dumb, allowlisted relay:
every decision (what to open, what to fill, which host is allowed) is made in the app
(`crates/engine/src/browser/bridge/`). This file is the contract between the two sides; change both
together.

## Identity and transport

- Extension id: `gghdiehhgfalihbgeafcjegccbbinodl` (fixed by the public `key` in `manifest.json`; there is
  no private key in the repo, the id only depends on the public one).
- WebSocket `ws://127.0.0.1:<port>/jobpilot`, default port **47615**. The app may be told another port
  with `JOBPILOT_BRIDGE_PORT`; the extension reads its port from, in order: `chrome.storage.local.port`
  (set in the popup), `config.json` packaged next to `manifest.json` (`{"port": 1234}`, tests write
  it), else 47615.
- **Test-only knob:** the testkit's `config.json` also says `"autoConsent": true`: the extension then
  sends `consent` for every `pair` code by itself, as if the user had pressed Pair in its popup. The
  extension honours it only from the packaged `config.json` (never from `chrome.storage` or from the
  app); the shipped package has no `config.json`.
- The app accepts the upgrade only when the `Origin` header is `chrome-extension://<extension id>` and
  the path is `/jobpilot`; anything else gets HTTP 403. The Origin is only a filter (a non-browser
  client can forge it): what lets a peer in is the handshake below. One extension connection at a
  time: a new authenticated connection replaces the old one.
- Every frame is one JSON text message with a `type`. Unknown `type`s are ignored by both sides.
- The extension sends `{"type":"ping"}` every 20 s (an active WebSocket keeps an MV3 service worker
  alive); the app answers `{"type":"pong"}`. The extension reconnects with backoff 1 s, 2 s, 4 s … 30 s
  max, forever, and on `chrome.runtime.onStartup` / `onInstalled` and a 1-minute `chrome.alarms` tick.

## Handshake and pairing

Neither side trusts the other until it proved it holds the pairing token (32 random bytes as 64 hex
characters, in the app's OS keyring and the extension's `chrome.storage.local.token`). **The token
travels once, in `paired`; after that only proofs do.** Any local program can bind the port before
JobPilot, so the extension treats whatever answers as untrusted.

A proof is lower-case hex of HMAC-SHA256 keyed by the token (its 64 ASCII characters) over a prefix
and the other side's nonce (64 hex characters, 32 random bytes):

- the app's proof: `HMAC(token, "jobpilot-app:" + extensionNonce)`;
- the extension's proof: `HMAC(token, "jobpilot-ext:" + appNonce)`.

The prefixes keep one side's proof from being replayed as the other's. Both sides compare proofs in
constant time (the app with the `hmac` crate, the extension with WebCrypto `crypto.subtle.verify`).
Test vector (checked by both sides' tests): token
`00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff`, nonce `0f` × 32 → app proof
`f121701b6248be0864763d4ba74c37535456af3125ef7e9d700c0a4eb862dbd0`, extension proof
`2cb63fe35b8ba5cc761110c06bef34353c149c6eebd169371c6cdcd15eb28f74`.

1. Extension → app, first message:
   `{"type":"hello","protocol":1,"extension":"0.1.0","browser":"<navigator.userAgent>","incognitoAllowed":true,"paired":true|false,"nonce":"<64 hex>"}`
   (`incognitoAllowed` = `chrome.extension.isAllowedIncognitoAccess()`; `paired`: it holds a token —
   it never sends it). A hello whose `protocol` is not 1 is rejected with reason `protocol`.
2. **Paired on both sides** (the app holds a token, `paired` is true and `nonce` is 64 hex):
   - App → `{"type":"challenge","proof":"<app proof for the extension's nonce>","nonce":"<64 hex>"}`.
   - The extension verifies the proof. **On a mismatch** it sends nothing else, closes the socket, keeps
     its token, and the popup says "Another program answered on port N, not JobPilot" (state
     `impostor`, kept while it retries with the usual backoff; the popup offers "Pair again", which
     drops the token, for when JobPilot was since paired with another browser). **On a match** →
     `{"type":"auth","proof":"<extension proof for the app's nonce>"}`.
   - The app verifies it: a match → `{"type":"welcome"}` (calls may follow); else (the extension's
     token is stale or unknown) → the pairing below. No `auth` within 10 s: the app closes.
3. **Otherwise** (either side has no token, or a bad nonce) → pairing, which needs the user's consent
   on **both** sides (numeric comparison):
   - App → `{"type":"pair","code":"123456"}`. The app shows the code in Settings (Pair / Decline); the
     extension's popup shows it with a "Pair with JobPilot" button and "Pair only if JobPilot shows
     the same code".
   - When the user presses Pair in the popup: extension → `{"type":"consent","code":"123456"}`.
   - The app sends `{"type":"paired","token":"<64 hex>"}` only once it has **both** the user's
     approval in the app and the extension's `consent` for that code, in either order (Settings says
     "Now press Pair in the extension's popup" while only the app side approved). Then `welcome`.
   - The extension takes a `paired` only if the user pressed Pair for the code of the latest `pair`
     on this connection; otherwise it drops the token, sends nothing, closes the socket and the
     popup shows the pairing as refused (state `refused`). It accepts `welcome` only after a verified
     `challenge` or a consented `paired` on the same connection; any other `welcome` closes the socket
     (state `impostor`).
   - `{"type":"rejected","reason":"..."}` (user declined, pairing timed out after 5 minutes): the app
     then closes the socket. The extension keeps whatever token it has.
4. `{"type":"forget"}` (app → extension, after `welcome`): the user forgot this browser. The extension
   clears its token, closes every tab it owns and reconnects (and will be asked to pair again). It
   ignores a `forget` before `welcome`, so an unproven peer cannot un-pair it. (Only the app the
   extension proved can send it; were a peer able to, it could only un-pair the browser, which the
   user then pairs again with both consents.)

Tokens, proofs and nonces never appear in logs on either side.

Known limit: pairing compares a code, it does not exchange keys. A program that holds the port *and*
relays to a real JobPilot listening on another port could pass the codes through and learn the token
if the user approves on both sides; a challenge cannot stop that, only the user's check that the app
is the one on the port the popup names.

## Calls (app → extension)

`{"type":"call","id":<u64>,"method":"<name>","params":{...}}` answered by
`{"type":"result","id":<u64>,"result":<value>}` or
`{"type":"error","id":<u64>,"error":{"code":"<code>","message":"<text>"}}`.

Error codes: `not_owned` (a tab the extension did not open), `not_allowed` (a CDP method outside the
allowlist), `incognito_not_allowed`, `bad_params` (also an unknown method), `cdp` (Chrome refused;
`message` is Chrome's text), `internal` (also a call before `welcome`: "not paired").

Calls run concurrently in the extension. **On a guarded tab, `tab.open` and `tab.navigate` only return
once the app has answered the `requestPaused` of the main document**, so the app must keep handling
events and other calls while one is pending.

Tabs are numbers (`chrome.tabs` ids). **The extension acts only on tabs it opened itself** (an
owned-tab set, kept in `chrome.storage.session` so it survives a service-worker restart). It never
reads, lists or touches any other tab of the user.

| method | params | result |
|---|---|---|
| `tab.open` | `{"url": string, "private": bool, "focus": bool, "guard": bool}` | `{"tab": number, "window": number}` |
| `tab.navigate` | `{"tab", "url"}` | `{}` (or error `cdp` with Chrome's `errorText`) |
| `tab.info` | `{"tab"}` | `{"url": string, "title": string, "loading": bool}` |
| `tab.focus` | `{"tab"}` | `{}` |
| `tab.close` | `{"tab"}` | `{}` (closing an owned tab that is already gone is not an error) |
| `tab.closeAll` | `{}` | `{}` |
| `cdp.send` | `{"tab", "method": string, "params": object, "session"?: string}` | Chrome's result object |
| `cookies.domains` | `{"domain": string, "private": bool}` | `{"domains": [string]}` |

### `tab.open`

1. Ensure the JobPilot window of that kind exists (one per kind: `private` = an incognito window,
   else a normal window): created with `chrome.windows.create({url:"about:blank", incognito, focused:
   focus, width:1280, height:800})` and remembered in `chrome.storage.session`. If `private` and the
   extension is not allowed in incognito: error `incognito_not_allowed`.
2. Create the tab `about:blank` in that window (`active: true`) — or, for a window just created, use its
   first tab — add it to a tab group titled "JobPilot" (grey) in that window, add it to the owned set.
   If a later step fails, the tab is closed again (a `tabClosed` may arrive for an id the app never
   saw).
3. `chrome.debugger.attach({tabId}, "1.3")`, then send:
   `Emulation.setAutomationOverride {enabled:true}` (sites can see the page is automated:
   `navigator.webdriver` is true; JobPilot never hides automation),
   `Emulation.setFocusEmulationEnabled {enabled:true}` (a background tab keeps rendering),
   `Page.enable`, `Runtime.enable`.
4. If `guard`: install the guard (below) before the first navigation.
5. Navigate with `Page.navigate {url}`; if Chrome returns `errorText`, the call fails with code `cdp`.
6. If `focus`: focus the window and the tab. Return without waiting for the load; the app polls
   `document.readyState` with `Runtime.evaluate`.

`url` must be `http:`, `https:` or `about:blank`; anything else is `bad_params`. The same rule applies
to `tab.navigate` and to `Page.navigate` sent through `cdp.send`.

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

1. **Literal hosts, in the extension.** Three `declarativeNetRequest` *session* rules (ids 1–3: names,
   IPv4, IPv6 — one regex for all is over Chrome's regex memory limit) whose `tabIds` are all guarded
   tabs (rebuilt when one opens or closes, removed when none is left) block every request whose host is `localhost`, `*.localhost`, `local`, `*.local`, `internal`, `*.internal`, an
   IPv4 literal in `0.0.0.0/8 10/8 100.64/10 127/8 169.254/16 172.16/12 192.168/16 198.18/15 224/4
   240/4` or an IPv6 literal (`[::1]`, `[fc..]`, `[fd..]`, `[fe80..]`, `[::ffff:..]`, `[::]`). The rule
   is removed when the tab closes.
2. **Every other request, in the app.** `Fetch.enable {patterns:[{urlPattern:"*",requestStage:"Request"}]}`
   on the tab, and `Target.setAutoAttach {autoAttach:true, waitForDebuggerOnStart:true, flatten:true}`
   so iframes in other processes and workers are attached too; on `Target.attachedToTarget` the
   extension sends `Fetch.enable` (same patterns) and `Target.setAutoAttach` (nested frames) to the child
   session, then `Runtime.runIfWaitingForDebugger`. Each `Fetch.requestPaused` becomes a `requestPaused` event; the
   app resolves the host with the `net::http` public-host rules and answers with
   `cdp.send Fetch.continueRequest {requestId}` or `Fetch.failRequest {requestId,
   errorReason:"BlockedByClient"}` on the same `session`. The extension sends nothing on its own while
   connected: an unanswered request stays paused. A request paused while the app is not connected is
   failed at once.

The guard is weaker than the old forward proxy (Chrome resolves the name again after the app checked
it, and a service worker that is not attached is not covered); the literal rule and Chrome's own
Private Network Access checks are the second line.

## Lifetime

- An owned tab is closed by `tab.close`, `tab.closeAll` (the app's exit), `forget`, or the user.
- When the socket drops, the extension keeps the tabs open (an apply waiting for the user must
  survive an app restart) but answers every still-paused guard request with `Fetch.failRequest`.
- The JobPilot windows are reused; when the last owned tab in one closes, the extension closes that
  window only if it holds no other tab.
