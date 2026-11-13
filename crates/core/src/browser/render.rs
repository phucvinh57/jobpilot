//! Rendering pages for job discovery, and the visible window for a challenge hand-off.
//!
//! - [`Renderer::render`]: a page in a headless browser, either with a throwaway profile or in the
//!   profile of a managed session (the source's bound session: its cookies are what a login or a
//!   solved check left behind). The result says whether a login, captcha or block page was shown
//!   instead ([`challenge`](super::challenge)); nothing here tries to get past one.
//! - [`open_visible`]: a **visible** browser on a page, with a throwaway profile, for a source
//!   without a session. The user looks at the page, or deals with it, themselves.
//!
//! Only http(s) addresses are opened, and (unless private hosts are allowed, as in tests) not machine-
//! local or private-network ones. The browser follows redirects itself, so that check covers the
//! address given, not where a redirect leads; nothing read this way leaves the machine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chromiumoxide::Browser;
use futures::StreamExt;
use tokio::task::JoinHandle;

use super::fetch::{checked_url, finish_page, Rendered, TempProfile, LOAD_TIMEOUT, RENDER_TIMEOUT};
use super::session::{browser_config, prepare_profile, SessionManager};
use super::chrome;
use crate::net::http::public_http_url;
use crate::{Core, Paths};

/// Renders pages for discovery. Cheap to clone (the clones share their per-session locks).
#[derive(Clone)]
pub struct Renderer {
    inner: Arc<Inner>,
}

struct Inner {
    paths: Paths,
    sessions: SessionManager,
    allow_private_hosts: bool,
    /// One render at a time per session: a profile has one live browser.
    session_locks: Mutex<HashMap<i64, Arc<tokio::sync::Mutex<()>>>>,
}

impl Renderer {
    /// A renderer over the app's browser sessions. `allow_private_hosts` is for tests (fixture pages
    /// are on `127.0.0.1`).
    pub fn new(core: &Core, allow_private_hosts: bool) -> Self {
        Self {
            inner: Arc::new(Inner {
                paths: core.paths.clone(),
                sessions: core.sessions(),
                allow_private_hosts,
                session_locks: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Render `url` headless: in session `session_id`'s profile when there is one, else with a
    /// throwaway profile. The error text never contains `url`.
    pub async fn render(&self, session_id: Option<i64>, url: &str) -> Result<Rendered> {
        let url = public_http_url(url, self.inner.allow_private_hosts)?.to_string();
        let rendered = match session_id {
            None => super::fetch::render_checked(&self.inner.paths, &url).await,
            Some(id) => self.render_in_session(id, &url).await,
        };
        rendered.map_err(|e| scrub(e, &url))
    }

    async fn render_in_session(&self, id: i64, url: &str) -> Result<Rendered> {
        let lock = {
            let mut locks = self.inner.session_locks.lock().unwrap_or_else(|e| e.into_inner());
            locks.entry(id).or_default().clone()
        };
        let _one_at_a_time = lock.lock().await;
        let handle = self.inner.sessions.launch(id, true).await?;
        let result = tokio::time::timeout(RENDER_TIMEOUT, async {
            let page = handle.new_page_with_timeout(url, LOAD_TIMEOUT).await?;
            let rendered = finish_page(&page, url).await;
            let _ = page.close().await;
            rendered
        })
        .await
        .unwrap_or_else(|_| Err(anyhow!("the browser did not render the page within {} seconds (timed out)", RENDER_TIMEOUT.as_secs())));
        let _ = handle.close().await;
        result
    }
}

/// `e` as one message without the page's address, and with Chrome's network errors worded so the
/// app classifies them as network problems (Retry).
fn scrub(e: anyhow::Error, url: &str) -> anyhow::Error {
    let mut text = format!("{e:#}").replace(url, "<page>");
    if let Some(parsed) = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string)) {
        text = text.replace(&parsed, "<host>");
    }
    const NETWORK: [&str; 6] = ["CONNECTION_", "NAME_NOT_RESOLVED", "TIMED_OUT", "INTERNET_DISCONNECTED", "NETWORK_", "ADDRESS_UNREACHABLE"];
    if text.contains("net::ERR_") && NETWORK.iter().any(|c| text.contains(&format!("net::ERR_{c}"))) {
        text = format!("network error: {text}");
    }
    anyhow!("{text}")
}

/// A visible browser window on a throwaway profile, for the user to look at a page. Closing it (or
/// dropping it) ends the browser and deletes the profile.
pub struct Visible {
    browser: Browser,
    driver: JoinHandle<()>,
    _profile: TempProfile,
}

/// Open a visible browser on `url` (http(s) only).
pub async fn open_visible(paths: &Paths, url: &str) -> Result<Visible> {
    let url = checked_url(url)?;
    let chrome = chrome::resolve(paths)
        .context("Chrome not found: install Chrome/Chromium or download the browser in Settings > Browser sessions")?;
    let profile = TempProfile::create(paths)?;
    prepare_profile(profile.path())?;
    let config = browser_config(&chrome.path, profile.path(), false, Some(&url))?;
    let (browser, mut handler) =
        Browser::launch(config).await.with_context(|| format!("launching {}", chrome.path.display()))?;
    let driver = tokio::spawn(async move { while handler.next().await.is_some() {} });
    Ok(Visible { browser, driver, _profile: profile })
}

impl Visible {
    /// Is the window still open? `false` once the user closed it.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.browser.try_wait(), Ok(None))
    }

    /// Close the window.
    pub async fn close(mut self) {
        let _ = self.browser.close().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.browser.wait()).await;
        let _ = self.browser.kill().await;
        self.driver.abort();
    }
}

impl Drop for Visible {
    fn drop(&mut self) {
        self.driver.abort();
        // The browser (kill on drop) goes with it.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_lose_the_page_address_and_chrome_network_errors_become_network_problems() {
        let e = anyhow!("navigating to https://jobs.acme.test/careers").context("opening the page");
        let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
        assert!(!text.contains("acme"), "{text}");
        let e = anyhow!("net::ERR_CONNECTION_REFUSED at https://jobs.acme.test/careers");
        let text = format!("{:#}", scrub(e, "https://jobs.acme.test/careers"));
        assert_eq!(crate::health::classify(&text), crate::health::Problem::Network, "{text}");
        let e = anyhow!("Chrome not found: install Chrome/Chromium");
        assert_eq!(crate::health::classify(&format!("{:#}", scrub(e, "https://a.test/"))), crate::health::Problem::NoChrome);
        let e = anyhow!("net::ERR_CERT_AUTHORITY_INVALID");
        assert_eq!(crate::health::classify(&format!("{:#}", scrub(e, "https://a.test/"))), crate::health::Problem::Other);
    }
}
