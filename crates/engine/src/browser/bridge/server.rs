//! The WebSocket server the extension connects to: the upgrade check (path and `Origin`), the
//! handshake (both sides prove they hold the token, which never travels again) and pairing (the user
//! approves the code in the app and in the extension), call answers, and the events (the guard's
//! paused requests are answered here).

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::handshake::server::{Callback, ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::Message;

use super::pairing::{self, TOKEN_KEY};
use super::protocol::{reason, Event, Incoming, Nonce, Outgoing, Proof, Token, PROTOCOL};
use super::{Bridge, BridgeConfig, BridgeError, BridgeEvent, Out, Pairing, Peer, Shared, TabShared, PATH};
use crate::browser::guard::{allows, Policy};

/// How long a new connection has to say hello (and then to answer the challenge).
const HELLO_WAIT: Duration = Duration::from_secs(10);
/// Largest message (a full-page screenshot, a file to upload).
const MAX_MESSAGE: usize = 64 * 1024 * 1024;
/// How long the answer to a paused request may take.
const GUARD_ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// Hosts remembered per tab by the guard.
const HOST_CACHE_MAX: usize = 1024;

/// Bind `127.0.0.1:<config.port>` and serve in the background. Returns the port.
pub(super) async fn serve(bridge: Bridge, config: BridgeConfig) -> Result<u16> {
    let shared = bridge.shared.clone();
    let listener = match TcpListener::bind(("127.0.0.1", config.port)).await {
        Ok(listener) => listener,
        Err(e) => {
            {
                let mut state = shared.state();
                state.listening = false;
                state.port = config.port;
            }
            shared.emit(BridgeEvent::StatusChanged);
            return Err(e).with_context(|| format!("port {} on 127.0.0.1 is in use", config.port));
        }
    };
    let port = listener.local_addr().context("the browser bridge has no address")?.port();
    // Whether some browser was paired before (read once; approve/forget keep it current).
    let paired = match shared.secrets.get(TOKEN_KEY) {
        Ok(token) => token.is_some_and(|t| !t.is_empty()),
        Err(e) => {
            tracing::warn!("browser bridge: reading the pairing token: {e:#}");
            false
        }
    };
    {
        let mut state = shared.state();
        state.listening = true;
        state.port = port;
        state.auto_approve = config.auto_approve;
        state.paired = paired;
    }
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("browser bridge listening on 127.0.0.1:{port}");

    let origin = Arc::new(config.allowed_origin);
    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let (bridge, origin) = (bridge.clone(), origin.clone());
                    tokio::spawn(async move {
                        if let Err(e) = connection(bridge, stream, &origin).await {
                            tracing::debug!("browser bridge connection: {e:#}");
                        }
                    });
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    });
    Ok(port)
}

/// The upgrade check: accepted for `PATH` from `origin` only; anything else is a 403.
struct Upgrade<'a> {
    origin: &'a str,
}

impl Callback for Upgrade<'_> {
    fn on_request(self, req: &Request, resp: Response) -> Result<Response, ErrorResponse> {
        let from = req.headers().get("origin").and_then(|v| v.to_str().ok());
        if req.uri().path() == PATH && from == Some(self.origin) {
            return Ok(resp);
        }
        let mut refused = ErrorResponse::new(Some("forbidden".into()));
        *refused.status_mut() = StatusCode::FORBIDDEN;
        Err(refused)
    }
}

