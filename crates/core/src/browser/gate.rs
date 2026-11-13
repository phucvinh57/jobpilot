//! One use of a session's profile at a time inside the app, for job discovery.
//!
//! A profile has one live browser ([`ProfileLock`](super::ProfileLock) refuses a second launch with an
//! opaque "already open"). Discovery uses a session in two ways that must not meet: **reads** (a
//! headless render of a page, in the source's bound session) and the **window** opened for the user
//! at a challenge (it lives until they continue). This module is the one process-wide place that
//! orders them, shared by every [`Renderer`](super::render::Renderer) and by `discover_open_browser`.
//!
//! Each session has one [`State`], in one mutex, so the answer to "may I?" and the change it causes
//! are one step:
//!
//! ```text
//!            read                 window                    read ends
//!   Free --------------> Reading ------------> WindowPending ----------> Window
//!     |  window                 (a read is running: the window waits its turn)   |
//!     +---------------------------------------------------------> Window <------+
//!   Window / WindowPending --window ends--> Free
//! ```
//!
//! - reads of one session go one after the other (a later one waits, up to [`READ_WAIT`]);
//! - a read that meets a window that is **open or still opening** does not wait for it:
//!   [`SessionInUse::Window`], at once; reads already waiting fail the same way when a window is
//!   next in line;
//! - opening the window waits for the read in progress (a read ends within a minute) and, while
//!   it waits, no new read starts in front of it.
//!
//! Gates are keyed by the profile directory, so two data dirs (tests) never share one.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokio::time::Instant;

/// How long a read waits for the read in front of it.
pub(crate) const READ_WAIT: Duration = Duration::from_secs(120);
/// How long opening the window waits for a read in progress.
pub(crate) const WINDOW_WAIT: Duration = Duration::from_secs(90);
/// How often a waiting caller looks at the state again.
const POLL: Duration = Duration::from_millis(25);

/// Why a session could not be used right now. The text is for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionInUse {
    /// The window opened for the user is open (or opening).
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

/// Who has a session's profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Free,
    /// A read is running (others wait).
    Reading,
    /// The window waits for the running read to end; the next owner is the window.
    WindowPending,
    /// The window is open.
    Window,
}

struct Gate {
    state: Mutex<State>,
}

impl Gate {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn gate_for(dir: &Path) -> Arc<Gate> {
    static GATES: OnceLock<Mutex<HashMap<PathBuf, Arc<Gate>>>> = OnceLock::new();
    let mut gates = GATES.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    gates.entry(dir.to_path_buf()).or_insert_with(|| Arc::new(Gate { state: Mutex::new(State::Free) })).clone()
}

/// The right to use a session's profile; dropping it gives the right back.
pub(crate) struct SessionUse {
    gate: Arc<Gate>,
    window: bool,
}

impl Drop for SessionUse {
    fn drop(&mut self) {
        let mut state = self.gate.state();
        *state = match (*state, self.window) {
            // A read ends: the window that waited for it is next, else the profile is free.
            (State::WindowPending, false) => State::Window,
            _ => State::Free,
        };
    }
}

/// Take the profile at `dir` for a read: waits for earlier reads, refuses while the window is open
/// or opening.
pub(crate) async fn read(dir: &Path) -> Result<SessionUse, SessionInUse> {
    read_within(dir, READ_WAIT).await
}

async fn read_within(dir: &Path, wait: Duration) -> Result<SessionUse, SessionInUse> {
    let gate = gate_for(dir);
    let deadline = Instant::now() + wait;
    loop {
        {
            let mut state = gate.state();
            match *state {
                State::Free => {
                    *state = State::Reading;
                    drop(state);
                    return Ok(SessionUse { gate, window: false });
                }
                State::Window | State::WindowPending => return Err(SessionInUse::Window),
                State::Reading => {}
            }
        }
        if Instant::now() >= deadline {
            return Err(SessionInUse::Busy);
        }
        tokio::time::sleep(POLL).await;
    }
}

/// Take the profile at `dir` for the window opened for the user: waits for a read in progress (and
/// no read starts in front of it meanwhile). One window per session.
pub(crate) async fn window(dir: &Path) -> Result<SessionUse, SessionInUse> {
    window_within(dir, WINDOW_WAIT).await
}

async fn window_within(dir: &Path, wait: Duration) -> Result<SessionUse, SessionInUse> {
    let gate = gate_for(dir);
    let deadline = Instant::now() + wait;
    {
        let mut state = gate.state();
        match *state {
            State::Free => {
                *state = State::Window;
                drop(state);
                return Ok(SessionUse { gate, window: true });
            }
            State::Window | State::WindowPending => return Err(SessionInUse::Window),
            State::Reading => *state = State::WindowPending,
        }
    }
    // The read in progress hands the profile over when it ends (see `SessionUse::drop`). If this
    // wait is given up or dropped before that, `pending` puts the state back.
    let mut pending = Pending { gate: gate.clone(), claimed: false };
    loop {
        tokio::time::sleep(POLL).await;
        let state = *gate.state();
        match state {
            State::Window => {
                pending.claimed = true;
                return Ok(SessionUse { gate, window: true });
            }
            State::WindowPending if Instant::now() >= deadline => return Err(SessionInUse::Busy),
            _ => {}
        }
    }
}

/// A window that waits for the read in progress. Unless it was claimed, dropping it (the wait timed
/// out or the caller went away) undoes the wait: reads may start again, and a profile handed over
/// to nobody is freed.
struct Pending {
    gate: Arc<Gate>,
    claimed: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if self.claimed {
            return;
        }
        let mut state = self.gate.state();
        *state = match *state {
            State::WindowPending => State::Reading,
            State::Window => State::Free,
            other => other,
        };
    }
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

