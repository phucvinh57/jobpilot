//! Browsers for auto-apply (P5): either a managed session (a logged-in profile, launched through the
//! [`SessionManager`]) or a throwaway profile for ATSes that need no login (Greenhouse, Lever, Ashby).
//!
//! Both are driven through the same four calls: [`FormBrowser::new_page`], [`is_alive`], [`close`] and
//! [`is_headless`]. A throwaway profile lives in `<data_dir>/tmp/apply-<tag>/` and is deleted on close,
//! so no cookies of an application stay behind.
//!
//! [`is_alive`]: FormBrowser::is_alive
//! [`close`]: FormBrowser::close
//! [`is_headless`]: FormBrowser::is_headless

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use chromiumoxide::error::CdpError;
use chromiumoxide::{Browser, Page};
use futures::StreamExt;

use super::chrome;
use super::session::{browser_config, prepare_profile, wait_loaded, NAVIGATE_TIMEOUT};
use super::{BrowserHandle, SessionManager};
use crate::Paths;

/// A running browser for one application. Cheap to clone; all clones share the browser.
#[derive(Clone)]
pub enum FormBrowser {
    /// A managed session's profile.
    Session(BrowserHandle),
    /// A throwaway profile.
    Temp(TempBrowser),
}

impl FormBrowser {
    /// Launch on a managed session (fails when the profile is already open).
    pub async fn launch_session(mgr: &SessionManager, session_id: i64, headless: bool) -> Result<Self> {
        Ok(FormBrowser::Session(mgr.launch(session_id, headless).await?))
    }

    /// Launch on a throwaway profile named by `tag` (reusing the directory of an earlier launch with
    /// the same tag, so a headed handoff continues in the profile the headless run used).
    pub async fn launch_temp(paths: &Paths, tag: &str, headless: bool) -> Result<Self> {
        Ok(FormBrowser::Temp(TempBrowser::launch(paths, tag, headless).await?))
    }

    pub fn is_headless(&self) -> bool {
        match self {
            FormBrowser::Session(h) => h.is_headless(),
            FormBrowser::Temp(t) => t.inner.headless,
        }
    }

    pub fn session_id(&self) -> Option<i64> {
        match self {
            FormBrowser::Session(h) => Some(h.session_id()),
            FormBrowser::Temp(_) => None,
        }
    }

    /// Is the Chrome process still running (false after the user closed the window)?
    pub async fn is_alive(&self) -> bool {
        match self {
            FormBrowser::Session(h) => h.is_alive().await,
            FormBrowser::Temp(t) => t.is_alive().await,
        }
    }

    /// Open `url` in a new tab and wait for it to load.
    pub async fn new_page(&self, url: &str) -> Result<Page> {
        match self {
            FormBrowser::Session(h) => h.new_page(url).await,
            FormBrowser::Temp(t) => t.new_page(url).await,
        }
    }

    /// Close the browser and release its profile. Idempotent.
    pub async fn close(&self) {
        match self {
            FormBrowser::Session(h) => {
                let _ = h.close().await;
            }
            FormBrowser::Temp(t) => t.close().await,
        }
    }
}

/// A Chrome on a throwaway profile directory.
#[derive(Clone)]
pub struct TempBrowser {
    inner: Arc<TempInner>,
}

struct TempInner {
    headless: bool,
    dir: PathBuf,
    browser: tokio::sync::Mutex<Option<Browser>>,
    handler_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
    closed: AtomicBool,
}

impl Drop for TempInner {
    fn drop(&mut self) {
        if let Some(task) = self.handler_task.get_mut().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
    }
}

impl TempBrowser {
    async fn launch(paths: &Paths, tag: &str, headless: bool) -> Result<Self> {
        let chrome = chrome::resolve(paths)
            .context("Chrome not found: install Chrome/Chromium or download the browser in Settings")?;
        let tag: String = tag.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        let dir = paths.data_dir.join("tmp").join(format!("apply-{tag}"));
        prepare_profile(&dir)?;
        let dir = std::fs::canonicalize(&dir).with_context(|| format!("resolving {}", dir.display()))?;

        let start_url = (!headless).then_some("about:blank");
        let config = browser_config(&chrome.path, &dir, headless, start_url)?;
        let (browser, mut handler) = Browser::launch(config)
            .await
            .with_context(|| format!("launching {}", chrome.path.display()))?;
        let handler_task = tokio::spawn(async move {
            while let Some(event) = handler.next().await {
                match event {
                    Ok(()) => {}
                    Err(CdpError::Ws(_)) | Err(CdpError::Io(_)) => break,
                    Err(e) => tracing::debug!("chromiumoxide: {e}"),
                }
            }
        });
        Ok(TempBrowser {
            inner: Arc::new(TempInner {
                headless,
                dir,
                browser: tokio::sync::Mutex::new(Some(browser)),
                handler_task: Mutex::new(Some(handler_task)),
                closed: AtomicBool::new(false),
            }),
        })
    }

    async fn is_alive(&self) -> bool {
        let mut guard = self.inner.browser.lock().await;
        match guard.as_mut() {
            None => false,
            Some(b) => matches!(b.try_wait(), Ok(None)),
        }
    }

    async fn new_page(&self, url: &str) -> Result<Page> {
        let guard = self.inner.browser.lock().await;
        let browser = guard.as_ref().context("the browser is closed")?;
        let page = browser.new_page("about:blank").await.context("opening a tab")?;
        drop(guard);
        page.goto(url).await.with_context(|| format!("navigating to {url}"))?;
        wait_loaded(&page, NAVIGATE_TIMEOUT).await;
        Ok(page)
    }

    async fn close(&self) {
        let browser = self.inner.browser.lock().await.take();
        self.inner.closed.store(true, Ordering::SeqCst);
        if let Some(mut browser) = browser {
            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                let _ = browser.close().await;
                let _ = browser.wait().await;
            })
            .await;
            if matches!(browser.try_wait(), Ok(None)) {
                let _ = browser.kill().await;
            }
        }
        if let Some(task) = self.inner.handler_task.lock().unwrap_or_else(|e| e.into_inner()).take() {
            task.abort();
        }
        // Chrome may still be flushing files for a moment.
        for _ in 0..5 {
            match std::fs::remove_dir_all(&self.inner.dir) {
                Ok(()) => return,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        }
    }
}