async fn connection(bridge: Bridge, stream: TcpStream, origin: &str) -> Result<()> {
    let config = WebSocketConfig::default().max_message_size(Some(MAX_MESSAGE)).max_frame_size(Some(MAX_MESSAGE));
    let ws = tokio_tungstenite::accept_hdr_async_with_config(stream, Upgrade { origin }, Some(config)).await.context("the WebSocket upgrade failed")?;
    let (mut sink, mut incoming) = ws.split();

    // The writer: one task owns the sink; `Out::Close` (or the end of the channel) ends the connection.
    let (tx, mut rx) = mpsc::unbounded_channel::<Out>();
    let (kill_tx, mut kill_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        while let Some(out) = rx.recv().await {
            match out {
                Out::Msg(msg) => {
                    let Ok(text) = serde_json::to_string(&msg) else { continue };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Out::Close => break,
            }
        }
        let _ = sink.send(Message::Close(None)).await;
        let _ = sink.close().await;
        drop(kill_tx);
    });

    let shared = bridge.shared.clone();
    let conn = shared.next_id();
    let hello = tokio::select! {
        hello = tokio::time::timeout(HELLO_WAIT, first(&mut incoming, &tx, |m| match m {
            Incoming::Hello(hello) => Some(hello),
            _ => None,
        })) => hello.ok().flatten(),
        _ = &mut kill_rx => None,
    };
    let Some(hello) = hello else {
        let _ = tx.send(Out::Close);
        return Ok(());
    };
    let peer = Peer { conn, tx: tx.clone(), hello };
    if peer.hello.protocol != PROTOCOL {
        tracing::warn!("browser bridge: the extension speaks protocol {}, not {PROTOCOL}", peer.hello.protocol);
        peer.send(Outgoing::Rejected { reason: reason::PROTOCOL.into() });
        peer.close();
        return Ok(());
    }

    // A paired extension and a token here: each side proves it holds the token (PROTOCOL.md).
    match challenge(&shared, &peer) {
        None => start_pairing(&shared, peer),
        Some((token, app_nonce)) => {
            let proof = tokio::select! {
                proof = tokio::time::timeout(HELLO_WAIT, first(&mut incoming, &tx, |m| match m {
                    Incoming::Auth { proof } => Some(proof),
                    _ => None,
                })) => proof.ok().flatten(),
                _ = &mut kill_rx => None,
            };
            let Some(Proof(proof)) = proof else {
                let _ = tx.send(Out::Close);
                return Ok(());
            };
            if pairing::verify_ext_proof(&token, &app_nonce, &proof) {
                welcome(&shared, peer);
            } else {
                // Not the token the app holds (a stale one, or not the extension at all).
                tracing::info!("browser bridge: the extension's proof did not match (connection {conn})");
                start_pairing(&shared, peer);
            }
        }
    }

    loop {
        let frame = tokio::select! {
            frame = incoming.next() => frame,
            _ = &mut kill_rx => None,
        };
        let text = match frame {
            Some(Ok(Message::Text(text))) => text,
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
            Some(Ok(_)) => continue,
        };
        let msg = match serde_json::from_str::<Incoming>(&text) {
            Ok(msg) => msg,
            Err(e) => {
                tracing::debug!("browser bridge: an unreadable message ({:?})", e.classify());
                continue;
            }
        };
        match msg {
            Incoming::Ping => {
                let _ = tx.send(Out::Msg(Outgoing::Pong));
            }
            Incoming::CallResult { id, result } => shared.state().answer(conn, id, Ok(result)),
            Incoming::CallError { id, error } => shared.state().answer(conn, id, Err(BridgeError::from_code(&error.code, &error.message))),
            Incoming::Event { event, params } => {
                let live = shared.state().live.as_ref().is_some_and(|p| p.conn == conn);
                if live {
                    if let Some(event) = Event::parse(&event, &params) {
                        on_event(&bridge, event);
                    }
                }
            }
            Incoming::Consent { code } => consent(&shared, conn, &code),
            Incoming::Hello(_) | Incoming::Auth { .. } | Incoming::Unknown => {}
        }
    }
    gone(&shared, conn);
    let _ = tx.send(Out::Close);
    Ok(())
}

/// Wait for the first message `pick` takes (answering pings meanwhile, ignoring anything else).
/// `None` when the connection ends first.
async fn first<S, T>(incoming: &mut S, tx: &mpsc::UnboundedSender<Out>, pick: impl Fn(Incoming) -> Option<T>) -> Option<T>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    while let Some(frame) = incoming.next().await {
        let text = match frame {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => continue,
        };
        match serde_json::from_str::<Incoming>(&text) {
            Ok(Incoming::Ping) => {
                let _ = tx.send(Out::Msg(Outgoing::Pong));
            }
            Ok(msg) => {
                if let Some(picked) = pick(msg) {
                    return Some(picked);
                }
            }
            Err(_) => {}
        }
    }
    None
}

/// When the extension says it is paired and this app holds a token: send `challenge` (the app's
/// proof for the extension's nonce, and the app's own nonce) and return the token and that nonce
/// to check the answer. `None`: the connection has to pair.
fn challenge(shared: &Arc<Shared>, peer: &Peer) -> Option<(String, String)> {
    let ext_nonce = peer.hello.nonce.as_ref().map(|n| n.0.as_str()).filter(|n| pairing::is_nonce(n));
    let (true, Some(ext_nonce)) = (peer.hello.paired, ext_nonce) else { return None };
    let token = match shared.secrets.get(TOKEN_KEY) {
        Ok(stored) => stored.filter(|t| !t.is_empty())?,
        Err(e) => {
            tracing::warn!("browser bridge: reading the pairing token: {e:#}");
            return None;
        }
    };
    let app_nonce = pairing::new_nonce();
    peer.send(Outgoing::Challenge { proof: Proof(pairing::app_proof(&token, ext_nonce)), nonce: Nonce(app_nonce.clone()) });
    Some((token, app_nonce))
}

