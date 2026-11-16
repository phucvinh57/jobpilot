//! The browser bridge: JobPilot drives tabs in the user's own Chrome through the JobPilot extension.
//!
//! The app is a WebSocket server on `127.0.0.1:<port>/jobpilot` (default 47615, `JOBPILOT_BRIDGE_PORT`);
//! the extension's service worker is its only client. The contract is `extension/PROTOCOL.md`:
//!
//! - [`protocol`]: the messages;
//! - [`pairing`]: the token in the keyring, the 6-digit code the user approves;
//! - `server`: the socket, the handshake and the events (it also answers the guard's paused requests).
//!
//! Get the [`Bridge`] with [`Core::browser`] (one per data dir, cheap to clone), start it once with
//! [`Bridge::start`], then open tabs with [`Bridge::open`] (see [`super::tab`]):
//!
//! ```text
//! let bridge = core.browser();
//! let tab = bridge.open("https://example.com/jobs", OpenOptions { profile: Profile::Private, focus: false, guard: Some(Policy::Public) }).await?;
//! tab.wait_loaded(Duration::from_secs(20)).await;
//! let html = tab.content().await?;
//! tab.close().await;
//! ```
//!
//! A failed call is a [`BridgeError`] inside the `anyhow::Error` (`e.downcast_ref::<BridgeError>()`).

pub mod pairing;
pub mod protocol;
mod server;

#[cfg(test)]
mod chromium_tests;
#[cfg(test)]
mod tests;
/// A real Chromium with the extension for tests (also for `tests/` with the `test-browser` feature).
#[cfg(any(test, feature = "test-browser"))]
#[doc(hidden)]
pub mod testkit;

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

use self::protocol::{Hello, Outgoing};
use super::guard::Policy;
use crate::engine::{Core, Secrets};

/// Part of [`BridgeError::NotConnected`]'s text (`health::classify` keys on it).
pub const NOT_CONNECTED: &str = "the JobPilot browser extension is not connected";
/// Part of [`BridgeError::IncognitoNotAllowed`]'s text (`health::classify` keys on it).
pub const INCOGNITO_NOT_ALLOWED: &str = "the JobPilot extension is not allowed in Incognito";

/// The port the extension looks for by default.
pub const DEFAULT_PORT: u16 = 47615;
/// Env var: another port (the extension must be told the same one in its popup).
pub const PORT_ENV: &str = "JOBPILOT_BRIDGE_PORT";
/// The extension's id (fixed by the public key in its manifest).
pub const EXTENSION_ID: &str = "gghdiehhgfalihbgeafcjegccbbinodl";
/// The WebSocket path.
pub const PATH: &str = "/jobpilot";

/// How long a call waits for an answer unless told otherwise.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a call waits for the extension to (re)connect before failing with `NotConnected`.
pub const CONNECT_WAIT: Duration = Duration::from_secs(3);

/// Why a call to the browser failed. Rides inside `anyhow::Error`; the text is shown to the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeError {
    /// No paired extension is connected.
    NotConnected,
    /// The tab was closed (or is not one JobPilot opened).
    NotOwned,
    /// The extension refused a browser command JobPilot asked for.
    NotAllowed,
    /// A private (Incognito) tab was asked for, and the extension may not run in Incognito.
    IncognitoNotAllowed,
    /// Chrome refused the command; Chrome's text.
    Cdp(String),
    /// No answer in time.
    Timeout,
    /// Anything else (bad parameters, an error inside the extension); its text.
    Other(String),
}

impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BridgeError::NotConnected => {
                write!(f, "{NOT_CONNECTED}: open Chrome with the JobPilot extension and pair it in Settings")
            }
            BridgeError::NotOwned => f.write_str("the browser tab was closed"),
            BridgeError::NotAllowed => f.write_str("the JobPilot extension refused a browser command (update the extension)"),
            BridgeError::IncognitoNotAllowed => write!(
                f,
                "{INCOGNITO_NOT_ALLOWED}: turn on \"Allow in Incognito\" for it on the chrome://extensions page"
            ),
            BridgeError::Cdp(message) => write!(f, "the browser refused: {message}"),
            BridgeError::Timeout => f.write_str("the browser did not answer in time"),
            BridgeError::Other(message) => write!(f, "the browser extension failed: {message}"),
        }
    }
}

