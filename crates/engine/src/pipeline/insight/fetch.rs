//! The fetch of one company page. The address comes from code, but a redirect or a DNS answer can
//! still lead somewhere that must not be read, so every hop is checked: http(s) only, a public DNS
//! name (no IP literal, `localhost`, single-label or `.local`-style name), never https -> http.
//! The resolver drops loopback, private, link-local and other special-use addresses, so a
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
    // `localhost.` and `a.local.` are the same names as without the root dot.
    let host = host.trim_end_matches('.');
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
                let seg = v6.segments();
                let first = seg[0];
                let embedded = |hi: u16, lo: u16| {
                    let [a, b] = hi.to_be_bytes();
                    let [c, d] = lo.to_be_bytes();
                    is_public_ip(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)))
                };
                // NAT64 (64:ff9b::/96), 6to4 (2002::/16) and IPv4-compatible (::/96) addresses carry
                // an IPv4 address that decides where they lead; 2001:db8::/32 is documentation.
                if first == 0x64 && seg[1] == 0xff9b && seg[2..6] == [0; 4] {
                    return embedded(seg[6], seg[7]);
                }
                if first == 0x2002 {
                    return embedded(seg[1], seg[2]);
                }
                if seg[..6] == [0; 6] {
                    return embedded(seg[6], seg[7]);
                }
                !(v6.is_loopback()
                    || (first == 0x2001 && seg[1] == 0x0db8)
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

/// May the fetch follow a redirect from the URLs in `previous` (oldest first) to `next`? Not past
/// [`MAX_REDIRECTS`], not to an address the policy refuses, and not from https down to http.
fn follow_redirect(previous: &[Url], next: &Url, policy: Policy) -> bool {
    previous.len() < MAX_REDIRECTS
        && policy.allows(next)
        && !(next.scheme() == "http" && previous.iter().any(|u| u.scheme() == "https"))
}

fn client(policy: Policy) -> Result<reqwest::Client> {
    let redirects = reqwest::redirect::Policy::custom(move |attempt| {
        if follow_redirect(attempt.previous(), attempt.url(), policy) {
            attempt.follow()
        } else {
            // The redirect answer itself comes back and is not a page.
            attempt.stop()
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
    let html = decode(&body, &content_type);
    let text = tokio::task::spawn_blocking(move || extract_html(&html, Some(&base))).await?;
    Ok((text.chars().count() >= MIN_PAGE_CHARS).then_some(text))
}

/// The body as text in the charset the `Content-Type` declares (a byte order mark wins), else UTF-8;
/// bytes that do not fit become U+FFFD. A `<meta charset>` is not looked at.
fn decode(body: &[u8], content_type: &str) -> String {
    let declared = content_type
        .split(';')
        .filter_map(|part| part.trim().strip_prefix("charset="))
        .find_map(|label| encoding_rs::Encoding::for_label(label.trim_matches(['"', '\'']).trim().as_bytes()));
    declared.unwrap_or(encoding_rs::UTF_8).decode(body).0.into_owned()
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
            "http://localhost./x",
            "http://printer.local./",
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
            "64:ff9b::a00:1",
            "64:ff9b::a9fe:a9fe",
            "2002:7f00:1::",
            "::10.0.0.1",
            "2001:db8::1",
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

    #[test]
    fn a_redirect_never_goes_from_https_down_to_http() {
        let url = |s: &str| Url::parse(s).unwrap();
        let ok = |previous: &[&str], next: &str| {
            let previous: Vec<Url> = previous.iter().map(|u| url(u)).collect();
            follow_redirect(&previous, &url(next), Policy::PUBLIC)
        };
        assert!(ok(&["https://acme.com/"], "https://www.acme.com/"));
        assert!(ok(&["http://acme.com/"], "https://acme.com/"), "upgrades are fine");
        assert!(ok(&["http://acme.com/"], "http://www.acme.com/"));
        assert!(!ok(&["https://acme.com/"], "http://acme.com/"), "downgrade");
        assert!(!ok(&["https://acme.com/", "https://www.acme.com/"], "http://cdn.acme.com/"));
        // Once the chain has been https, a later http hop is a downgrade even through another http one.
        assert!(!ok(&["https://acme.com/", "http://a.example.com/"], "http://b.example.com/"));
        // The other rules still apply: refused hosts and the hop limit.
        assert!(!ok(&["https://acme.com/"], "https://localhost/"));
        let long: Vec<String> = (0..MAX_REDIRECTS).map(|i| format!("https://acme.com/{i}")).collect();
        let long: Vec<&str> = long.iter().map(String::as_str).collect();
        assert!(!ok(&long, "https://acme.com/next"));
        assert!(ok(&long[1..], "https://acme.com/next"));
    }

    #[test]
    fn bodies_are_decoded_with_their_declared_charset() {
        assert_eq!(decode("café – ünï".as_bytes(), "text/html"), "café – ünï");
        assert_eq!(decode("café".as_bytes(), "text/html; charset=utf-8"), "café");
        assert_eq!(decode(b"caf\xe9", "text/html; charset=iso-8859-1"), "café");
        assert_eq!(decode(b"caf\xe9", "text/html; charset=\"ISO-8859-1\""), "café");
        assert_eq!(decode(b"\x93quoted\x94", "text/html; charset=windows-1252"), "\u{201c}quoted\u{201d}");
        assert_eq!(decode(b"\x82\xb1\x82\xf1", "text/html;charset=shift_jis"), "こん");
        // A BOM wins over the declaration.
        assert_eq!(decode(b"\xef\xbb\xbfcaf\xc3\xa9", "text/html; charset=iso-8859-1"), "café");
        // No or unknown charset: UTF-8, with bad bytes replaced.
        assert_eq!(decode(b"caf\xe9", "text/html"), "caf\u{fffd}");
        assert_eq!(decode(b"caf\xe9", "text/html; charset=nonsense"), "caf\u{fffd}");
    }
}