/// `peer` is in: it becomes the one live connection (an older one is closed).
fn welcome(shared: &Arc<Shared>, peer: Peer) {
    let conn = peer.conn;
    let version = peer.hello.extension.clone();
    // Sent before the connection is visible to callers, so `welcome` comes before any call.
    peer.send(Outgoing::Welcome);
    {
        let mut state = shared.state();
        if state.pairing.as_ref().is_some_and(|p| p.peer.conn == conn) {
            state.pairing = None;
        }
        if let Some(old) = state.live.replace(peer) {
            old.close();
            state.fail_calls_of(old.conn);
        }
    }
    shared.connected.send_replace(true);
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("browser extension {version} connected (connection {conn})");
}

fn start_pairing(shared: &Arc<Shared>, peer: Peer) {
    let conn = peer.conn;
    let code = pairing::new_code();
    peer.send(Outgoing::Pair { code: code.clone() });
    {
        let mut state = shared.state();
        // Tests only: the app side approves at once (the extension still has to consent).
        let approved = state.auto_approve;
        if let Some(old) = state.pairing.replace(Pairing { peer, code, approved, consented: false }) {
            old.peer.close();
        }
    }
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("browser extension asks to be paired (connection {conn})");
    let shared = shared.clone();
    tokio::spawn(async move {
        tokio::time::sleep(pairing::PAIRING_TTL).await;
        let expired = {
            let mut state = shared.state();
            match &state.pairing {
                Some(p) if p.peer.conn == conn => state.pairing.take(),
                _ => None,
            }
        };
        if let Some(expired) = expired {
            expired.peer.send(Outgoing::Rejected { reason: reason::TIMEOUT.into() });
            expired.peer.close();
            shared.emit(BridgeEvent::StatusChanged);
            tracing::info!("browser pairing timed out (connection {conn})");
        }
    });
}

/// The user approved the code in the app. Pairs at once when the extension already consented.
pub(super) fn approve(shared: &Arc<Shared>) -> Result<()> {
    let ready = {
        let mut state = shared.state();
        let waiting = state.pairing.as_mut().ok_or_else(|| anyhow!("no browser is waiting to be paired"))?;
        waiting.approved = true;
        waiting.consented
    };
    shared.emit(BridgeEvent::StatusChanged);
    if ready {
        return finish_pairing(shared);
    }
    tracing::info!("browser pairing approved in the app; waiting for the extension");
    Ok(())
}

/// The user pressed Pair in the extension's popup for `code` (on connection `conn`). Pairs at once
/// when the app side already approved.
fn consent(shared: &Arc<Shared>, conn: u64, code: &str) {
    let ready = {
        let mut state = shared.state();
        match state.pairing.as_mut() {
            Some(waiting) if waiting.peer.conn == conn && pairing::ct_eq(waiting.code.as_bytes(), code.as_bytes()) => {
                waiting.consented = true;
                waiting.approved
            }
            _ => {
                tracing::info!("browser bridge: a consent for no pairing of this connection (connection {conn})");
                return;
            }
        }
    };
    shared.emit(BridgeEvent::StatusChanged);
    if ready {
        if let Err(e) = finish_pairing(shared) {
            tracing::warn!("browser bridge: pairing: {e:#}");
        }
    } else {
        tracing::info!("browser pairing approved in the extension; waiting for the app (connection {conn})");
    }
}

/// Both sides approved: make the token, give it to the extension (its only trip) and let it in.
fn finish_pairing(shared: &Arc<Shared>) -> Result<()> {
    let waiting = {
        let mut state = shared.state();
        match &state.pairing {
            Some(p) if p.approved && p.consented => state.pairing.take(),
            _ => None,
        }
    };
    let Some(mut waiting) = waiting else { return Ok(()) };
    let token = pairing::new_token();
    if let Err(e) = shared.secrets.set(TOKEN_KEY, &token) {
        // Still waiting: the user can approve again.
        waiting.approved = false;
        shared.state().pairing.get_or_insert(waiting);
        shared.emit(BridgeEvent::StatusChanged);
        return Err(e.context("saving the browser pairing"));
    }
    shared.state().paired = true;
    let conn = waiting.peer.conn;
    waiting.peer.send(Outgoing::Paired { token: Token(token) });
    welcome(shared, waiting.peer);
    tracing::info!("browser paired (connection {conn})");
    Ok(())
}

pub(super) fn decline(shared: &Arc<Shared>) -> Result<()> {
    let waiting = shared.state().pairing.take().ok_or_else(|| anyhow!("no browser is waiting to be paired"))?;
    waiting.peer.send(Outgoing::Rejected { reason: reason::DECLINED.into() });
    waiting.peer.close();
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("browser pairing declined (connection {})", waiting.peer.conn);
    Ok(())
}