impl std::error::Error for BridgeError {}

impl BridgeError {
    /// The error of a protocol `error` answer.
    pub fn from_code(code: &str, message: &str) -> Self {
        use protocol::code;
        match code {
            code::NOT_OWNED => BridgeError::NotOwned,
            code::NOT_ALLOWED => BridgeError::NotAllowed,
            code::INCOGNITO_NOT_ALLOWED => BridgeError::IncognitoNotAllowed,
            code::CDP => BridgeError::Cdp(message.to_string()),
            _ => BridgeError::Other(if message.is_empty() { code.to_string() } else { message.to_string() }),
        }
    }
}

/// What the Settings page shows about the browser.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BridgeStatus {
    /// The server is listening (false when the port was taken by something else).
    pub listening: bool,
    pub port: u16,
    /// A paired extension is connected.
    pub connected: bool,
    /// The connected browser ("Chrome 153"), also while pairing.
    pub browser: Option<String>,
    pub extension_version: Option<String>,
    /// The extension may open Incognito tabs (`Profile::Private`).
    pub incognito_allowed: bool,
    /// The code to approve, while an extension waits for pairing.
    pub pairing_code: Option<String>,
    /// A token is stored (some browser was paired).
    pub paired: bool,
}

/// What happened in the browser.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BridgeEvent {
    /// [`Bridge::status`] changed.
    StatusChanged,
    /// A tab JobPilot opened was closed (by the user or by JobPilot).
    TabClosed(i64),
    /// Chrome stopped letting JobPilot drive the tab (the user pressed Cancel on the debugging bar).
    Detached { tab: i64, reason: String },
    /// A page opened a JavaScript dialog (`alert`, `confirm`, `prompt`, `beforeunload`).
    Dialog { tab: i64, kind: String, message: String },
}

/// How the server runs (tests use an ephemeral port, auto-approval and their own origin).
#[doc(hidden)]
#[derive(Clone, Debug)]
pub struct BridgeConfig {
    /// 0 = any free port.
    pub port: u16,
    /// Approve every pairing request at once (tests only).
    pub auto_approve: bool,
    /// The only `Origin` an upgrade is accepted from.
    pub allowed_origin: String,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        Self { port: port_from_env(), auto_approve: false, allowed_origin: format!("chrome-extension://{EXTENSION_ID}") }
    }
}

fn port_from_env() -> u16 {
    std::env::var(PORT_ENV).ok().and_then(|p| p.trim().parse().ok()).unwrap_or(DEFAULT_PORT)
}

/// The browser bridge. Cheap to clone; every clone is the same bridge.
#[derive(Clone)]
pub struct Bridge {
    pub(crate) shared: Arc<Shared>,
}

/// One frame for a connection's writer.
pub(crate) enum Out {
    Msg(Outgoing),
    /// Send a close frame and end the connection.
    Close,
}

/// A connected extension (before or after it was let in).
#[derive(Clone)]
pub(crate) struct Peer {
    pub(crate) conn: u64,
    pub(crate) tx: mpsc::UnboundedSender<Out>,
    pub(crate) hello: Hello,
}

impl Peer {
    pub(crate) fn send(&self, msg: Outgoing) {
        let _ = self.tx.send(Out::Msg(msg));
    }

    pub(crate) fn close(&self) {
        let _ = self.tx.send(Out::Close);
    }
}

pub(crate) struct Pairing {
    pub(crate) peer: Peer,
    pub(crate) code: String,
}

/// A call waiting for its answer.
struct Pending {
    conn: u64,
    tx: oneshot::Sender<Result<Value, BridgeError>>,
}

/// What the app knows about one tab it opened.
pub(crate) struct TabShared {
    pub(crate) closed: watch::Sender<bool>,
    pub(crate) guard: Mutex<Option<Policy>>,
    pub(crate) blocked: AtomicUsize,
    /// `host:port` -> allowed, for the guard.
    pub(crate) hosts: Mutex<HashMap<String, bool>>,
}

impl TabShared {
    pub(crate) fn new(guard: Option<Policy>) -> Arc<Self> {
        Arc::new(Self { closed: watch::channel(false).0, guard: Mutex::new(guard), blocked: AtomicUsize::new(0), hosts: Mutex::default() })
    }

