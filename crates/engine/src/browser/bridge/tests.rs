//! The bridge against a fake extension (a tokio-tungstenite client): the upgrade checks, the
//! handshake (proofs both ways, the token never sent again), pairing (approved in the app and in the
//! extension, in either order), calls and their errors, events and the guard's answers. No browser.

use std::time::{Duration, Instant};

use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use super::pairing::{self, TOKEN_KEY};
use super::*;
use crate::browser::tab::{OpenOptions, Profile};

fn origin() -> String {
    format!("chrome-extension://{EXTENSION_ID}")
}

async fn started(auto_approve: bool) -> (Bridge, u16, Secrets) {
    let secrets = Secrets::memory();
    let bridge = Bridge::new(secrets.clone());
    let port = bridge.start_with(BridgeConfig { port: 0, auto_approve, allowed_origin: origin() }).await.unwrap();
    (bridge, port, secrets)
}

struct Fake {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
}

async fn dial(port: u16, path: &str, origin: Option<&str>) -> Result<Fake, tokio_tungstenite::tungstenite::Error> {
    let mut req = format!("ws://127.0.0.1:{port}{path}").into_client_request()?;
    if let Some(origin) = origin {
        req.headers_mut().insert("Origin", HeaderValue::from_str(origin).unwrap());
    }
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    Ok(Fake { ws })
}

impl Fake {
    async fn connect(port: u16) -> Fake {
        dial(port, PATH, Some(&origin())).await.unwrap()
    }

    async fn send(&mut self, v: Value) {
        self.ws.send(Message::Text(v.to_string().into())).await.unwrap();
    }

    /// The first message: `paired` says the extension holds a token; `nonce` is its nonce.
    async fn hello_with(&mut self, paired: bool, nonce: &str) {
        self.send(json!({"type":"hello","protocol":1,"extension":"0.1.0","browser":"Mozilla/5.0 (X11; Linux x86_64) Chrome/153.0.0.0 Safari/537.36","incognitoAllowed":false,"paired":paired,"nonce":nonce})).await;
    }

    /// The hello of an extension without a token.
    async fn hello(&mut self) {
        self.hello_with(false, &pairing::new_nonce()).await;
    }

    /// The hello of an extension holding `token`: checks the app's challenge proves the same token,
    /// and returns the app's nonce.
    async fn hello_paired(&mut self, token: &str) -> String {
        let nonce = pairing::new_nonce();
        self.hello_with(true, &nonce).await;
        let challenge = self.recv().await;
        assert_eq!(challenge["type"], "challenge", "{challenge}");
        assert_eq!(challenge["proof"].as_str(), Some(pairing::app_proof(token, &nonce).as_str()), "the app proves it holds the token");
        assert!(!challenge.to_string().contains(token), "the token is never sent");
        let app_nonce = challenge["nonce"].as_str().unwrap().to_string();
        assert!(pairing::is_nonce(&app_nonce) && app_nonce != nonce, "{app_nonce}");
        app_nonce
    }

    /// Reconnect with `token`: the challenge, the extension's proof, then `welcome`.
    async fn authenticate(&mut self, token: &str) {
        let app_nonce = self.hello_paired(token).await;
        self.send(json!({"type":"auth","proof":pairing::ext_proof(token, &app_nonce)})).await;
        assert_eq!(self.recv().await, json!({"type":"welcome"}));
    }

    async fn consent(&mut self, code: &str) {
        self.send(json!({"type":"consent","code":code})).await;
    }

    /// Nothing arrives for a while.
    async fn quiet(&mut self) {
        if let Ok(frame) = tokio::time::timeout(Duration::from_millis(300), self.ws.next()).await {
            panic!("expected no message, got {frame:?}");
        }
    }

