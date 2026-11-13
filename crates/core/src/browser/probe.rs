//! "Am I still logged in?" health probe and the helpers behind it.

use std::collections::BTreeSet;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use chromiumoxide::cdp::browser_protocol::network::Cookie;
use regex::Regex;

use super::session::BrowserHandle;

/// How long a probe waits for the start page to finish loading.
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Extra time after load for client-side redirects to a login page (SPAs).
const SETTLE: Duration = Duration::from_millis(1500);

/// Visible `<input type=password>` anywhere in the page.
const PASSWORD_JS: &str = r#"(() => Array.from(document.querySelectorAll('input[type="password"]')).some(e => {
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
    /// All cookie domains in the profile (sorted, no leading dot).
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

/// Visit `start_url` in the (headless) browser and judge whether the profile is logged in.
/// Errors when the page cannot be reached at all (the caller keeps the previous status then).
pub(crate) async fn probe(handle: &BrowserHandle, start_url: &str) -> Result<ProbeResult> {
    let page = handle.new_page_with_timeout(start_url, LOAD_TIMEOUT).await.context("opening the start page")?;
    tokio::time::sleep(SETTLE).await;

    let final_url = page.url().await.ok().flatten().unwrap_or_default();
    let cookies = handle.cookies().await.context("reading cookies")?;
    let password_input = match page.evaluate_expression(PASSWORD_JS).await {
        Ok(v) => v.into_value::<bool>().unwrap_or(false),
        Err(_) => false,
    };
    let _ = page.close().await;

    let host = host_of(start_url).unwrap_or_default();
    Ok(ProbeResult {
        has_cookies: cookies.iter().any(|c| cookie_domain_matches(&c.domain, &host)),
        login_url: looks_like_login_url(&final_url),
        password_input,
        domains: cookie_domains(&cookies),
        final_url,
    })
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

/// Sorted, de-duplicated cookie domains without the leading dot.
pub fn cookie_domains(cookies: &[Cookie]) -> Vec<String> {
    cookies
        .iter()
        .map(|c| c.domain.trim_start_matches('.').to_ascii_lowercase())
        .filter(|d| !d.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
    fn host_extraction() {
        assert_eq!(host_of("https://WWW.LinkedIn.com/login").as_deref(), Some("www.linkedin.com"));
        assert_eq!(host_of("not a url"), None);
    }
}