    pub(crate) fn mark_closed(&self) {
        self.closed.send_replace(true);
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) listening: bool,
    pub(crate) port: u16,
    pub(crate) auto_approve: bool,
    pub(crate) paired: bool,
    /// The paired, welcomed connection.
    pub(crate) live: Option<Peer>,
    /// The connection waiting for the user's approval.
    pub(crate) pairing: Option<Pairing>,
    calls: HashMap<u64, Pending>,
    pub(crate) tabs: HashMap<i64, Arc<TabShared>>,
    /// Guarded `tab.open` calls in flight, by call id: their tab is not known yet, but the guard
    /// must already answer its requests (Chrome's navigation waits for them).
    pub(crate) guarded_opens: HashMap<u64, Policy>,
}

pub(crate) struct Shared {
    pub(crate) secrets: Secrets,
    state: Mutex<State>,
    pub(crate) events: broadcast::Sender<BridgeEvent>,
    /// True while a paired extension is connected.
    pub(crate) connected: watch::Sender<bool>,
    next_id: AtomicU64,
    started: AtomicBool,
}

impl Shared {
    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub(crate) fn emit(&self, event: BridgeEvent) {
        let _ = self.events.send(event);
    }
}

/// One bridge per data dir, so every `core.browser()` talks to the one server.
static BRIDGES: OnceLock<Mutex<HashMap<PathBuf, Bridge>>> = OnceLock::new();

impl Core {
    /// The browser bridge (cheap to clone; share it with `runtime::spawn`).
    pub fn browser(&self) -> Bridge {
        let mut map = BRIDGES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.entry(self.paths.data_dir.clone()).or_insert_with(|| Bridge::new(self.secrets.clone())).clone()
    }
}

impl Bridge {
    /// A bridge that keeps its token in `secrets`. Not started.
    pub(crate) fn new(secrets: Secrets) -> Self {
        let shared = Shared {
            secrets,
            state: Mutex::default(),
            events: broadcast::channel(256).0,
            connected: watch::channel(false).0,
            next_id: AtomicU64::new(1),
            started: AtomicBool::new(false),
        };
        Self { shared: Arc::new(shared) }
    }

    /// Start the server on the engine runtime (port from `JOBPILOT_BRIDGE_PORT`, default 47615).
    /// Idempotent. When the port is taken the app runs without the bridge: [`status`](Self::status)
    /// says `listening: false`.
    pub fn start(&self) {
        let this = self.clone();
        crate::engine::runtime::handle().spawn(async move {
            // A second start (or one after `start_with`) is a no-op.
            if let Err(e) = this.start_with(BridgeConfig::default()).await {
                tracing::warn!("the browser bridge is off: {e:#}");
            }
        });
    }

    /// Start the server with `config` on the current runtime; returns the port it listens on.
    /// A bridge that was already started returns its port.
    #[doc(hidden)]
    pub async fn start_with(&self, config: BridgeConfig) -> anyhow::Result<u16> {
        if self.shared.started.swap(true, Ordering::SeqCst) {
            return Ok(self.shared.state().port);
        }
        let served = server::serve(self.clone(), config).await;
        if served.is_err() {
            // A later start may try again.
            self.shared.started.store(false, Ordering::SeqCst);
        }
        served
    }

    /// What the Settings page shows.
    pub fn status(&self) -> BridgeStatus {
        let state = self.shared.state();
        let peer = state.live.as_ref().or(state.pairing.as_ref().map(|p| &p.peer));
        BridgeStatus {
            listening: state.listening,
            port: state.port,
            connected: state.live.is_some(),
            browser: peer.map(|p| browser_label(&p.hello.browser)),
            extension_version: peer.map(|p| p.hello.extension.clone()).filter(|v| !v.is_empty()),
            incognito_allowed: peer.is_some_and(|p| p.hello.incognito_allowed),
            pairing_code: state.pairing.as_ref().map(|p| p.code.clone()),
            paired: state.paired,
        }
    }