    /// The next JSON message; `None` when the server closed the connection.
    async fn next(&mut self) -> Option<Value> {
        loop {
            let frame = tokio::time::timeout(Duration::from_secs(5), self.ws.next()).await.expect("no message within 5 s");
            match frame {
                Some(Ok(Message::Text(text))) => return Some(serde_json::from_str(&text).unwrap()),
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return None,
                Some(Ok(_)) => continue,
            }
        }
    }

    async fn recv(&mut self) -> Value {
        self.next().await.expect("the connection was closed")
    }

    /// The next call (method, id, params).
    async fn call(&mut self) -> (String, u64, Value) {
        let msg = self.recv().await;
        assert_eq!(msg["type"], "call", "{msg}");
        (msg["method"].as_str().unwrap().to_string(), msg["id"].as_u64().unwrap(), msg["params"].clone())
    }

    async fn answer(&mut self, id: u64, result: Value) {
        self.send(json!({"type":"result","id":id,"result":result})).await;
    }

    /// Pair through the code (approved by the user in the app, then in the extension) and return
    /// the token.
    async fn pair(&mut self, bridge: &Bridge) -> String {
        self.hello().await;
        let pair = self.recv().await;
        assert_eq!(pair["type"], "pair", "{pair}");
        bridge.approve_pairing().unwrap();
        self.consent(pair["code"].as_str().unwrap()).await;
        let paired = self.recv().await;
        assert_eq!(paired["type"], "paired", "{paired}");
        assert_eq!(self.recv().await, json!({"type":"welcome"}));
        paired["token"].as_str().unwrap().to_string()
    }
}

async fn until(what: &str, mut check: impl FnMut() -> bool) {
    for _ in 0..100 {
        if check() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn an_upgrade_from_another_origin_or_path_is_forbidden() {
    let (_bridge, port, _) = started(false).await;
    for (path, from) in [(PATH, Some("https://evil.test")), (PATH, None), ("/other", Some(origin().as_str())), ("/", Some(origin().as_str()))] {
        match dial(port, path, from).await {
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => assert_eq!(resp.status(), 403, "{path} {from:?}"),
            Err(e) => panic!("{path} {from:?}: {e}"),
            Ok(_) => panic!("{path} {from:?} was accepted"),
        }
    }
    assert!(dial(port, PATH, Some(&origin())).await.is_ok());
}

#[tokio::test]
async fn an_unknown_extension_pairs_with_the_code_the_user_approves() {
    let (bridge, port, secrets) = started(false).await;
    assert!(bridge.status().listening && bridge.status().port == port && !bridge.status().paired);
    let mut events = bridge.subscribe();
    let mut ext = Fake::connect(port).await;
    ext.hello().await;
    let pair = ext.recv().await;
    assert_eq!(pair["type"], "pair");
    let code = pair["code"].as_str().unwrap().to_string();
    assert!(code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()), "{code}");
    until("the code in the status", || bridge.status().pairing_code.as_deref() == Some(code.as_str())).await;
    let status = bridge.status();
    assert!(!status.connected);
    assert_eq!(status.browser.as_deref(), Some("Chrome 153"));
    assert_eq!(status.extension_version.as_deref(), Some("0.1.0"));
    assert_eq!(events.recv().await.unwrap(), BridgeEvent::StatusChanged);

    // Pings are answered while the user decides.
    ext.send(json!({"type":"ping"})).await;
    assert_eq!(ext.recv().await, json!({"type":"pong"}));

    // Approved in the app: nothing is sent until the user also presses Pair in the extension.
    bridge.approve_pairing().unwrap();
    until("waiting for the extension", || bridge.status().pairing_waiting_for_extension).await;
    ext.quiet().await;
    assert_eq!(secrets.get(TOKEN_KEY).unwrap(), None, "no token before both approved");
    assert!(bridge.approve_pairing().is_ok(), "approving again changes nothing");
    // A consent for another code is ignored.
    ext.consent("000000x").await;
    ext.quiet().await;

    ext.consent(&code).await;
    let paired = ext.recv().await;
    assert_eq!(paired["type"], "paired");
    let token = paired["token"].as_str().unwrap().to_string();
    assert!(token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()), "{token}");
    assert_eq!(ext.recv().await, json!({"type":"welcome"}));
    assert_eq!(secrets.get(TOKEN_KEY).unwrap().as_deref(), Some(token.as_str()));
    let status = bridge.status();
    assert!(status.connected && status.paired && status.pairing_code.is_none() && !status.pairing_waiting_for_extension, "{status:?}");
    assert!(bridge.approve_pairing().is_err(), "nothing is waiting any more");

    // The same browser proves it holds the token (never sending it) and is let in; it replaces
    // the older connection.
    let mut again = Fake::connect(port).await;
    again.authenticate(&token).await;
    assert_eq!(ext.next().await, None, "the older connection is closed");
    assert!(bridge.status().connected);
}

