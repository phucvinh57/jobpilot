//! The challenge hand-off: what happens after a source stopped at a login, captcha or block page.
//!
//! ```text
//! core.discover_waiting()?            // [WaitingSource { reason: Captcha { url }, .. }]   (the run paused)
//! needs_human(&w.reason)              // Some(Problem::NeedsHuman(Challenge::Captcha))     (what to show)
//! core.discover_open_browser(&w.thread).await?   // "Open browser": a visible window on that page
//! // ... the user solves it themselves ...
//! core.discover_resume(&w.thread).await?          // "Continue": the window closes, the source goes on
//! ```
//!
//! The app **never solves, evades or waits out** a challenge: it only opens the page for the user.
//! The window uses the profile of the source's bound session, so what the user does there (a login, a
//! passed check) is what the headless reads reuse afterwards; a source without a session gets a clean
//! throwaway window, in which nothing is remembered. "Continue" and "Skip" close the window first (a
//! profile has one live browser), so the next headless read can use the profile.
//!
//! [`text_probe`] is the default challenge hook of a run: the cheap, browser-free check of what a
//! fetch returned. Pages rendered in the browser report their own challenge (`ChallengeFound`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use futures::FutureExt;

use super::deps::{ChallengeProbe, CheckStage};
use super::state::parse_source_thread;
use crate::browser::challenge::detect_text;
use crate::browser::gate::{self, SessionUse};
use crate::browser::guard::{Guard, Policy};
use crate::browser::render::{open_visible, Visible};
use crate::browser::BrowserHandle;
use crate::graph::{ThreadStatus, WaitReason};
use crate::health::{Challenge, Problem};
use crate::net::http::public_http_url;
use crate::{runtime, Core, Paths};

/// How often a window is checked to see whether the user closed it.
const WATCH_EVERY: Duration = Duration::from_secs(3);

/// The default [`ChallengeProbe`]: after a search page nothing is checked here (the adapters that
/// render a page raise their own challenge); after a posting was fetched the start of its text is
/// read by [`detect_text`], which only trusts short pages, so a real posting is never mistaken for a
/// block page.
pub fn text_probe() -> ChallengeProbe {
    Arc::new(|check| {
        async move {
            Ok(match check.stage {
                CheckStage::Search => None,
                CheckStage::Posting => detect_text(None, &check.text),
            })
        }
        .boxed()
    })
}

/// What the UI shows for a paused run: the [`Problem`] of a source waiting at a login, captcha or
/// block page; `None` for any other reason.
pub fn needs_human(reason: &WaitReason) -> Option<Problem> {
    Some(Problem::NeedsHuman(match reason {
        WaitReason::Login { .. } => Challenge::Login,
        WaitReason::Captcha { .. } => Challenge::Captcha,
        WaitReason::Blocked { .. } => Challenge::Blocked,
        _ => return None,
    }))
}

/// The page a paused source waits at, from its reason.
fn page_of(reason: &WaitReason) -> Option<(i64, &str)> {
    match reason {
        WaitReason::Login { source_id, url } | WaitReason::Captcha { source_id, url } | WaitReason::Blocked { source_id, url } => {
            Some((*source_id, url))
        }
        _ => None,
    }
}

/// A window opened for the user. The fields after the browser outlive it: the request guard its
/// requests go through, and the right to use the session's profile.
enum Held {
    /// In a managed session's profile.
    Session(BrowserHandle, SessionUse, Guard),
    /// With a throwaway profile.
    Temp(Box<Visible>),
}

impl Held {
    async fn is_alive(&mut self) -> bool {
        match self {
            Held::Session(handle, ..) => handle.is_alive().await,
            Held::Temp(window) => window.is_alive(),
        }
    }

    async fn close(self) {
        match self {
            Held::Session(handle, _in_use, _guard) => {
                let _ = handle.close().await;
            }
            Held::Temp(window) => window.close().await,
        }
    }
}

/// The registries below are process-wide and keyed by data dir and source thread (two data dirs in
/// one process, as in tests, can have threads of the same name).
fn key(paths: &Paths, thread: &str) -> String {
    format!("{}\u{0}{thread}", paths.data_dir.display())
}

