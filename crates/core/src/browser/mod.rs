//! P4 - managed Chrome sessions (chromiumoxide).
//!
//! - [`chrome`]: which Chrome to run (`JOBPILOT_CHROME` -> managed Chrome-for-Testing -> system) and the
//!   explicit "download browser" step.
//! - [`session`]: [`SessionManager`], the single owner of every launch. Each session is an app-owned
//!   profile dir `sessions/<id>/`, guarded by a lock file ([`lock`]) so at most one browser runs on it.
//! - [`probe`]: the logged-in health check (cookies for the start domain and not on a login page).
//!
//! Get the manager with [`Core::sessions`]; every method is async and `'static`, so the UI runs them with
//! `runtime::spawn`:
//!
//! ```text
//! let mgr = core.sessions();
//! let rx = runtime::spawn(async move { mgr.check(id).await });
//! ```
//!
//! P5 launches automation browsers with [`SessionManager::launch`] and drives the returned
//! [`BrowserHandle`] (`new_page(url)` -> `chromiumoxide::Page`, `cookies()`, `close()`).

pub mod chrome;
#[cfg(test)]
mod integration;
pub mod lock;
pub mod probe;
pub mod session;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

pub use chrome::{Chrome, ChromeSource, DownloadProgress};
pub use lock::{LockError, ProfileLock};
pub use session::{BrowserHandle, SessionManager, SessionStatus};

use crate::Core;

/// One manager per data dir, so live browsers / login windows are tracked in one place however many
/// times `core.sessions()` is called (`Core` itself is owned by the foundation and has no field for it).
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