pub(super) fn forget(shared: &Arc<Shared>) -> Result<()> {
    shared.secrets.delete(TOKEN_KEY).context("forgetting the paired browser")?;
    let (live, tabs) = {
        let mut state = shared.state();
        state.paired = false;
        let live = state.live.take();
        if let Some(peer) = &live {
            state.fail_calls_of(peer.conn);
        }
        (live, state.tabs.drain().collect::<Vec<_>>())
    };
    if let Some(peer) = live {
        // The extension closes the tabs it opened and pairs again.
        peer.send(Outgoing::Forget);
        peer.close();
        shared.connected.send_replace(false);
    }
    for (id, tab) in tabs {
        tab.mark_closed();
        shared.emit(BridgeEvent::TabClosed(id));
    }
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("the paired browser was forgotten");
    Ok(())
}

/// Connection `conn` ended.
fn gone(shared: &Arc<Shared>, conn: u64) {
    let changed = {
        let mut state = shared.state();
        let mut changed = false;
        if state.live.as_ref().is_some_and(|p| p.conn == conn) {
            state.live = None;
            changed = true;
            shared.connected.send_replace(false);
        }
        if state.pairing.as_ref().is_some_and(|p| p.peer.conn == conn) {
            state.pairing = None;
            changed = true;
        }
        state.fail_calls_of(conn);
        changed
    };
    if changed {
        shared.emit(BridgeEvent::StatusChanged);
        tracing::info!("browser extension disconnected (connection {conn})");
    }
}

fn on_event(bridge: &Bridge, event: Event) {
    let shared = &bridge.shared;
    match event {
        Event::TabClosed { tab } => {
            // A tab this app never saw (a failed `tab.open` closes its tab again) is ignored.
            let known = shared.state().tabs.remove(&tab);
            if let Some(known) = known {
                known.mark_closed();
                shared.emit(BridgeEvent::TabClosed(tab));
            }
        }
        Event::Detached { tab, reason } => shared.emit(BridgeEvent::Detached { tab, reason }),
        Event::Dialog { tab, kind, message } => shared.emit(BridgeEvent::Dialog { tab, kind, message }),
        Event::RequestPaused { tab, session, request_id, url } => {
            let bridge = bridge.clone();
            tokio::spawn(async move { answer_paused(&bridge, tab, session, request_id, &url).await });
        }
    }
}

/// The guard: let a guarded tab's paused request go on, or fail it.
async fn answer_paused(bridge: &Bridge, tab: i64, session: Option<String>, request_id: String, url: &str) {
    let known = {
        let mut state = bridge.shared.state();
        match state.tabs.get(&tab) {
            Some(known) => Some(known.clone()),
            // A guarded `tab.open` is still in flight (Chrome's first navigation waits for this
            // answer): the tab is that one.
            None if !state.guarded_opens.is_empty() => {
                let mut policies = state.guarded_opens.values();
                let first = policies.next().cloned().unwrap_or(Policy::Public);
                let policy = if policies.all(|p| *p == first) { first } else { Policy::Public };
                let known = TabShared::new(Some(policy));
                state.tabs.insert(tab, known.clone());
                Some(known)
            }
            None => None,
        }
    };
    let allowed = match &known {
        // Not a tab this app opened (or from before a restart): refuse.
        None => false,
        Some(known) => {
            let policy = known.guard.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match policy {
                None => true,
                Some(policy) => decide(known, &policy, url).await,
            }
        }
    };
    let params = if allowed {
        json!({ "requestId": request_id })
    } else {
        if let Some(known) = &known {
            known.blocked.fetch_add(1, Ordering::SeqCst);
        }
        tracing::info!("browser guard: a request to a private address was refused (tab {tab})");
        json!({ "requestId": request_id, "errorReason": "BlockedByClient" })
    };
    let method = if allowed { "Fetch.continueRequest" } else { "Fetch.failRequest" };
    let mut call = json!({ "tab": tab, "method": method, "params": params });
    if let (Some(session), Value::Object(map)) = (session, &mut call) {
        map.insert("session".into(), Value::String(session));
    }
    if let Err(e) = bridge.call_timeout("cdp.send", call, GUARD_ANSWER_TIMEOUT).await {
        tracing::debug!("browser guard: answering a paused request: {e}");
    }
}

/// May `url` be requested by a tab guarded by `policy`? (Remembered per host.)
async fn decide(tab: &TabShared, policy: &Policy, url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else { return false };
    let key = match (url.host_str(), url.port_or_known_default()) {
        (Some(host), Some(port)) => format!("{}://{host}:{port}", url.scheme()),
        // No host to check (data:, blob:, ...): `allows` decides by the scheme.
        _ => return allows(policy, &url).await,
    };
    if let Some(known) = tab.hosts.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return *known;
    }
    let allowed = allows(policy, &url).await;
    let mut hosts = tab.hosts.lock().unwrap_or_else(|e| e.into_inner());
    if hosts.len() >= HOST_CACHE_MAX {
        hosts.clear();
    }
    hosts.insert(key, allowed);
    allowed
}