/// The open windows by [`key`]. A window lives until the user continues or skips the source (or
/// closes it themselves).
fn open_windows() -> &'static Mutex<HashMap<String, Held>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// One lock per [`key`]: opening, closing and the watcher's check of its window never overlap (a
/// double click opens one window).
fn thread_lock(thread: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    // Forget the locks nobody holds, so the map does not grow with every thread ever seen.
    locks.retain(|name, lock| name == thread || Arc::strong_count(lock) > 1);
    locks.entry(thread.to_string()).or_default().clone()
}

fn take_window(thread: &str) -> Option<Held> {
    open_windows().lock().unwrap_or_else(|e| e.into_inner()).remove(thread)
}

fn put_window(thread: &str, held: Held) {
    open_windows().lock().unwrap_or_else(|e| e.into_inner()).insert(thread.to_string(), held);
}

/// Is a window open for `thread`? (For tests.)
#[cfg(test)]
pub(crate) fn has_window(paths: &Paths, thread: &str) -> bool {
    open_windows().lock().unwrap_or_else(|e| e.into_inner()).contains_key(&key(paths, thread))
}

/// Close the window opened for `thread`, if any, and wait until its profile is free.
pub(crate) async fn close_window(paths: &Paths, thread: &str) {
    close_key(&key(paths, thread)).await;
}

async fn close_key(key: &str) {
    let lock = thread_lock(key);
    let _one_at_a_time = lock.lock().await;
    if let Some(held) = take_window(key) {
        held.close().await;
    }
}

/// Close the window opened for `thread` in the background (for a caller that cannot wait).
pub(crate) fn discard_window(paths: &Paths, thread: &str) {
    let key = key(paths, thread);
    drop(runtime::spawn(async move { close_key(&key).await }));
}

/// Watch the window under `key`: when the user closes it themselves, close what is left of it (the
/// session's profile is free again for reads) and stop. Also stops when the window is gone.
fn watch(key: String) {
    drop(runtime::spawn(async move {
        loop {
            tokio::time::sleep(WATCH_EVERY).await;
            let lock = thread_lock(&key);
            let _one_at_a_time = lock.lock().await;
            let Some(mut held) = take_window(&key) else { return };
            if held.is_alive().await {
                put_window(&key, held);
            } else {
                tracing::info!("the user closed a browser window opened for a paused source");
                held.close().await;
                return;
            }
        }
    }));
}

impl Core {
    /// "Open browser" for a source that waits at a login, captcha or block page: a **visible**
    /// browser opens on that page, in the profile of the source's bound session if it has one (else a
    /// clean throwaway window). The user deals with the page themselves; the app does nothing in it.
    /// Then "Continue" is [`Core::discover_resume`], which closes the window and goes on.
    ///
    /// Opening it again while the window is still open does nothing (also when two clicks come at
    /// once); if the user closed it, a new one opens. `Err` when the thread is not a source waiting
    /// for the user, when no Chrome is installed (`classify` gives `Problem::NoChrome`), when the
    /// session's profile is open elsewhere, or when the session is being read right now for longer
    /// than a minute ([`SessionInUse`](crate::browser::gate::SessionInUse)).
    ///
    /// The window is a browser like the headless reads: not disguised, and its requests go through the
    /// request guard (no machine-local or private hosts).
    pub async fn discover_open_browser(&self, thread: &str) -> Result<()> {
        open_browser(self, thread, false).await
    }
}

/// [`Core::discover_open_browser`]; `allow_private` lets tests open fixture pages on `127.0.0.1`.
pub(crate) async fn open_browser(core: &Core, thread: &str, allow_private: bool) -> Result<()> {
    parse_source_thread(thread).context("not a discovery source thread")?;
    let key = key(&core.paths, thread);
    let lock = thread_lock(&key);
    let _one_at_a_time = lock.lock().await;

    let status = core.graphs.status(thread)?.map(|t| t.status);
    let Some(ThreadStatus::Waiting(reason)) = status else {
        bail!("that source is not waiting for a login, captcha or block page");
    };
    let (source_id, url) = page_of(&reason).context("that source is not waiting for a login, captcha or block page")?;
    let url = public_http_url(url, allow_private).context("the page to open")?.to_string();
    let session_id = core.store.get_source(source_id)?.and_then(|s| s.session_id);

    if let Some(mut held) = take_window(&key) {
        if held.is_alive().await {
            put_window(&key, held);
            return Ok(());
        }
        // The user closed it themselves: release the profile and open a fresh one.
        held.close().await;
    }
    let held = match session_id {
        Some(id) => open_in_session(core, id, &url, allow_private).await?,
        None => Held::Temp(Box::new(open_visible(&core.paths, &url, allow_private).await?)),
    };
    tracing::info!(thread, source_id, with_session = session_id.is_some(), "browser opened for the user");
    put_window(&key, held);
    watch(key);
    Ok(())
}

/// A visible browser on session `id`'s profile, showing `url`.
async fn open_in_session(core: &Core, id: i64, url: &str, allow_private: bool) -> Result<Held> {
    let in_use = gate::window(&core.paths.session_dir(id)).await.map_err(anyhow::Error::new)?;
    let guard = Guard::start(Policy::new(allow_private)).await?;
    let handle = core.sessions().launch_reading(id, false, &guard.chrome_args()).await?;
    let shown = async {
        // The window opens on a blank tab: use it.
        match handle.pages().await?.into_iter().next() {
            Some(page) => {
                page.goto(url).await.context("opening the page")?;
            }
            None => {
                handle.new_page(url).await?;
            }
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(e) = shown {
        let _ = handle.close().await;
        return Err(e);
    }
    Ok(Held::Session(handle, in_use, guard))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::nodes::ChallengeKind;

    fn check(stage: CheckStage, text: &str) -> super::super::ChallengeCheck {
        super::super::ChallengeCheck {
            source: crate::store::Source {
                id: 1,
                kind: "career_page".into(),
                name: "Acme".into(),
                url: "https://acme.test/careers".into(),
                purpose: crate::store::SourcePurpose::Jobs,
                enabled: true,
                session_id: None,
                last_run_at: None,
                last_status: None,
            },
            stage,
            url: "https://acme.test/jobs/login-engineer".into(),
            text: text.into(),
        }
    }

    #[tokio::test]
    async fn the_default_probe_reads_a_fetched_postings_start_and_ignores_search_pages() {
        let probe = text_probe();
        let posting = format!("Backend Engineer. {}", "You will build services in Rust and own their quality. ".repeat(20));
        assert_eq!(probe(check(CheckStage::Posting, &posting)).await.unwrap(), None);
        assert_eq!(probe(check(CheckStage::Posting, "Just a moment... Enable JavaScript and cookies to continue")).await.unwrap(), Some(ChallengeKind::Captcha));
        assert_eq!(probe(check(CheckStage::Posting, "Access denied. You have been blocked.")).await.unwrap(), Some(ChallengeKind::Blocked));
        assert_eq!(probe(check(CheckStage::Posting, "Sign in to continue")).await.unwrap(), Some(ChallengeKind::Login));
        assert_eq!(probe(check(CheckStage::Search, "Just a moment...")).await.unwrap(), None, "nothing to read after a search");
        // A normal posting that talks about bots, traffic and sign-in is not a block page.
        assert_eq!(probe(check(CheckStage::Posting, crate::browser::challenge::tests::BOT_POSTING)).await.unwrap(), None);
        // The posting's own address says nothing: a job called "login engineer" is not a login wall.
        assert_eq!(probe(check(CheckStage::Posting, &posting)).await.unwrap(), None);
    }

    #[test]
    fn a_waiting_reason_becomes_the_problem_the_ui_shows() {
        let at = |kind: &str| -> WaitReason {
            serde_json::from_value(serde_json::json!({"kind": kind, "source_id": 3, "url": "https://x.test/"})).unwrap()
        };
        assert_eq!(needs_human(&at("login")), Some(Problem::NeedsHuman(Challenge::Login)));
        assert_eq!(needs_human(&at("captcha")), Some(Problem::NeedsHuman(Challenge::Captcha)));
        assert_eq!(needs_human(&at("blocked")), Some(Problem::NeedsHuman(Challenge::Blocked)));
        assert_eq!(needs_human(&WaitReason::UserMessage), None);
        assert_eq!(needs_human(&WaitReason::Unknown), None);
        assert_eq!(page_of(&at("captcha")), Some((3, "https://x.test/")));
        assert_eq!(page_of(&WaitReason::ConfirmPreferences), None);
    }
}
