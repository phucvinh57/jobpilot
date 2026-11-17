//! "Am I still signed in?": the check of a session (a site the user is signed in to in their own
//! Chrome) and the helpers behind it (the login-page tests are shared with [`super::challenge`]).

use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use regex::Regex;

use super::bridge::Bridge;
use super::tab::{OpenOptions, Profile, Tab};
use crate::mail::classify::registrable_domain;

/// How long a probe waits for the start page to finish loading.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra time after load for client-side redirects to a login page (SPAs).
const SETTLE: Duration = Duration::from_millis(1500);

/// Visible `<input type=password>` anywhere in the page.
pub(super) const PASSWORD_JS: &str = r#"(() => Array.from(document.querySelectorAll('input[type="password"]')).some(e => {
    const r = e.getBoundingClientRect(); const s = getComputedStyle(e);
    return r.width > 0 && r.height > 0 && s.visibility !== 'hidden' && s.display !== 'none';
}))()"#;

/// What the probe saw (kept for logging/tests; the verdict is [`ProbeResult::logged_in`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeResult {
    pub final_url: String,
    pub has_cookies: bool,
    pub login_url: bool,
    pub password_input: bool,
    /// The browser's cookie domains for the site (sorted, no leading dot).
    pub domains: Vec<String>,
}

impl ProbeResult {
    pub fn logged_in(&self) -> bool {
        decide(self.has_cookies, self.login_url, self.password_input)
    }
}

/// Logged in = cookies for the start domain AND the page does not look like a login page.
pub fn decide(has_cookies: bool, login_url: bool, password_input: bool) -> bool {
    has_cookies && !login_url && !password_input
}

/// Open `start_url` in a background tab of the user's own (signed-in) browser profile and judge
/// whether they are signed in: the browser holds cookies for the site, and the page is not a login
/// page. The tab is closed again. Errors when the browser or the page cannot be reached at all (the
/// caller keeps the previous status then). Only cookie *domains* are read, never cookies.
pub(crate) async fn probe(bridge: &Bridge, start_url: &str) -> Result<ProbeResult> {
    let tab = bridge
        .open(start_url, OpenOptions { profile: Profile::Signed, focus: false, guard: None })
        .await
        .context("opening the start page")?;
    let seen = look(&tab).await;
    tab.close().await;
    let (final_url, password_input) = seen?;

    let host = host_of(start_url).unwrap_or_default();
    let domains = bridge.cookie_domains(&cookie_scope(&host), false).await.context("reading the cookie domains")?;
    Ok(ProbeResult {
        has_cookies: !domains.is_empty(),
        login_url: looks_like_login_url(&final_url),
        password_input,
        domains,
        final_url,
    })
}

/// Where the page ended up and whether it shows a password field.
async fn look(tab: &Tab) -> Result<(String, bool)> {
    tab.wait_loaded(LOAD_TIMEOUT).await;
    tokio::time::sleep(SETTLE).await;
    let final_url = tab.url().await.context("reading the page address")?;
    let password_input = tab.eval_as::<bool>(PASSWORD_JS).await.unwrap_or(false);
    Ok((final_url, password_input))
}

/// The domain whose cookies say the user is signed in to `host`: its registrable domain
/// (`www.linkedin.com` -> `linkedin.com`, so a login cookie set for the parent counts), the host
/// itself for an address literal.
pub fn cookie_scope(host: &str) -> String {
    let host = host.trim_start_matches('.').to_ascii_lowercase();
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return host;
    }
    registrable_domain(&host)
}

fn login_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // A whole token delimited by URL punctuation, so `/author/x` or `/loginfo` do not match but
        // `/login`, `/uas/login-submit`, `/sign-in`, `?next=/auth` and `/checkpoint/challenge` do.
        Regex::new(
            r"(?i)(^|[/._?&=#-])(login|log-in|signin|sign-in|signon|sign-on|auth|authenticate|authwall|authorize|oauth|sso|checkpoint)($|[/._?&=#-])",
        )
        .expect("valid regex")
    })
}

