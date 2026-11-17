//! The challenge hand-off: what happens after a source stopped at a login, captcha or block page.
//!
//! ```text
//! core.discover_waiting()?            // [WaitingSource { reason: Captcha { url }, .. }]   (the run paused)
//! needs_human(&w.reason)              // Some(Problem::NeedsHuman(Challenge::Captcha))     (what to show)
//! core.discover_open_browser(&w.thread).await?   // "Open browser": a tab on that page, in front
//! // ... the user solves it themselves ...
//! core.discover_resume(&w.thread).await?          // "Continue": the tab closes, the source goes on
//! ```
//!
//! The app **never solves, evades or waits out** a challenge: it only opens the page for the user, in
//! a focused tab of their own Chrome. A source with a session gets a tab in the user's normal
//! profile, where they are signed in, so what they do there (a login, a passed check) is what the
//! source's next reads use; a source without one gets a tab in the JobPilot Incognito window, in
//! which nothing is remembered. "Continue" and "Skip" close the tab; a tab the user closed
//! themselves is forgotten.
//!
//! [`text_probe`] is the default challenge hook of a run: the cheap, browser-free check of what a
//! fetch returned. Pages rendered in the browser report their own challenge (`ChallengeFound`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{bail, Context, Result};
use futures::FutureExt;

use super::deps::{ChallengeProbe, CheckStage};
use super::state::parse_source_thread;
use crate::engine::browser::challenge::detect_text;
use crate::engine::browser::guard::Policy;
use crate::engine::browser::render::open_visible;
use crate::engine::browser::{Profile, Tab};
use crate::engine::graph::{ThreadStatus, WaitReason};
use crate::engine::health::{Challenge, Problem};
use crate::engine::net::http::public_http_url;
use crate::engine::{runtime, Core, Paths};

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

/// The registries below are process-wide and keyed by data dir and source thread (two data dirs in
/// one process, as in tests, can have threads of the same name).
fn key(paths: &Paths, thread: &str) -> String {
    format!("{}\u{0}{thread}", paths.data_dir.display())
}

/// The tabs opened for the user, by [`key`]. A tab lives until the user continues or skips the
/// source (or closes it themselves).
fn open_tabs() -> &'static Mutex<HashMap<String, Tab>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Tab>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

/// One lock per [`key`]: opening and closing its tab never overlap (a double click opens one tab).
fn thread_lock(thread: &str) -> Arc<tokio::sync::Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> = OnceLock::new();
    let mut locks = LOCKS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    // Forget the locks nobody holds, so the map does not grow with every thread ever seen.
    locks.retain(|name, lock| name == thread || Arc::strong_count(lock) > 1);
    locks.entry(thread.to_string()).or_default().clone()
}

fn take_tab(key: &str) -> Option<Tab> {
    open_tabs().lock().unwrap_or_else(|e| e.into_inner()).remove(key)
}

fn put_tab(key: &str, tab: Tab) {
    open_tabs().lock().unwrap_or_else(|e| e.into_inner()).insert(key.to_string(), tab);
}

/// Is a tab open for `thread`? (For tests.)
#[cfg(test)]
pub(crate) fn has_window(paths: &Paths, thread: &str) -> bool {
    open_tabs().lock().unwrap_or_else(|e| e.into_inner()).get(&key(paths, thread)).is_some_and(Tab::is_open)
}

/// The tab open for `thread`, if any. (For tests.)
#[cfg(test)]
pub(crate) fn tab_of(paths: &Paths, thread: &str) -> Option<Tab> {
    open_tabs().lock().unwrap_or_else(|e| e.into_inner()).get(&key(paths, thread)).cloned()
}

/// Close the tab opened for `thread`, if any.
pub(crate) async fn close_window(paths: &Paths, thread: &str) {
    close_key(&key(paths, thread)).await;
}

async fn close_key(key: &str) {
    let lock = thread_lock(key);
    let _one_at_a_time = lock.lock().await;
    if let Some(tab) = take_tab(key) {
        tab.close().await;
    }
}

/// Close the tab opened for `thread` in the background (for a caller that cannot wait).
pub(crate) fn discard_window(paths: &Paths, thread: &str) {
    let key = key(paths, thread);
    drop(runtime::spawn(async move { close_key(&key).await }));
}

