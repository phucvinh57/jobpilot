//! The browser tab of one application: a tab in the user's own Chrome, opened through the JobPilot
//! extension ([`Bridge::open`]).
//!
//! - an ATS that needs a login (a browser session matched it) opens in [`Profile::Signed`], the
//!   user's normal profile where they are signed in;
//! - every other form opens in [`Profile::Private`], the JobPilot Incognito window, so no cookies of
//!   an application stay behind.
//!
//! The tab opens in the background JobPilot window (`focus: false`) and has no guard (apply pages
//! are the job's own form). While JobPilot drives it, JavaScript dialogs the page opens (`alert`,
//! `confirm`, `beforeunload` after a submit, ...) are accepted at once, since an open dialog blocks
//! every script call; [`FormBrowser::hand_over`] stops that when the user takes the tab over.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use super::bridge::{Bridge, BridgeEvent};
use super::tab::{OpenOptions, Profile, Tab};

/// How long the first load of the form may take.
const LOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// The tab of one application. Cheap to clone; all clones share the tab.
#[derive(Clone, Debug)]
pub struct FormBrowser {
    tab: Tab,
    session_id: Option<i64>,
    /// JobPilot answers the page's dialogs (false once the user took the tab over).
    driving: Arc<AtomicBool>,
    /// Chrome stopped letting JobPilot drive the tab (the user cancelled the debugging bar).
    detached: Arc<AtomicBool>,
}

impl FormBrowser {
    /// Open `url` in a background tab and wait (at most 30 s) for it to load: the user's signed-in
    /// profile when `session_id` is set (a browser session covers the ATS), else the JobPilot
    /// Incognito window. Fails with a [`BridgeError`](super::BridgeError) when the extension is not
    /// connected or may not open Incognito tabs.
    pub async fn open(bridge: &Bridge, url: &str, session_id: Option<i64>, focus: bool) -> Result<Self> {
        let profile = if session_id.is_some() { Profile::Signed } else { Profile::Private };
        // Subscribed before the tab exists, so a dialog during the first load is seen too.
        let events = bridge.subscribe();
        let tab = bridge.open(url, OpenOptions { profile, focus, guard: None }).await?;
        let browser = FormBrowser {
            tab,
            session_id,
            driving: Arc::new(AtomicBool::new(!focus)),
            detached: Arc::new(AtomicBool::new(false)),
        };
        tokio::spawn(watch(browser.clone(), events));
        tracing::info!(tab = browser.tab.id(), ?profile, "apply: form tab opened");
        if !focus {
            browser.tab.wait_loaded(LOAD_TIMEOUT).await;
        }
        Ok(browser)
    }

    /// The tab, to drive it.
    pub fn tab(&self) -> &Tab {
        &self.tab
    }

    pub fn session_id(&self) -> Option<i64> {
        self.session_id
    }

    /// Can JobPilot still drive the tab (false after the user closed it, or stopped the debugging)?
    pub fn is_alive(&self) -> bool {
        self.tab.is_open() && !self.detached.load(Ordering::SeqCst)
    }

    /// Is the tab still open (the user may finish the form there)?
    pub fn is_open(&self) -> bool {
        self.tab.is_open()
    }

    /// The user takes the tab over: JobPilot no longer answers its dialogs.
    pub fn hand_over(&self) {
        self.driving.store(false, Ordering::SeqCst);
    }

    /// Bring the tab and its window to the front.
    pub async fn focus(&self) -> Result<()> {
        self.tab.focus().await
    }

    /// Close the tab. Idempotent.
    pub async fn close(&self) {
        self.hand_over();
        self.tab.close().await;
    }
}

/// Until the tab closes: accept the dialogs it opens while JobPilot drives it, and note when Chrome
/// stops letting JobPilot drive it.
async fn watch(browser: FormBrowser, mut events: tokio::sync::broadcast::Receiver<BridgeEvent>) {
    let id = browser.tab.id();
    loop {
        let event = tokio::select! {
            _ = browser.tab.closed() => return,
            event = events.recv() => event,
        };
        match event {
            Ok(BridgeEvent::Dialog { tab, kind, .. }) if tab == id => {
                if !browser.driving.load(Ordering::SeqCst) {
                    continue;
                }
                // The message is page text: never logged.
                tracing::info!(tab, kind = kind.as_str(), "apply: accepting a page dialog");
                let answer = json!({ "accept": true, "promptText": "" });
                if let Err(e) = browser.tab.cdp("Page.handleJavaScriptDialog", answer).await {
                    tracing::warn!(tab, "apply: the page dialog could not be answered: {e:#}");
                }
            }
            Ok(BridgeEvent::Detached { tab, .. }) if tab == id => {
                tracing::warn!(tab, "apply: the browser stopped letting JobPilot drive the form tab");
                browser.detached.store(true, Ordering::SeqCst);
            }
            Ok(BridgeEvent::TabClosed(tab)) if tab == id => return,
            Ok(_) | Err(RecvError::Lagged(_)) => {}
            Err(RecvError::Closed) => return,
        }
    }
}
