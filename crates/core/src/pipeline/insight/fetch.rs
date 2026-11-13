//! The fetch of one company page, with rules of its own: the address comes from code, but a
//! redirect (or a DNS answer) can still lead somewhere that must not be read, so every hop is
//! checked, not only the first.
//!
//! A page is fetched only when its scheme is http(s) and its host is a public DNS name: no IP
//! literal, `localhost`, single-label or `.local`-style name. The same check runs on every
//! redirect (a refused hop ends the fetch), and the connection itself only goes to public
//! addresses: the resolver drops loopback, private, link-local and other special-use ranges, so a
//! public-looking name that points inside the network is refused too. (Behind an HTTP proxy the
//! proxy resolves names, so the name check is the part that still holds.)

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use url::{Host, Url};

use crate::pipeline::ingest::{extract_html, MIN_PAGE_CHARS, USER_AGENT};

/// Longest a page may take, DNS and body included.
const PAGE_TIMEOUT: Duration = Duration::from_secs(12);
/// Most bytes read of one page; the rest is dropped.
const MAX_BYTES: usize = 1 << 20;
const MAX_REDIRECTS: usize = 5;

/// Which addresses may be fetched. Production code uses [`Policy::PUBLIC`] only.
#[derive(Clone, Copy, Debug)]
pub(super) struct Policy {
    allow_ip_literals: bool,
}

impl Policy {
    /// Public DNS names over http(s).
    pub(super) const PUBLIC: Policy = Policy { allow_ip_literals: false };

    /// Tests only: also IP literals, so that fixture servers on `127.0.0.1` can be read.
    #[cfg(test)]
    pub(super) const FIXTURES: Policy = Policy { allow_ip_literals: true };

    /// May `url` be fetched (as a first request or as a redirect target)?
    pub(super) fn allows(&self, url: &Url) -> bool {
        matches!(url.scheme(), "http" | "https")
            && match url.host() {
                Some(Host::Domain(name)) => is_public_name(name),
                Some(Host::Ipv4(_) | Host::Ipv6(_)) => self.allow_ip_literals,
                None => false,
            }
    }
}

/// A DNS name on the public internet, as far as its spelling tells.
fn is_public_name(host: &str) -> bool {
    const PRIVATE_SUFFIXES: &[&str] = &[".local", ".localhost", ".internal", ".lan", ".home", ".corp", ".intranet"];
    host.contains('.')
        && host != "localhost"
        && !PRIVATE_SUFFIXES.iter().any(|s| host.ends_with(s))
        && !host.starts_with('.')
}

/// Is `ip` a routable public address (not loopback, private, link-local, shared, reserved)?
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (b & 0xc0) == 64)) // 100.64.0.0/10, carrier-grade NAT
        }
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => is_public_ip(IpAddr::V4(v4)),
            None => {
                let first = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || v6.is_multicast()
                    || (first & 0xfe00) == 0xfc00 // fc00::/7, unique local
                    || (first & 0xffc0) == 0xfe80) // fe80::/10, link-local
            }
        },
    }
}

/// The addresses of `host`, public ones only; an error when it has none.
async fn public_addrs(host: &str) -> std::io::Result<Vec<SocketAddr>> {
    let public: Vec<SocketAddr> = tokio::net::lookup_host((host, 0)).await?.filter(|a| is_public_ip(a.ip())).collect();
    if public.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "the name has no public address"));
    }
    Ok(public)
}

/// The DNS resolver of the client: a name that only resolves to private space cannot be connected to.
struct PublicOnly;

impl Resolve for PublicOnly {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_string();
        Box::pin(async move { Ok(Box::new(public_addrs(&host).await?.into_iter()) as Addrs) })
    }
}

fn client(policy: Policy) -> Result<reqwest::Client> {
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS || !policy.allows(attempt.url()) {
            // The redirect answer itself comes back and is not a page.
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .redirect(redirects)
        .dns_resolver(Arc::new(PublicOnly))
        .build()
        .context("building the HTTP client")
}

/// The readable text of the HTML page at `url`, or `None` for anything else: an address the policy
/// refuses, a failed or slow request, an answer that is not success (a 3xx that was not followed,
/// 401/403/429 bot or login walls, 404), a page that is not HTML, or one with too little text to be
/// a page rather than a script shell. At most one MiB is read.
pub(super) async fn fetch_text(url: &str, policy: Policy) -> Option<String> {
    let url = Url::parse(url.trim()).ok().filter(|u| policy.allows(u))?;
    tokio::time::timeout(PAGE_TIMEOUT, get(url, policy)).await.ok()?.ok().flatten()
}

async fn get(url: Url, policy: Policy) -> Result<Option<String>> {
    let mut response = client(policy)?
        .get(url)
        .header("Accept", "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await?;
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !response.status().is_success() || !(content_type.is_empty() || content_type.contains("html")) {
        return Ok(None);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        body.extend_from_slice(&chunk);
        if body.len() >= MAX_BYTES {
            body.truncate(MAX_BYTES);
            break;
        }
    }
    let base = response.url().to_string();
    let html = String::from_utf8_lossy(&body).into_owned();
    let text = tokio::task::spawn_blocking(move || extract_html(&html, Some(&base))).await?;
    Ok((text.chars().count() >= MIN_PAGE_CHARS).then_some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(policy: Policy, url: &str) -> bool {
        policy.allows(&Url::parse(url).unwrap())
    }

    #[test]
    fn only_public_dns_names_over_http_are_allowed() {
        for ok in ["https://acme.com/about", "http://careers.acme.co.uk/", "https://acme.com:8443/x"] {
            assert!(allowed(Policy::PUBLIC, ok), "{ok}");
        }
        for refused in [
            "ftp://acme.com/x",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "http://localhost/x",
            "http://localhost:8080/",
            "http://127.0.0.1/",
            "http://127.1/",
            "http://0x7f.0.0.1/",
            "http://2130706433/",
            "http://10.0.0.5/",
            "http://192.168.1.5/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]:8080/",
            "http://[::ffff:127.0.0.1]/",
            "http://intranet/wiki",
            "http://wiki.corp/x",
            "http://printer.local/",
            "http://db.internal/",
        ] {
            assert!(!allowed(Policy::PUBLIC, refused), "{refused}");
        }
        // The test policy adds IP literals and nothing else.
        assert!(allowed(Policy::FIXTURES, "http://127.0.0.1:8080/"));
        assert!(!allowed(Policy::FIXTURES, "http://localhost:8080/"));
        assert!(!allowed(Policy::FIXTURES, "http://intranet/"));
        assert!(!allowed(Policy::FIXTURES, "ftp://127.0.0.1/"));
    }

    #[test]
    fn special_use_addresses_are_not_public() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "240.0.0.1",
            "192.0.2.1",
            "::1",
            "::",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(ip(private)), "{private}");
        }
        for public in ["8.8.8.8", "1.1.1.1", "172.32.0.1", "100.128.0.1", "2606:4700:4700::1111", "::ffff:8.8.8.8"] {
            assert!(is_public_ip(ip(public)), "{public}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_only_to_private_space_has_no_public_address() {
        // `localhost` is in /etc/hosts, so this needs no network.
        let err = public_addrs("localhost").await.unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied, "{err}");
    }
}
