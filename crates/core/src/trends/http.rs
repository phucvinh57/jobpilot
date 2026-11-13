//! The one HTTP client of the collectors: read-only, http(s) only, a size cap, and a minimum gap
//! between two requests to the same host (shared by every collector of a run).
//!
//! Redirects are followed here, not by `reqwest`, so that every hop gets the same treatment as the
//! first request: the host check (no machine-local or private addresses unless
//! `allow_private_hosts`), the per-host delay, and the credential rule below. Names are also
//! resolved through a resolver that drops private addresses, so a public name pointing at a private
//! one is refused too.
//!
//! Credential rule: a header other than `Accept` may be a secret (an API key). Such a request is
//! never redirected to another origin, and a POST is never redirected at all; the key is only ever
//! sent to the origin the caller named.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use reqwest::Method;
use serde::de::DeserializeOwned;
use tokio::time::Instant;

use super::config::CollectLimits;

/// Says who is asking. The collectors do not pretend to be a browser.
const USER_AGENT: &str = concat!("JobPilot/", env!("CARGO_PKG_VERSION"), " (personal market-trend reader; read-only)");

/// Why a request produced no body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FetchError {
    /// Not an http(s) URL with a host.
    BadUrl(String),
    /// The server answered with this status (never 2xx).
    Status(u16),
    /// The host is this machine or a private network (and `allow_private_hosts` is off).
    Private,
    /// A redirect that was not followed, with the reason.
    Redirect(String),
    /// The body is larger than the cap (bytes).
    TooLarge(usize),
    /// The request itself failed (connect, TLS, timeout, ...). The text never contains the URL:
    /// feed URLs can carry a private token.
    Transport(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::BadUrl(why) => write!(f, "{why}"),
            FetchError::Status(code) => write!(f, "HTTP {code}"),
            FetchError::Private => write!(f, "addresses on this machine or a private network are not read"),
            FetchError::Redirect(why) => write!(f, "{why}"),
            FetchError::TooLarge(max) => write!(f, "the response is larger than {} MB", max / 1024 / 1024),
            FetchError::Transport(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for FetchError {}

/// A successful (2xx) response, fully read.
#[derive(Clone, Debug)]
pub struct Response {
    /// The address after redirects.
    pub url: String,
    /// Lower-cased `Content-Type`, empty when missing.
    pub content_type: String,
    pub body: Vec<u8>,
}

impl Response {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    /// The body as JSON of type `T`. The error names the kind of mismatch, never the content.
    pub fn json<T: DeserializeOwned>(&self, what: &str) -> Result<T> {
        serde_json::from_slice(&self.body)
            .map_err(|e| anyhow::anyhow!("{what}: unexpected response ({:?} at line {}, column {})", e.classify(), e.line(), e.column()))
    }
}

/// `url` as an http(s) URL with a host, or the reason it is not.
pub fn checked_url(raw: &str) -> Result<url::Url, FetchError> {
    let url = url::Url::parse(raw.trim()).map_err(|_| FetchError::BadUrl("not a valid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(FetchError::BadUrl(format!("only http(s) addresses are read, got a `{}` one", url.scheme())));
    }
    Ok(url)
}

/// Redirects followed for one request.
const MAX_REDIRECTS: usize = 5;

/// Whether a request to this URL (the first one or a redirect target) is allowed.
type HostCheck = Arc<dyn Fn(&url::Url) -> bool + Send + Sync>;

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    delay: Duration,
    max_bytes: usize,
    allowed: HostCheck,
    /// Per host: the earliest time the next request may start.
    slots: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Http {
    /// A client for `limits`: public hosts only, unless `allow_private_hosts`.
    pub fn new(limits: &CollectLimits) -> Result<Self> {
        if limits.allow_private_hosts {
            Self::build(limits, Arc::new(|_| true), false)
        } else {
            Self::build(limits, Arc::new(is_public_host), true)
        }
    }

    /// Like [`new`](Self::new) with `allowed` deciding which URLs may be requested (tests: allow
    /// one local fixture server and refuse another).
    #[cfg(test)]
    pub(crate) fn with_host_check(limits: &CollectLimits, allowed: impl Fn(&url::Url) -> bool + Send + Sync + 'static) -> Result<Self> {
        Self::build(limits, Arc::new(allowed), false)
    }

    fn build(limits: &CollectLimits, allowed: HostCheck, public_dns: bool) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(limits.request_timeout)
            // Redirects are followed in `request`, hop by hop.
            .redirect(reqwest::redirect::Policy::none());
        if public_dns {
            builder = builder.dns_resolver(Arc::new(PublicResolver));
        }
        let client = builder.build().context("building the HTTP client")?;
        Ok(Self { client, delay: limits.host_delay, max_bytes: limits.max_bytes, allowed, slots: Arc::default() })
    }

    /// `GET url`; a non-2xx status is [`FetchError::Status`].
    pub async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, FetchError> {
        self.request(Method::GET, url, headers, None).await
    }

    /// `POST url` with a JSON body. A redirect is an error: the body is never re-sent elsewhere.
    pub async fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &serde_json::Value) -> Result<Response, FetchError> {
        self.request(Method::POST, url, headers, Some(body)).await
    }

    async fn request(&self, method: Method, url: &str, headers: &[(&str, &str)], body: Option<&serde_json::Value>) -> Result<Response, FetchError> {
        let mut url = checked_url(url)?;
        let origin = url.origin();
        // `Accept` is harmless anywhere; any other header may carry a key.
        let credentialed = headers.iter().any(|(name, _)| !name.eq_ignore_ascii_case("accept"));
        for hop in 0..=MAX_REDIRECTS {
            if !(self.allowed)(&url) {
                return Err(FetchError::Private);
            }
            self.wait_turn(url.host_str().unwrap_or_default()).await;
            let mut request = self.client.request(method.clone(), url.clone());
            for (name, value) in headers {
                request = request.header(*name, *value);
            }
            if let Some(body) = body {
                request = request.json(body);
            }
            let resp = request.send().await.map_err(transport)?;
            let status = resp.status().as_u16();
            if !matches!(status, 301 | 302 | 303 | 307 | 308) {
                return self.read(resp).await;
            }

            let target = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|location| url.join(location).ok())
                .ok_or_else(|| FetchError::Redirect(format!("HTTP {status} without a usable Location")))?;
            let target = checked_url(target.as_str())?;
            if method != Method::GET {
                return Err(FetchError::Redirect(format!("HTTP {status} on a POST: not followed")));
            }
            if credentialed && target.origin() != origin {
                return Err(FetchError::Redirect("a request carrying a key was redirected to another site: not followed".into()));
            }
            if hop == MAX_REDIRECTS {
                break;
            }
            url = target;
        }
        Err(FetchError::Redirect("too many redirects".into()))
    }

    /// The body of a non-redirect response, within the size cap.
    async fn read(&self, mut resp: reqwest::Response) -> Result<Response, FetchError> {
        let status = resp.status();
        if !status.is_success() {
            return Err(FetchError::Status(status.as_u16()));
        }
        if resp.content_length().is_some_and(|n| n > self.max_bytes as u64) {
            return Err(FetchError::TooLarge(self.max_bytes));
        }
        let final_url = resp.url().to_string();
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_lowercase();
        let mut body = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(transport)? {
            if body.len() + chunk.len() > self.max_bytes {
                return Err(FetchError::TooLarge(self.max_bytes));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response { url: final_url, content_type, body })
    }

    /// Reserve the next slot of `host` and sleep until it comes (concurrent callers queue up).
    async fn wait_turn(&self, host: &str) {
        if self.delay.is_zero() {
            return;
        }
        let wait = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let slot = slots.get(&host.to_ascii_lowercase()).copied().filter(|t| *t > now).unwrap_or(now);
            slots.insert(host.to_ascii_lowercase(), slot + self.delay);
            slot - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Resolves names like the system does, but drops private and local addresses.
struct PublicResolver;

impl reqwest::dns::Resolve for PublicResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let public = public_addrs(name.as_str()).await?;
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// The public addresses `name` resolves to; an error when there are none.
async fn public_addrs(name: &str) -> Result<Vec<std::net::SocketAddr>, Box<dyn std::error::Error + Send + Sync>> {
    let found = tokio::net::lookup_host((name, 0)).await?;
    let public: Vec<std::net::SocketAddr> = found.filter(|a| ip_is_public(a.ip())).collect();
    if public.is_empty() {
        return Err("the name resolves only to addresses on this machine or a private network".into());
    }
    Ok(public)
}

fn transport(e: reqwest::Error) -> FetchError {
    let e = e.without_url();
    // The chain carries the cause ("connection refused", "operation timed out").
    let mut text = e.to_string();
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    if e.is_timeout() && !text.contains("timed out") {
        text.push_str(" (timed out)");
    }
    FetchError::Transport(text)
}

/// Fail unless `url` is something a collector may fetch from third-party data: http(s), and, unless
/// `allow_private`, not this machine or a private network.
pub fn public_http_url(raw: &str, allow_private: bool) -> Result<url::Url> {
    let url = checked_url(raw)?;
    if !allow_private && !is_public_host(&url) {
        bail!("addresses on this machine or a private network are not read");
    }
    Ok(url)
}

fn is_public_host(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            // A single label ("intranet") only resolves inside a network.
            name.contains('.')
                && name != "localhost"
                && !(name.ends_with(".localhost") || name.ends_with(".local") || name.ends_with(".internal") || name.ends_with(".lan"))
        }
        Some(url::Host::Ipv4(ip)) => ipv4_is_public(ip),
        Some(url::Host::Ipv6(ip)) => ipv6_is_public(ip),
        None => false,
    }
}

fn ip_is_public(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => ipv4_is_public(ip),
        std::net::IpAddr::V6(ip) => ipv6_is_public(ip),
    }
}

