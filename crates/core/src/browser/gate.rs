//! One use of a session's profile at a time inside the app, for job discovery.
//!
//! A profile has one live browser ([`ProfileLock`](super::ProfileLock) refuses a second launch with an
//! opaque "already open"). Discovery uses a session in two ways that must not meet: **reads** (a
//! headless render of a page, in the source's bound session) and the **window** opened for the user
//! at a challenge (it lives until they continue). This module is the one process-wide place that
//! orders them, shared by every [`Renderer`](super::render::Renderer) and by `discover_open_browser`:
//!
//! - reads of one session go one after the other (a later one waits, up to [`READ_WAIT`]);
//! - a read that finds the window open does not wait for it: [`SessionInUse::Window`];
//! - opening the window waits for a read in progress (a read ends within a minute).
//!
//! Gates are keyed by the profile directory, so two data dirs (tests) never share one.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::OwnedMutexGuard;

/// How long a read waits for the read in front of it.
pub(crate) const READ_WAIT: Duration = Duration::from_secs(120);
/// How long opening the window waits for a read in progress.
pub(crate) const WINDOW_WAIT: Duration = Duration::from_secs(90);

/// Why a session could not be used right now. The text is for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionInUse {
    /// The window opened for the user is still open.
    Window,
    /// Another read of the session is still running.
    Busy,
}

impl fmt::Display for SessionInUse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SessionInUse::Window => "the browser session is in use: its window is open for you. Continue or skip the paused source (that closes it), then try again",
            SessionInUse::Busy => "the browser session is in use: another read of it is still running. Try again in a minute",
        })
    }
}

impl std::error::Error for SessionInUse {}

struct Gate {
    slot: Arc<tokio::sync::Mutex<()>>,
    /// The holder of the slot is the window for the user.
    window: AtomicBool,
}

fn gate_for(dir: &Path) -> Arc<Gate> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Gate>>>> = OnceLock::new();
    let mut gates = GATES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    gates.entry(dir.to_path_buf()).or_insert_with(|| Arc::new(Gate { slot: Arc::default(), window: AtomicBool::new(false) })).clone()
}

/// The right to use a session's profile; dropping it gives the right back.
pub(crate) struct SessionUse {
    gate: Arc<Gate>,
    window: bool,
    _slot: OwnedMutexGuard<()>,
}

impl Drop for SessionUse {
    fn drop(&mut self) {
        if self.window {
            self.gate.window.store(false, Ordering::SeqCst);
        }
        // `_slot` is released after this.
    }
}

/// Take the profile at `dir` for a read: waits for earlier reads, refuses while the window is open.
pub(crate) async fn read(dir: &Path) -> Result<SessionUse, SessionInUse> {
    read_within(dir, READ_WAIT).await
}

async fn read_within(dir: &Path, wait: Duration) -> Result<SessionUse, SessionInUse> {
    let gate = gate_for(dir);
    if let Ok(slot) = gate.slot.clone().try_lock_owned() {
        return Ok(SessionUse { gate, window: false, _slot: slot });
    }
    if gate.window.load(Ordering::SeqCst) {
        return Err(SessionInUse::Window);
    }
    match tokio::time::timeout(wait, gate.slot.clone().lock_owned()).await {
        Ok(slot) => Ok(SessionUse { gate, window: false, _slot: slot }),
        Err(_) if gate.window.load(Ordering::SeqCst) => Err(SessionInUse::Window),
        Err(_) => Err(SessionInUse::Busy),
    }
}

/// Take the profile at `dir` for the window opened for the user: waits for a read in progress.
pub(crate) async fn window(dir: &Path) -> Result<SessionUse, SessionInUse> {
    window_within(dir, WINDOW_WAIT).await
}

async fn window_within(dir: &Path, wait: Duration) -> Result<SessionUse, SessionInUse> {
    let gate = gate_for(dir);
    let slot = match tokio::time::timeout(wait, gate.slot.clone().lock_owned()).await {
        Ok(slot) => slot,
        Err(_) if gate.window.load(Ordering::SeqCst) => return Err(SessionInUse::Window),
        Err(_) => return Err(SessionInUse::Busy),
    };
    gate.window.store(true, Ordering::SeqCst);
    Ok(SessionUse { gate, window: true, _slot: slot })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        PathBuf::from(format!("/gate-test/{}/{name}", std::process::id()))
    }

    #[tokio::test]
    async fn reads_of_one_session_go_one_after_the_other_and_other_sessions_are_free() {
        let (a, b) = (dir("reads-a"), dir("reads-b"));
        let first = read(&a).await.unwrap();
        // Another session is not held up.
        drop(read(&b).await.unwrap());
        // A second read of the same session waits until the first is done.
        let waiting = tokio::spawn({
            let a = a.clone();
            async move { read(&a).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!waiting.is_finished(), "it waits for the first read");
        drop(first);
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), waiting).await.unwrap().unwrap(), Ok(()));
    }

    #[tokio::test]
    async fn a_read_that_finds_the_window_open_is_told_so_at_once_and_after_it_closes_reads_go_on() {
        let d = dir("window");
        let open = window(&d).await.unwrap();
        let started = std::time::Instant::now();
        let refused = read(&d).await.err().expect("refused while the window is open");
        assert_eq!(refused, SessionInUse::Window);
        assert!(started.elapsed() < Duration::from_secs(2), "no waiting for a window the user holds");
        assert!(refused.to_string().contains("in use") && refused.to_string().contains("window"));
        assert_eq!(window_within(&d, Duration::from_millis(50)).await.err(), Some(SessionInUse::Window), "one window per session");
        drop(open);
        drop(read(&d).await.unwrap());
        drop(window(&d).await.unwrap());
    }

    #[tokio::test]
    async fn the_window_waits_for_a_read_in_progress_and_a_read_that_waits_too_long_says_busy() {
        let d = dir("busy");
        let reading = read(&d).await.unwrap();
        assert_eq!(read_within(&d, Duration::from_millis(50)).await.err(), Some(SessionInUse::Busy));
        let opening = tokio::spawn({
            let d = d.clone();
            async move { window(&d).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!opening.is_finished());
        drop(reading);
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), opening).await.unwrap().unwrap(), Ok(()));
        assert!(SessionInUse::Busy.to_string().contains("in use"));
    }
}