/// Forget the tab under `key` once it is closed (by the user, or by "Continue" / "Skip"), unless a
/// newer tab took its place.
fn watch(key: String, tab: Tab) {
    drop(runtime::spawn(async move {
        tab.closed().await;
        let lock = thread_lock(&key);
        let _one_at_a_time = lock.lock().await;
        let mut tabs = open_tabs().lock().unwrap_or_else(|e| e.into_inner());
        if tabs.get(&key).is_some_and(|held| held.id() == tab.id()) {
            tracing::info!(tab = tab.id(), "the user closed a browser tab opened for a paused source");
            tabs.remove(&key);
        }
    }));
}

impl Core {
    /// "Open browser" for a source that waits at a login, captcha or block page: a **focused** tab
    /// of the user's Chrome on that page, in their signed-in profile when the source has a session
    /// (else in the JobPilot Incognito window). The user deals with the page themselves; the app
    /// does nothing in it. "Continue" ([`Core::discover_resume`]) closes the tab and goes on.
    ///
    /// While the tab is open (double clicks included) it is only brought to the front; once the
    /// user closed it, a new one opens. `Err` when the thread is not a waiting source, the page is
    /// on this machine or a private network, or the browser cannot be used (a
    /// [`BridgeError`](crate::engine::browser::BridgeError): the extension is not connected, or not
    /// allowed in Incognito).
    ///
    /// Like the reads, the tab is not disguised and its requests are checked by the guard (no
    /// machine-local or private hosts).
    pub async fn discover_open_browser(&self, thread: &str) -> Result<()> {
        open_browser(self, thread, false).await
    }
}

/// [`Core::discover_open_browser`]; `allow_private` lets tests open fixture pages on this machine.
pub(crate) async fn open_browser(core: &Core, thread: &str, allow_private: bool) -> Result<()> {
    parse_source_thread(thread).context("not a discovery source thread")?;
    let session_of = |source_id: i64| -> Result<Option<i64>> { Ok(core.store.get_source(source_id)?.and_then(|s| s.session_id)) };
    open_waiting_page(core, thread, "source", allow_private, session_of).await
}

/// "Open browser" for any graph `thread` paused at a login, captcha or block page (a discovery
/// source, the source suggestion search): a focused tab on the page it waits at, in the signed-in
/// profile when `session_of(source_id)` names a session, else in the JobPilot Incognito window.
/// `what` names the thread in the error ("that source is not waiting ..."). While the tab is open
/// it is only brought to the front; [`close_window`] closes it.
pub(crate) async fn open_waiting_page(
    core: &Core,
    thread: &str,
    what: &str,
    allow_private: bool,
    session_of: impl FnOnce(i64) -> Result<Option<i64>>,
) -> Result<()> {
    let key = key(&core.paths, thread);
    let lock = thread_lock(&key);
    let _one_at_a_time = lock.lock().await;

    let status = core.graphs.status(thread)?.map(|t| t.status);
    let Some(ThreadStatus::Waiting(reason)) = status else {
        bail!("that {what} is not waiting for a login, captcha or block page");
    };
    let (source_id, url) = page_of(&reason).with_context(|| format!("that {what} is not waiting for a login, captcha or block page"))?;
    let url = public_http_url(url, allow_private).context("the page to open")?.to_string();
    let session_id = session_of(source_id)?;

    if let Some(tab) = take_tab(&key) {
        if tab.is_open() {
            if let Err(e) = tab.focus().await {
                tracing::debug!("bringing a browser tab to the front: {e}");
            }
            put_tab(&key, tab);
            return Ok(());
        }
    }
    let profile = if session_id.is_some() { Profile::Signed } else { Profile::Private };
    let tab = open_visible(&core.browser(), &url, profile, Policy::new(allow_private)).await?;
    tracing::info!(thread, source_id, tab = tab.id(), with_session = session_id.is_some(), "browser tab opened for the user");
    put_tab(&key, tab.clone());
    watch(key, tab);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::graph::nodes::ChallengeKind;

    fn check(stage: CheckStage, text: &str) -> super::super::ChallengeCheck {
        super::super::ChallengeCheck {
            source: crate::engine::store::Source {
                id: 1,
                kind: "career_page".into(),
                name: "Acme".into(),
                url: "https://acme.test/careers".into(),
                purpose: crate::engine::store::SourcePurpose::Jobs,
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
        assert_eq!(probe(check(CheckStage::Posting, crate::engine::browser::challenge::tests::BOT_POSTING)).await.unwrap(), None);
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