/// Does this URL look like a login / auth / challenge page? Matches the path, query and fragment, and a
/// `login.`/`auth.`/`signin.`/`sso.` host label; never the rest of the host name.
pub fn looks_like_login_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else { return login_re().is_match(url) };
    if let Some(host) = parsed.host_str() {
        let first = host.split('.').next().unwrap_or("");
        if matches!(first, "login" | "auth" | "signin" | "sso") {
            return true;
        }
    }
    let mut rest = parsed.path().to_string();
    if let Some(q) = parsed.query() {
        rest.push('?');
        rest.push_str(q);
    }
    if let Some(f) = parsed.fragment() {
        rest.push('#');
        rest.push_str(f);
    }
    login_re().is_match(&rest)
}

/// Lower-cased host of a URL.
pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok()?.host_str().map(|h| h.to_ascii_lowercase())
}

/// Does a cookie with `cookie_domain` (e.g. `.linkedin.com`) belong to `host` (e.g. `www.linkedin.com`)?
/// True when one is a dot-boundary suffix of the other.
pub fn cookie_domain_matches(cookie_domain: &str, host: &str) -> bool {
    let d = cookie_domain.trim_start_matches('.').to_ascii_lowercase();
    let h = host.trim_start_matches('.').to_ascii_lowercase();
    if d.is_empty() || h.is_empty() {
        return false;
    }
    d == h || h.ends_with(&format!(".{d}")) || d.ends_with(&format!(".{h}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_urls_are_detected() {
        for url in [
            "https://www.linkedin.com/login",
            "https://www.linkedin.com/uas/login?session_redirect=%2Ffeed",
            "https://www.linkedin.com/checkpoint/challenge/abc",
            "https://www.linkedin.com/authwall?trk=x",
            "https://example.com/users/sign-in",
            "https://example.com/signin",
            "https://example.com/account/login-submit",
            "https://example.com/app?next=/auth/callback",
            "https://example.com/oauth/authorize?client_id=1",
            "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            "https://auth.example.com/",
            "https://example.com/#/login",
        ] {
            assert!(looks_like_login_url(url), "{url} should look like a login page");
        }
    }

    #[test]
    fn normal_urls_are_not_login_pages() {
        for url in [
            "https://www.linkedin.com/feed/",
            "https://www.linkedin.com/jobs/view/123",
            "https://example.com/author/jane",
            "https://example.com/authors",
            "https://author.example.com/home",
            "https://loginfo.example.com/",
            "https://example.com/blog/logging-in-rust-apps",
            "https://boards.greenhouse.io/acme/jobs/1",
            "about:blank",
        ] {
            assert!(!looks_like_login_url(url), "{url} should not look like a login page");
        }
    }

    #[test]
    fn cookie_domain_matching() {
        assert!(cookie_domain_matches(".linkedin.com", "www.linkedin.com"));
        assert!(cookie_domain_matches("www.linkedin.com", "www.linkedin.com"));
        assert!(cookie_domain_matches("www.linkedin.com", "linkedin.com"));
        assert!(!cookie_domain_matches(".notlinkedin.com", "www.linkedin.com"));
        assert!(!cookie_domain_matches(".example.com", "www.linkedin.com"));
        assert!(!cookie_domain_matches("", "www.linkedin.com"));
    }

    #[test]
    fn verdict_needs_cookies_and_no_login_page() {
        assert!(decide(true, false, false));
        assert!(!decide(false, false, false), "no cookies for the domain");
        assert!(!decide(true, true, false), "redirected to a login url");
        assert!(!decide(true, false, true), "password field visible");
    }

    #[test]
    fn the_cookie_scope_is_the_registrable_domain() {
        assert_eq!(cookie_scope("www.linkedin.com"), "linkedin.com");
        assert_eq!(cookie_scope("careers.acme.co.uk"), "acme.co.uk");
        assert_eq!(cookie_scope("site.test"), "site.test");
        assert_eq!(cookie_scope("127.0.0.1"), "127.0.0.1");
        assert_eq!(cookie_scope("[::1]"), "[::1]");
    }

    #[test]
    fn host_extraction() {
        assert_eq!(host_of("https://WWW.LinkedIn.com/login").as_deref(), Some("www.linkedin.com"));
        assert_eq!(host_of("not a url"), None);
    }
}