#[tokio::test]
async fn the_extension_may_consent_before_the_app_approves() {
    let (bridge, port, secrets) = started(false).await;
    let mut ext = Fake::connect(port).await;
    ext.hello().await;
    let pair = ext.recv().await;
    let code = pair["code"].as_str().unwrap().to_string();
    ext.consent(&code).await;
    until("the code in the status", || bridge.status().pairing_code.is_some()).await;
    ext.quiet().await;
    assert!(!bridge.status().pairing_waiting_for_extension, "the extension already said yes");
    assert_eq!(secrets.get(TOKEN_KEY).unwrap(), None);

    bridge.approve_pairing().unwrap();
    let paired = ext.recv().await;
    assert_eq!(paired["type"], "paired");
    assert_eq!(ext.recv().await, json!({"type":"welcome"}));
    assert_eq!(secrets.get(TOKEN_KEY).unwrap().as_deref(), paired["token"].as_str());
    assert!(bridge.status().connected);
}

#[tokio::test]
async fn the_token_never_travels_after_pairing() {
    let (bridge, port, _) = started(false).await;
    let mut ext = Fake::connect(port).await;
    let token = ext.pair(&bridge).await;
    drop(ext);
    until("disconnected", || !bridge.status().connected).await;

    let mut again = Fake::connect(port).await;
    let nonce = pairing::new_nonce();
    again.hello_with(true, &nonce).await;
    let mut seen = vec![again.recv().await];
    let app_nonce = seen[0]["nonce"].as_str().unwrap().to_string();
    again.send(json!({"type":"auth","proof":pairing::ext_proof(&token, &app_nonce)})).await;
    seen.push(again.recv().await);
    assert_eq!(seen[1], json!({"type":"welcome"}));

    // A call and its answer, then forget: still no token on the wire.
    let b = bridge.clone();
    let call = tokio::spawn(async move { b.call("tab.info", json!({"tab": 1})).await });
    let msg = again.recv().await;
    again.answer(msg["id"].as_u64().unwrap(), json!({"url":"","title":"","loading":false})).await;
    call.await.unwrap().unwrap();
    seen.push(msg);
    bridge.forget().unwrap();
    while let Some(msg) = again.next().await {
        seen.push(msg);
    }
    for msg in &seen {
        assert!(!msg.to_string().contains(&token), "the token was sent: {msg}");
    }
    assert!(seen.iter().any(|m| m["type"] == "forget"), "{seen:?}");
}

