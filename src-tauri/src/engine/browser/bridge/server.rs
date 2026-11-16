//! The WebSocket server the extension connects to: the upgrade check (path and `Origin`), the
//! handshake and pairing, call answers, and the events (the guard's paused requests are answered
//! here).

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
use super::protocol::{reason, Event, Hello, Incoming, Outgoing, Token, PROTOCOL};
use super::{Bridge, BridgeConfig, BridgeError, BridgeEvent, Out, Pairing, Peer, Shared, TabShared, PATH};
use crate::engine::browser::guard::{allows, Policy};

/// How long a new connection has to say hello.
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
    let config = WebSocketConfig { max_message_size: Some(MAX_MESSAGE), max_frame_size: Some(MAX_MESSAGE), ..Default::default() };
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
                    if sink.send(Message::Text(text)).await.is_err() {
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
        hello = tokio::time::timeout(HELLO_WAIT, first_hello(&mut incoming, &tx)) => hello.ok().flatten(),
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
    admit(&shared, peer);

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
            Incoming::Hello(_) | Incoming::Unknown => {}
        }
    }
    gone(&shared, conn);
    let _ = tx.send(Out::Close);
    Ok(())
}

/// Wait for the `hello` (answering pings meanwhile). `None` when the connection ends first.
async fn first_hello<S>(incoming: &mut S, tx: &mpsc::UnboundedSender<Out>) -> Option<Hello>
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
            Ok(Incoming::Hello(hello)) => return Some(hello),
            Ok(Incoming::Ping) => {
                let _ = tx.send(Out::Msg(Outgoing::Pong));
            }
            _ => {}
        }
    }
    None
}

/// Let `peer` in when its token matches, else ask the user to pair it.
fn admit(shared: &Arc<Shared>, peer: Peer) {
    let stored = match shared.secrets.get(TOKEN_KEY) {
        Ok(stored) => stored,
        Err(e) => {
            tracing::warn!("browser bridge: reading the pairing token: {e:#}");
            None
        }
    };
    let offered = peer.hello.token.as_ref().map(|t| t.0.as_str());
    if pairing::matches(stored.as_deref(), offered) {
        welcome(shared, peer);
    } else {
        start_pairing(shared, peer);
    }
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
    let auto_approve = {
        let mut state = shared.state();
        if let Some(old) = state.pairing.replace(Pairing { peer, code }) {
            old.peer.close();
        }
        state.auto_approve
    };
    shared.emit(BridgeEvent::StatusChanged);
    tracing::info!("browser extension asks to be paired (connection {conn})");
    if auto_approve {
        if let Err(e) = approve(shared) {
            tracing::warn!("browser bridge: approving the pairing: {e:#}");
        }
        return;
    }
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

pub(super) fn approve(shared: &Arc<Shared>) -> Result<()> {
    let waiting = shared.state().pairing.take().ok_or_else(|| anyhow!("no browser is waiting to be paired"))?;
    let token = pairing::new_token();
    if let Err(e) = shared.secrets.set(TOKEN_KEY, &token) {
        // Still waiting: the user can try again.
        shared.state().pairing.get_or_insert(waiting);
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
