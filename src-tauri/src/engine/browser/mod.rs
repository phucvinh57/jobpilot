//! Managed Chrome sessions (chromiumoxide).
//!
//! - [`bridge`] + [`tab`]: the replacement being ported to: tabs in the user's own Chrome, driven
//!   through the JobPilot extension ([`Core::browser`] -> [`Bridge::open`] -> [`Tab`]).
//! - [`chrome`]: which Chrome to run (`JOBPILOT_CHROME` -> managed Chrome-for-Testing -> system) and the
//!   explicit "download browser" step.
//! - [`session`]: [`SessionManager`], the single owner of every launch. Each session is an app-owned
//!   profile dir `sessions/<id>/`, guarded by a lock file ([`lock`]) so at most one browser runs on it.
//! - [`probe`]: the logged-in health check (cookies for the start domain and not on a login page).
//! - [`challenge`]: is a login, captcha or block page in the way? (detection only; the user deals with it)
//! - [`render`]: render a page for discovery in a tab of the user's Chrome (a private tab, or their
//!   signed-in profile for a source with a session) and run that check; the tab is guarded
//!   (`guard::allows`) so pages stay off the local network. [`fetch`]: the same render for JD ingest.
//!
//! Get the manager with [`Core::sessions`]; every method is async and `'static`, so the UI runs them with
//! `runtime::spawn`:
//!
//! ```text
//! let mgr = core.sessions();
//! let rx = runtime::spawn(async move { mgr.check(id).await });
//! ```
//!
//! Auto-apply opens one tab per application in the user's Chrome ([`form`]: [`FormBrowser`] around a
//! [`Tab`]).

pub mod bridge;
pub mod challenge;
pub mod chrome;
pub mod fetch;
pub mod form;
// The session gate, the guard proxy and the session reads are unused since discovery reads pages
// through the browser bridge; they go with the rest of the chromiumoxide code.
#[allow(dead_code)]
pub(crate) mod gate;
#[allow(dead_code)]
pub(crate) mod guard;
#[cfg(test)]
mod integration;
pub mod lock;
pub mod probe;
pub mod render;
#[allow(dead_code)]
pub mod session;
pub mod tab;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use bridge::{Bridge, BridgeError, BridgeEvent, BridgeStatus, INCOGNITO_NOT_ALLOWED, NOT_CONNECTED};
pub use chrome::{Chrome, ChromeSource, DownloadProgress};
pub use guard::Policy;
pub use tab::{Clip, OpenOptions, Profile, Tab};
pub use form::FormBrowser;
pub use lock::{LockError, ProfileLock};
pub use session::{BrowserHandle, SessionManager, SessionStatus};

use crate::engine::Core;

/// One manager per data dir, so live browsers / login windows are tracked in one place however many
/// times `core.sessions()` is called (`Core` has no field for it).
static MANAGERS: OnceLock<Mutex<HashMap<PathBuf, SessionManager>>> = OnceLock::new();

impl Core {
    /// The browser session manager (cheap to clone; share it with `runtime::spawn`).
    pub fn sessions(&self) -> SessionManager {
        let mut map = MANAGERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.entry(self.paths.data_dir.clone())
            .or_insert_with(|| SessionManager::new(self.paths.clone(), self.store.clone()))
            .clone()
    }

    /// The Chrome used for sessions (`None` = not installed yet).
    pub fn chrome(&self) -> Option<Chrome> {
        chrome::resolve(&self.paths)
    }
}