#[tokio::test]
async fn a_wrong_proof_from_the_extension_is_asked_to_pair() {
    let (bridge, port, secrets) = started(false).await;
    let mut ext = Fake::connect(port).await;
    let token = ext.pair(&bridge).await;
    drop(ext);
    until("disconnected", || !bridge.status().connected).await;

    // The right challenge, a proof with another token: pairing, and the stored token stays.
    let mut other = Fake::connect(port).await;
    let app_nonce = other.hello_paired(&token).await;
    other.send(json!({"type":"auth","proof":pairing::ext_proof(&"b".repeat(64), &app_nonce)})).await;
    assert_eq!(other.recv().await["type"], "pair");
    until("pairing", || bridge.status().pairing_code.is_some()).await;
    assert!(!bridge.status().connected);
    drop(other);
    until("no pairing", || bridge.status().pairing_code.is_none()).await;

    // Replaying the app's own proof as the extension's does not work either.
    let mut replay = Fake::connect(port).await;
    let app_nonce = replay.hello_paired(&token).await;
    replay.send(json!({"type":"auth","proof":pairing::app_proof(&token, &app_nonce)})).await;
    assert_eq!(replay.recv().await["type"], "pair");
    assert_eq!(secrets.get(TOKEN_KEY).unwrap().as_deref(), Some(token.as_str()), "the stored token stays");
}

#[tokio::test]
async fn an_unpaired_extension_or_an_app_without_a_token_means_pairing() {
    let (bridge, port, secrets) = started(false).await;
    // The extension says it holds a token, the app has none: pairing (no challenge).
    let mut ext = Fake::connect(port).await;
    ext.hello_with(true, &pairing::new_nonce()).await;
    assert_eq!(ext.recv().await["type"], "pair");
    drop(ext);
    until("no pairing", || bridge.status().pairing_code.is_none()).await;

    // The app holds a token, the extension says it has none: pairing.
    secrets.set(TOKEN_KEY, &"a".repeat(64)).unwrap();
    let mut ext = Fake::connect(port).await;
    ext.hello().await;
    assert_eq!(ext.recv().await["type"], "pair");
    drop(ext);
    until("no pairing", || bridge.status().pairing_code.is_none()).await;

    // A nonce that is not 64 hex characters is not answered with a proof.
    for nonce in ["", "abc", &"z".repeat(64)] {
        let mut ext = Fake::connect(port).await;
        ext.hello_with(true, nonce).await;
        assert_eq!(ext.recv().await["type"], "pair", "{nonce:?}");
    }
}

#[tokio::test]
async fn a_wrong_token_is_asked_to_pair_and_a_declined_pairing_is_rejected() {
    let (bridge, port, secrets) = started(false).await;
    secrets.set(TOKEN_KEY, &"a".repeat(64)).unwrap();
    let mut ext = Fake::connect(port).await;
    let app_nonce = ext.hello_paired(&"a".repeat(64)).await;
    ext.send(json!({"type":"auth","proof":pairing::ext_proof(&"b".repeat(64), &app_nonce)})).await;
    assert_eq!(ext.recv().await["type"], "pair");
    until("pairing", || bridge.status().pairing_code.is_some()).await;
    // Declined even after the extension consented.
    let code = bridge.status().pairing_code.unwrap();
    ext.consent(&code).await;
    until("extension consent", || bridge.shared.state().pairing.as_ref().is_some_and(|p| p.consented)).await;
    bridge.decline_pairing().unwrap();
    assert_eq!(ext.recv().await, json!({"type":"rejected","reason":"declined"}));
    assert_eq!(ext.next().await, None);
    until("no pairing", || bridge.status().pairing_code.is_none()).await;
    assert_eq!(secrets.get(TOKEN_KEY).unwrap(), Some("a".repeat(64)), "the stored token stays");
    assert!(!bridge.status().connected);
}

#[tokio::test]
async fn another_protocol_version_is_rejected() {
    let (bridge, port, _) = started(true).await;
    let mut ext = Fake::connect(port).await;
    ext.send(json!({"type":"hello","protocol":2,"extension":"9.0.0","browser":"x","incognitoAllowed":false,"paired":false,"nonce":pairing::new_nonce()})).await;
    assert_eq!(ext.recv().await, json!({"type":"rejected","reason":"protocol"}));
    assert_eq!(ext.next().await, None);
    assert!(!bridge.status().connected);
}

