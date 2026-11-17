//! The user's own Chrome, driven through the JobPilot extension.
//!
//! - [`bridge`] + [`tab`]: tabs in the user's Chrome ([`Core::browser`] -> [`Bridge::open`] -> [`Tab`]).
//! - [`session`]: [`SessionManager`], the sites the user is signed in to (a row each; JobPilot keeps no
//!   browser profile and never touches the user's cookies), their login tabs and the check.
//! - [`probe`]: the signed-in check (cookie domains for the site and not on a login page).
//! - [`challenge`]: is a login, captcha or block page in the way? (detection only; the user deals with it)
//! - [`render`]: render a page for discovery in a tab of the user's Chrome (a private tab, or their
//!   signed-in profile for a source with a session) and run that check; the tab is guarded
//!   ([`guard::allows`]) so pages stay off the local network. [`fetch`]: the same render for JD ingest.
//!
//! Get the manager with [`Core::sessions`]; every async method is `'static`, so the UI runs them with
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
pub mod fetch;
pub mod form;
pub(crate) mod guard;
pub mod probe;
pub mod render;
pub mod session;
pub mod tab;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use bridge::{Bridge, BridgeError, BridgeEvent, BridgeStatus, INCOGNITO_NOT_ALLOWED, NOT_CONNECTED};
pub use form::FormBrowser;
pub use guard::Policy;
pub use session::{SessionManager, SessionStatus};
pub use tab::{Clip, OpenOptions, Profile, Tab};

use crate::Core;

/// One manager per data dir, so login tabs are tracked in one place however many times
/// `core.sessions()` is called (`Core` has no field for it).
static MANAGERS: OnceLock<Mutex<HashMap<PathBuf, SessionManager>>> = OnceLock::new();

impl Core {
    /// The session manager (cheap to clone; share it with `runtime::spawn`).
    pub fn sessions(&self) -> SessionManager {
        let mut map = MANAGERS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
        map.entry(self.paths.data_dir.clone())
            .or_insert_with(|| SessionManager::new(&self.paths, self.store.clone(), self.secrets.clone()))
            .clone()
    }
}