fn ipv6_is_public(ip: std::net::Ipv6Addr) -> bool {
    let first = ip.segments()[0];
    let unique_local = first & 0xfe00 == 0xfc00;
    let link_local = first & 0xffc0 == 0xfe80;
    !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local) && ip.to_ipv4_mapped().is_none_or(ipv4_is_public)
}

fn ipv4_is_public(ip: std::net::Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    let shared = a == 100 && (64..128).contains(&b); // carrier-grade NAT
    !(ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast() || a == 0 || shared)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trends::testutil::Fixtures;

    /// Limits for tests against local fixture servers.
    fn limits(delay_ms: u64) -> CollectLimits {
        CollectLimits { host_delay: Duration::from_millis(delay_ms), allow_private_hosts: true, ..CollectLimits::default() }
    }

    /// Allow requests to the fixture server `fx` only.
    fn only(fx: &Fixtures) -> impl Fn(&url::Url) -> bool + Send + Sync + 'static {
        let port = url::Url::parse(fx.base()).unwrap().port();
        move |url| url.port() == port
    }

    #[test]
    fn only_http_urls_with_a_host() {
        assert!(checked_url(" https://a.test/x ").is_ok());
        for bad in ["file:///etc/passwd", "ftp://a.test/", "javascript:alert(1)", "", "a.test", "http://"] {
            assert!(checked_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn private_hosts_are_refused_unless_allowed() {
        for ok in ["https://example.com/a", "http://93.184.216.34/x", "https://sub.example.co.uk/"] {
            assert!(public_http_url(ok, false).is_ok(), "{ok}");
        }
        for bad in [
            "http://localhost/x",
            "http://127.0.0.1:8080/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://printer.local/",
            "http://intranet/",
            "http://0.0.0.0/",
        ] {
            assert!(public_http_url(bad, false).is_err(), "{bad}");
            assert!(public_http_url(bad, true).is_ok(), "{bad} when allowed");
        }
    }

    #[tokio::test]
    async fn status_and_size_are_reported_and_errors_never_carry_the_url() {
        let fx = Fixtures::start().await;
        fx.route("/ok", 200, "text/plain", "hello").route("/gone", 404, "text/plain", "x").route("/big", 200, "text/plain", "0123456789".repeat(100));
        let base = fx.base();
        let http = Http::new(&CollectLimits { max_bytes: 500, ..limits(0) }).unwrap();
        let ok = http.get(&format!("{base}/ok"), &[]).await.unwrap();
        assert_eq!((ok.text().as_str(), ok.content_type.as_str()), ("hello", "text/plain"));
        assert_eq!(http.get(&format!("{base}/gone"), &[]).await.unwrap_err(), FetchError::Status(404));
        assert_eq!(http.get(&format!("{base}/big"), &[]).await.unwrap_err(), FetchError::TooLarge(500));
        assert!(matches!(http.get("ftp://x.test/", &[]).await, Err(FetchError::BadUrl(_))));

        // Nothing listens on this port: the message must not repeat the (secret) query.
        let err = http.get("http://127.0.0.1:1/feed?token=hunter2hunter2", &[]).await.unwrap_err().to_string();
        assert!(!err.contains("hunter2") && !err.contains("127.0.0.1:1/feed"), "{err}");
    }

    #[tokio::test]
    async fn requests_to_one_host_are_spaced() {
        let fx = Fixtures::start().await;
        fx.route("/a", 200, "text/plain", "a");
        let base = fx.base();
        let http = Http::new(&limits(120)).unwrap();
        let started = std::time::Instant::now();
        let url = format!("{base}/a");
        let (a, b, c) = tokio::join!(http.get(&url, &[]), http.get(&url, &[]), http.get(&url, &[]));
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        assert!(started.elapsed() >= Duration::from_millis(240), "three requests, two gaps: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn json_errors_do_not_echo_the_body() {
        let fx = Fixtures::start().await;
        fx.route("/j", 200, "application/json", r#"{"secret": "hunter2hunter2"}"#);
        let resp = Http::new(&limits(0)).unwrap().get(&format!("{}/j", fx.base()), &[]).await.unwrap();
        #[derive(serde::Deserialize, Debug)]
        struct Wants {
            #[allow(dead_code)]
            secret: u32,
        }
        let err = resp.json::<Wants>("the API").unwrap_err().to_string();
        assert!(err.contains("the API") && !err.contains("hunter2"), "{err}");
    }
    #[tokio::test]
    async fn the_host_check_is_part_of_get_and_applies_unless_private_hosts_are_allowed() {
        let fx = Fixtures::start().await;
        fx.route("/ok", 200, "text/plain", "hello");
        let strict = Http::new(&CollectLimits { allow_private_hosts: false, ..limits(0) }).unwrap();
        // A caller cannot skip the guard: the fixture is on 127.0.0.1.
        assert_eq!(strict.get(&format!("{}/ok", fx.base()), &[]).await.unwrap_err(), FetchError::Private);
        assert_eq!(strict.get("http://localhost:1/x", &[]).await.unwrap_err(), FetchError::Private);
        assert_eq!(strict.get("http://169.254.169.254/latest/meta-data", &[]).await.unwrap_err(), FetchError::Private);
        assert!(strict.post_json("http://10.0.0.5/x", &[], &serde_json::json!({})).await.is_err());
        assert!(fx.hits().is_empty(), "nothing was requested");
        assert!(Http::new(&limits(0)).unwrap().get(&format!("{}/ok", fx.base()), &[]).await.is_ok());
    }

    #[tokio::test]
    async fn a_redirect_to_a_refused_host_is_not_followed() {
        let (a, b) = (Fixtures::start().await, Fixtures::start().await);
        a.redirect("/to-b", 302, &format!("{}/secret", b.base()));
        b.route("/secret", 200, "text/plain", "internal");
        // Only `a` may be requested: `b` stands for a private address.
        let http = Http::with_host_check(&limits(0), only(&a)).unwrap();
        assert_eq!(http.get(&format!("{}/to-b", a.base()), &[]).await.unwrap_err(), FetchError::Private);
        assert!(b.hits().is_empty(), "the refused target was never requested");
    }

    #[tokio::test]
    async fn redirects_are_followed_hop_by_hop_within_a_limit() {
        let fx = Fixtures::start().await;
        fx.redirect("/start", 301, "/middle").redirect("/middle", 307, &format!("{}/end", fx.base()));
        fx.route("/end", 200, "text/plain", "arrived");
        fx.redirect("/loop", 302, "/loop").redirect("/nowhere", 302, "ftp://files.test/x").route("/blank", 302, "text/plain", "no location");
        let http = Http::new(&limits(0)).unwrap();

        let ok = http.get(&format!("{}/start", fx.base()), &[("Accept", "text/plain")]).await.unwrap();
        assert_eq!((ok.text().as_str(), ok.url.as_str()), ("arrived", format!("{}/end", fx.base()).as_str()));
        let err = http.get(&format!("{}/loop", fx.base()), &[]).await.unwrap_err();
        assert_eq!(err, FetchError::Redirect("too many redirects".into()));
        assert_eq!(fx.hits_on("/loop").len(), MAX_REDIRECTS + 1);
        assert!(matches!(http.get(&format!("{}/nowhere", fx.base()), &[]).await, Err(FetchError::BadUrl(_))), "a redirect to ftp is not followed");
        assert!(matches!(http.get(&format!("{}/blank", fx.base()), &[]).await, Err(FetchError::Redirect(_))));
    }

    #[tokio::test]
    async fn every_hop_waits_for_its_host() {
        let fx = Fixtures::start().await;
        fx.redirect("/r", 302, "/final").route("/final", 200, "text/plain", "ok");
        let http = Http::new(&limits(150)).unwrap();
        let started = std::time::Instant::now();
        http.get(&format!("{}/r", fx.base()), &[]).await.unwrap();
        assert!(started.elapsed() >= Duration::from_millis(150), "two requests to one host, one gap: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_key_is_never_sent_to_another_origin_and_a_post_is_never_redirected() {
        let (a, b) = (Fixtures::start().await, Fixtures::start().await);
        a.redirect("/api", 302, &format!("{}/steal", b.base())).redirect("/same", 302, "/final").route("/final", 200, "text/plain", "ok");
        a.redirect("/post", 307, "/final");
        b.route("/steal", 200, "text/plain", "thanks");
        let http = Http::with_host_check(&limits(0), |_| true).unwrap();
        let key = [("X-Subscription-Token", "secret-key-0123456789")];

        let err = http.get(&format!("{}/api", a.base()), &key).await.unwrap_err();
        assert!(matches!(err, FetchError::Redirect(_)), "{err:?}");
        assert!(!err.to_string().contains("secret-key"), "{err}");
        assert!(b.hits().is_empty(), "the other origin saw nothing");

        // The same origin keeps the key (it is where the caller sent it anyway).
        http.get(&format!("{}/same", a.base()), &key).await.unwrap();
        assert!(a.hits_on("/final").iter().all(|h| h.header("x-subscription-token") == Some("secret-key-0123456789")));

        // A POST is not redirected at all, not even within the origin.
        let err = http.post_json(&format!("{}/post", a.base()), &key, &serde_json::json!({"query": "q"})).await.unwrap_err();
        assert!(matches!(err, FetchError::Redirect(_)), "{err:?}");
        assert_eq!(a.hits_on("/final").len(), 1, "only the GET above reached /final");

        // A request with `Accept` only (a feed, a page) may be redirected to another origin.
        a.redirect("/feed", 302, &format!("{}/feed", b.base()));
        b.route("/feed", 200, "text/plain", "moved feed");
        let moved = http.get(&format!("{}/feed", a.base()), &[("Accept", "application/rss+xml")]).await.unwrap();
        assert_eq!(moved.text(), "moved feed");
    }

    #[tokio::test]
    async fn names_that_resolve_only_to_private_addresses_are_refused() {
        // `localhost` comes from /etc/hosts: no network involved.
        let err = public_addrs("localhost").await.unwrap_err().to_string();
        assert!(err.contains("private network"), "{err}");
        assert!(ip_is_public("93.184.216.34".parse().unwrap()) && !ip_is_public("::1".parse().unwrap()) && !ip_is_public("10.1.2.3".parse().unwrap()));
    }
}