#[tokio::test]
async fn auto_approval_pairs_once_the_extension_consents() {
    let (bridge, port, secrets) = started(true).await;
    let mut ext = Fake::connect(port).await;
    ext.hello().await;
    let pair = ext.recv().await;
    assert_eq!(pair["type"], "pair");
    until("waiting for the extension", || bridge.status().pairing_waiting_for_extension).await;
    ext.quiet().await;
    ext.consent(pair["code"].as_str().unwrap()).await;
    let paired = ext.recv().await;
    assert_eq!(paired["type"], "paired");
    assert_eq!(ext.recv().await, json!({"type":"welcome"}));
    assert_eq!(secrets.get(TOKEN_KEY).unwrap().as_deref(), paired["token"].as_str());
    assert!(bridge.status().connected);
}

#[tokio::test]
async fn calls_get_their_results_and_typed_errors() {
    let (bridge, port, _) = started(false).await;
    let mut ext = Fake::connect(port).await;
    ext.pair(&bridge).await;

    let b = bridge.clone();
    let call = tokio::spawn(async move { b.call("tab.info", json!({"tab": 3})).await });
    let (method, id, params) = ext.call().await;
    assert_eq!((method.as_str(), params), ("tab.info", json!({"tab": 3})));
    ext.answer(id, json!({"url":"https://a.test/","title":"A","loading":false})).await;
    assert_eq!(call.await.unwrap().unwrap()["title"], "A");

    for (code, message, expected) in [
        ("not_owned", "no such tab", BridgeError::NotOwned),
        ("not_allowed", "Network.getCookies", BridgeError::NotAllowed),
        ("incognito_not_allowed", "", BridgeError::IncognitoNotAllowed),
        ("cdp", "Cannot navigate to invalid URL", BridgeError::Cdp("Cannot navigate to invalid URL".into())),
        ("bad_params", "url must be http", BridgeError::Other("url must be http".into())),
        ("internal", "", BridgeError::Other("internal".into())),
    ] {
        let b = bridge.clone();
        let call = tokio::spawn(async move { b.call("tab.focus", json!({"tab": 3})).await });
        let (_, id, _) = ext.call().await;
        ext.send(json!({"type":"error","id":id,"error":{"code":code,"message":message}})).await;
        let err = call.await.unwrap().unwrap_err();
        assert_eq!(err, expected, "{code}");
        // It rides in anyhow and can be found again.
        let wrapped = anyhow::Error::from(err.clone());
        assert_eq!(wrapped.downcast_ref::<BridgeError>(), Some(&expected));
    }
    assert!(BridgeError::IncognitoNotAllowed.to_string().contains(INCOGNITO_NOT_ALLOWED));

    // No answer: a timeout.
    let started = Instant::now();
    let err = bridge.call_timeout("tab.info", json!({"tab": 3}), Duration::from_millis(300)).await.unwrap_err();
    assert_eq!(err, BridgeError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(2));
    let _ = ext.call().await;

    // The connection drops while a call waits: NotConnected.
    let b = bridge.clone();
    let call = tokio::spawn(async move { b.call("tab.info", json!({"tab": 3})).await });
    let _ = ext.call().await;
    drop(ext);
    assert_eq!(call.await.unwrap().unwrap_err(), BridgeError::NotConnected);
    until("disconnected", || !bridge.status().connected).await;
}

#[tokio::test]
async fn a_call_without_an_extension_fails_after_waiting_three_seconds() {
    let (bridge, _, _) = started(false).await;
    let started = Instant::now();
    let err = bridge.call("tab.info", json!({"tab": 1})).await.unwrap_err();
    assert_eq!(err, BridgeError::NotConnected);
    assert!(started.elapsed() >= Duration::from_millis(2900), "{:?}", started.elapsed());
    assert!(err.to_string().contains(NOT_CONNECTED), "{err}");
}