    /// Status changes and browser events from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<BridgeEvent> {
        self.shared.events.subscribe()
    }

    /// The user approved the code: give the waiting extension a token and let it in.
    pub fn approve_pairing(&self) -> anyhow::Result<()> {
        server::approve(&self.shared)
    }

    /// The user declined the code: the extension is told so and disconnected.
    pub fn decline_pairing(&self) -> anyhow::Result<()> {
        server::decline(&self.shared)
    }

    /// Forget the paired browser: the token is deleted, the extension is told to forget it (it
    /// closes the tabs JobPilot opened) and has to pair again.
    pub fn forget(&self) -> anyhow::Result<()> {
        server::forget(&self.shared)
    }

    /// Close every tab JobPilot opened (the app is exiting). Best effort, a few seconds at most,
    /// and nothing at all when no extension is connected.
    pub async fn close_all(&self) {
        if self.shared.state().live.is_some() {
            if let Err(e) = self.call_timeout("tab.closeAll", serde_json::json!({}), Duration::from_secs(3)).await {
                tracing::warn!("closing the browser tabs: {e}");
            }
        }
        let tabs: Vec<_> = self.shared.state().tabs.drain().collect();
        for (id, tab) in tabs {
            tab.mark_closed();
            self.shared.emit(BridgeEvent::TabClosed(id));
        }
    }

    /// Call `method` of the extension (see `extension/PROTOCOL.md`), waiting [`CALL_TIMEOUT`].
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        self.call_timeout(method, params, CALL_TIMEOUT).await
    }

    /// Call `method`, waiting at most `timeout` for the answer (plus up to [`CONNECT_WAIT`] for the
    /// extension to connect).
    pub async fn call_timeout(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, BridgeError> {
        let id = self.shared.next_id();
        self.call_id(id, method, params, timeout).await
    }

    /// [`call_timeout`](Self::call_timeout) with an id chosen by the caller (`tab.open` registers
    /// its id first).
    pub(crate) async fn call_id(&self, id: u64, method: &str, params: Value, timeout: Duration) -> Result<Value, BridgeError> {
        let peer = self.live_peer().await?;
        let (tx, rx) = oneshot::channel();
        self.shared.state().calls.insert(id, Pending { conn: peer.conn, tx });
        if peer.tx.send(Out::Msg(Outgoing::Call { id, method: method.to_string(), params })).is_err() {
            self.shared.state().calls.remove(&id);
            return Err(BridgeError::NotConnected);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(answer)) => answer,
            // The connection went away before the answer.
            Ok(Err(_)) => Err(BridgeError::NotConnected),
            Err(_) => {
                self.shared.state().calls.remove(&id);
                Err(BridgeError::Timeout)
            }
        }
    }

    /// The live connection, waiting up to [`CONNECT_WAIT`] for one.
    async fn live_peer(&self) -> Result<Peer, BridgeError> {
        let mut connected = self.shared.connected.subscribe();
        let wait = tokio::time::timeout(CONNECT_WAIT, async {
            loop {
                if let Some(peer) = self.shared.state().live.clone() {
                    return Some(peer);
                }
                if connected.changed().await.is_err() {
                    return None;
                }
            }
        });
        wait.await.ok().flatten().ok_or(BridgeError::NotConnected)
    }
}

impl State {
    /// Deliver the answer of call `id` (only from the connection it was sent on).
    pub(crate) fn answer(&mut self, conn: u64, id: u64, answer: Result<Value, BridgeError>) {
        if self.calls.get(&id).is_some_and(|p| p.conn == conn) {
            if let Some(pending) = self.calls.remove(&id) {
                let _ = pending.tx.send(answer);
            }
        }
    }

    /// Fail every call sent on `conn` (it is gone).
    pub(crate) fn fail_calls_of(&mut self, conn: u64) {
        let ids: Vec<u64> = self.calls.iter().filter(|(_, p)| p.conn == conn).map(|(id, _)| *id).collect();
        for id in ids {
            if let Some(pending) = self.calls.remove(&id) {
                let _ = pending.tx.send(Err(BridgeError::NotConnected));
            }
        }
    }
}

/// "Chrome 153" from a user agent (Edge and Opera by their own names); the agent itself otherwise.
pub(crate) fn browser_label(ua: &str) -> String {
    let version = |marker: &str| {
        ua.split(marker).nth(1).map(|rest| rest.split(['.', ' ']).next().unwrap_or_default().to_string())
    };
    for (marker, name) in [("Edg/", "Microsoft Edge"), ("OPR/", "Opera"), ("Chrome/", "Chrome")] {
        if let Some(v) = version(marker) {
            return format!("{name} {v}");
        }
    }
    ua.chars().take(80).collect()
}
