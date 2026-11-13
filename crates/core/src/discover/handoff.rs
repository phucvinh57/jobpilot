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

use anyhow::{bail, Context, Result};
use futures::FutureExt;

use super::deps::{ChallengeProbe, CheckStage};
use super::state::parse_source_thread;
use crate::browser::challenge::detect_text;
use crate::browser::render::{open_visible, Visible};
use crate::browser::BrowserHandle;
use crate::graph::{ThreadStatus, WaitReason};
use crate::health::{Challenge, Problem};
use crate::net::http::checked_url;
use crate::{runtime, Core};

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

/// A window opened for the user.
enum Held {
    /// In a managed session's profile.
    Session(BrowserHandle),
    /// With a throwaway profile.
    Temp(Box<Visible>),
}

impl Held {
    async fn is_alive(&mut self) -> bool {
        match self {
            Held::Session(handle) => handle.is_alive().await,
            Held::Temp(window) => window.is_alive(),
        }
    }

    async fn close(self) {
        match self {
            Held::Session(handle) => {
                let _ = handle.close().await;
            }
            Held::Temp(window) => window.close().await,
        }
    }
}

/// The open windows by source thread. A window lives until the user continues or skips the source
/// (or closes it themselves).
fn open_windows() -> &'static Mutex<HashMap<String, Held>> {
    static OPEN: OnceLock<Mutex<HashMap<String, Held>>> = OnceLock::new();
    OPEN.get_or_init(Default::default)
}

fn take_window(thread: &str) -> Option<Held> {
    open_windows().lock().unwrap_or_else(|e| e.into_inner()).remove(thread)
}

/// Is a window open for `thread`? (For tests.)
#[cfg(test)]
pub(crate) fn has_window(thread: &str) -> bool {
    open_windows().lock().unwrap_or_else(|e| e.into_inner()).contains_key(thread)
}

/// Close the window opened for `thread`, if any, and wait until its profile is free.
pub(crate) async fn close_window(thread: &str) {
    if let Some(held) = take_window(thread) {
        held.close().await;
    }
}

/// Close the window opened for `thread` in the background (for a caller that cannot wait).
pub(crate) fn discard_window(thread: &str) {
    if let Some(held) = take_window(thread) {
        drop(runtime::spawn(held.close()));
    }
}

impl Core {
    /// "Open browser" for a source that waits at a login, captcha or block page: a **visible**
    /// browser opens on that page, in the profile of the source's bound session if it has one (else a
    /// clean throwaway window). The user deals with the page themselves; the app does nothing in it.
    /// Then "Continue" is [`Core::discover_resume`], which closes the window and goes on.
    ///
    /// Opening it again while the window is still open does nothing; if the user closed it, a new one
    /// opens. `Err` when the thread is not a source waiting for the user, when no Chrome is installed
    /// (`classify` gives `Problem::NoChrome`), or when the session's profile is open elsewhere.
    pub async fn discover_open_browser(&self, thread: &str) -> Result<()> {
        parse_source_thread(thread).context("not a discovery source thread")?;
        let status = self.graphs.status(thread)?.map(|t| t.status);
        let Some(ThreadStatus::Waiting(reason)) = status else {
            bail!("that source is not waiting for a login, captcha or block page");
        };
        let (source_id, url) = page_of(&reason).context("that source is not waiting for a login, captcha or block page")?;
        let url = checked_url(url).context("the page to open")?.to_string();
        let session_id = self.store.get_source(source_id)?.and_then(|s| s.session_id);

        if let Some(mut held) = take_window(thread) {
            if held.is_alive().await {
                open_windows().lock().unwrap_or_else(|e| e.into_inner()).insert(thread.to_string(), held);
                return Ok(());
            }
            // The user closed it themselves: release the profile and open a fresh one.
            held.close().await;
        }
        let held = match session_id {
            Some(id) => Held::Session(self.open_in_session(id, &url).await?),
            None => Held::Temp(Box::new(open_visible(&self.paths, &url).await?)),
        };
        tracing::info!(thread, source_id, with_session = session_id.is_some(), "browser opened for the user");
        open_windows().lock().unwrap_or_else(|e| e.into_inner()).insert(thread.to_string(), held);
        Ok(())
    }

    /// A visible browser on session `id`'s profile, showing `url`.
    async fn open_in_session(&self, id: i64, url: &str) -> Result<BrowserHandle> {
        let handle = self.sessions().launch(id, false).await?;
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
        Ok(handle)
    }
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