#[tokio::test]
async fn a_call_waits_for_the_extension_to_connect() {
    let (bridge, port, _) = started(true).await;
    let b = bridge.clone();
    let call = tokio::spawn(async move { b.call("tab.info", json!({"tab": 1})).await });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mut ext = Fake::connect(port).await;
    ext.hello().await;
    let pair = ext.recv().await;
    assert_eq!(pair["type"], "pair");
    ext.consent(pair["code"].as_str().unwrap()).await;
    for kind in ["paired", "welcome"] {
        assert_eq!(ext.recv().await["type"], kind);
    }
    let (_, id, _) = ext.call().await;
    ext.answer(id, json!({"url": "", "title": "", "loading": true})).await;
    assert!(call.await.unwrap().is_ok());
}

#[tokio::test]
async fn forget_deletes_the_token_and_tells_the_extension() {
    let (bridge, port, secrets) = started(false).await;
    let mut ext = Fake::connect(port).await;
    ext.pair(&bridge).await;
    assert!(secrets.get(TOKEN_KEY).unwrap().is_some());
    bridge.forget().unwrap();
    assert_eq!(ext.recv().await, json!({"type":"forget"}));
    assert_eq!(ext.next().await, None);
    assert_eq!(secrets.get(TOKEN_KEY).unwrap(), None);
    let status = bridge.status();
    assert!(!status.paired && !status.connected, "{status:?}");
}

#[tokio::test]
async fn tab_events_close_tabs_and_are_broadcast() {
    let (bridge, port, _) = started(false).await;
    let mut ext = Fake::connect(port).await;
    ext.pair(&bridge).await;
    let mut events = bridge.subscribe();

    let b = bridge.clone();
    let open = tokio::spawn(async move { b.open("https://example.com/", OpenOptions { profile: Profile::Signed, focus: true, guard: None }).await });
    let (method, id, params) = ext.call().await;
    assert_eq!(method, "tab.open");
    assert_eq!(params, json!({"url":"https://example.com/","private":false,"focus":true,"guard":false}));
    ext.answer(id, json!({"tab": 5, "window": 2})).await;
    let tab = open.await.unwrap().unwrap();
    assert_eq!((tab.id(), tab.profile(), tab.is_open()), (5, Profile::Signed, true));

    ext.send(json!({"type":"event","event":"dialog","params":{"tab":5,"kind":"alert","message":"hi"}})).await;
    ext.send(json!({"type":"event","event":"detached","params":{"tab":5,"reason":"canceled_by_user"}})).await;
    ext.send(json!({"type":"event","event":"somethingNew","params":{}})).await;
    ext.send(json!({"type":"event","event":"tabClosed","params":{"tab":5}})).await;
    tokio::time::timeout(Duration::from_secs(5), tab.closed()).await.expect("closed() resolves");
    assert!(!tab.is_open());
    let mut seen = Vec::new();
    while let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(200), events.recv()).await {
        seen.push(event);
    }
    assert!(seen.contains(&BridgeEvent::Dialog { tab: 5, kind: "alert".into(), message: "hi".into() }), "{seen:?}");
    assert!(seen.contains(&BridgeEvent::Detached { tab: 5, reason: "canceled_by_user".into() }), "{seen:?}");
    assert!(seen.contains(&BridgeEvent::TabClosed(5)), "{seen:?}");
}