    #[tokio::test]
    async fn a_read_never_queues_behind_a_window_that_is_opening() {
        let d = dir("opening");
        let reading = read(&d).await.unwrap();
        let opening = tokio::spawn({
            let d = d.clone();
            async move { window(&d).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        // The window is next in line: a new read is refused at once, not queued behind it.
        let started = std::time::Instant::now();
        assert_eq!(read(&d).await.err(), Some(SessionInUse::Window));
        assert!(started.elapsed() < Duration::from_secs(1), "no waiting");
        assert_eq!(window_within(&d, Duration::from_millis(50)).await.err(), Some(SessionInUse::Window), "one window per session");
        drop(reading);
        let window = tokio::time::timeout(Duration::from_secs(5), opening).await.unwrap().unwrap().unwrap();
        assert_eq!(read(&d).await.err(), Some(SessionInUse::Window), "and while it is open");
        drop(window);
        drop(read(&d).await.unwrap());
    }

    #[tokio::test]
    async fn reads_already_waiting_fail_when_a_window_is_next() {
        let d = dir("queue");
        let first = read(&d).await.unwrap();
        let queued = tokio::spawn({
            let d = d.clone();
            async move { read(&d).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!queued.is_finished(), "the second read waits for the first");
        let opening = tokio::spawn({
            let d = d.clone();
            async move { window(&d).await }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(tokio::time::timeout(Duration::from_secs(5), queued).await.unwrap().unwrap(), Err(SessionInUse::Window), "it does not run behind the window");
        drop(first);
        drop(tokio::time::timeout(Duration::from_secs(5), opening).await.unwrap().unwrap().unwrap());
    }

    #[tokio::test]
    async fn a_window_that_gives_up_or_is_dropped_while_waiting_does_not_strand_the_session() {
        let d = dir("giveup");
        let reading = read(&d).await.unwrap();
        // Gives up: busy, and reads may queue again.
        assert_eq!(window_within(&d, Duration::from_millis(60)).await.err(), Some(SessionInUse::Busy));
        assert_eq!(read_within(&d, Duration::from_millis(50)).await.err(), Some(SessionInUse::Busy), "a read still waits for the read, not for a window");
        // Dropped while waiting (the caller went away).
        let abandoned = tokio::spawn({
            let d = d.clone();
            async move { window(&d).await.map(|_| ()) }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        abandoned.abort();
        let _ = abandoned.await;
        drop(reading);
        // Neither left a window behind: the session is free.
        let started = std::time::Instant::now();
        drop(read(&d).await.unwrap());
        drop(window(&d).await.unwrap());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