#[tokio::test]
async fn the_guard_answers_paused_requests_of_a_guarded_tab() {
    let (bridge, port, _) = started(false).await;
    let mut ext = Fake::connect(port).await;
    ext.pair(&bridge).await;
    let allowed = 4321;
    let policy = Policy::OnlyPorts(vec![allowed]);

    // An address the policy refuses is refused before anything is opened.
    let refused = bridge.open("http://127.0.0.1:9/", OpenOptions { profile: Profile::Private, focus: false, guard: Some(policy.clone()) }).await;
    assert!(refused.is_err());

    let b = bridge.clone();
    let opts = OpenOptions { profile: Profile::Private, focus: false, guard: Some(policy) };
    let url = format!("http://127.0.0.1:{allowed}/jobs");
    let open = tokio::spawn(async move { b.open(&url, opts).await });
    let (method, open_id, params) = ext.call().await;
    assert_eq!(method, "tab.open");
    assert_eq!((params["private"].as_bool(), params["guard"].as_bool()), (Some(true), Some(true)));

    // Chrome's first navigation waits for these answers, before `tab.open` returns.
    let paused = |n: u32, url: String, session: Option<&str>| {
        let mut params = json!({"tab": 7, "requestId": format!("interception-{n}"), "url": url});
        if let Some(s) = session {
            params["session"] = json!(s);
        }
        json!({"type":"event","event":"requestPaused","params":params})
    };
    let cases = [
        (format!("http://127.0.0.1:{allowed}/jobs"), None, true),
        ("http://10.0.0.1/admin".to_string(), None, false),
        (format!("http://127.0.0.1:{}/", allowed + 1), Some("S1"), false),
        ("data:image/png;base64,AAAA".to_string(), Some("S2"), true),
        (format!("https://127.0.0.1:{allowed}/app.js"), Some("S3"), true),
    ];
    for (n, (url, session, ok)) in cases.iter().enumerate() {
        ext.send(paused(n as u32, url.clone(), *session)).await;
        let (method, id, params) = ext.call().await;
        assert_eq!(method, "cdp.send", "{url}");
        assert_eq!(params["tab"], 7);
        assert_eq!(params["session"].as_str(), *session, "{url}");
        assert_eq!(params["params"]["requestId"], format!("interception-{n}"));
        if *ok {
            assert_eq!(params["method"], "Fetch.continueRequest", "{url}");
        } else {
            assert_eq!(params["method"], "Fetch.failRequest", "{url}");
            assert_eq!(params["params"]["errorReason"], "BlockedByClient");
        }
        ext.answer(id, json!({})).await;
    }
    ext.answer(open_id, json!({"tab": 7, "window": 1})).await;
    let tab = open.await.unwrap().unwrap();
    assert_eq!(tab.blocked(), 2, "the refusals before tab.open returned count for the tab");

    // After the open, and for a tab that is not ours.
    ext.send(paused(10, "http://192.168.1.1/".into(), None)).await;
    let (_, id, params) = ext.call().await;
    assert_eq!(params["method"], "Fetch.failRequest");
    ext.answer(id, json!({})).await;
    assert_eq!(tab.blocked(), 3);
    ext.send(json!({"type":"event","event":"requestPaused","params":{"tab": 99, "requestId": "x", "url": "https://example.com/"}})).await;
    let (_, id, params) = ext.call().await;
    assert_eq!((params["tab"].as_i64(), params["method"].as_str()), (Some(99), Some("Fetch.failRequest")));
    ext.answer(id, json!({})).await;

    // goto checks the policy first too.
    assert!(tab.goto("http://10.1.2.3/").await.is_err());
}

#[test]
fn browser_labels() {
    assert_eq!(browser_label("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36"), "Chrome 153");
    assert_eq!(browser_label("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) HeadlessChrome/153.0.0.0 Safari/537.36"), "Chrome 153");
    assert_eq!(browser_label("Mozilla/5.0 ... Chrome/130.0.0.0 Safari/537.36 Edg/130.0.2849.68"), "Microsoft Edge 130");
    assert_eq!(browser_label("Something"), "Something");
}

#[tokio::test]
async fn a_taken_port_leaves_the_bridge_off() {
    let taken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = taken.local_addr().unwrap().port();
    let bridge = Bridge::new(Secrets::memory());
    let err = bridge.start_with(BridgeConfig { port, auto_approve: false, allowed_origin: origin() }).await.unwrap_err();
    assert!(err.to_string().contains("in use"), "{err}");
    let status = bridge.status();
    assert!(!status.listening && status.port == port, "{status:?}");
}
