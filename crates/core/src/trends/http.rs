//! The one HTTP client of the collectors: read-only, http(s) only, a size cap, and a minimum gap
//! between two requests to the same host (shared by every collector of a run).

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
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

#[derive(Clone)]
pub struct Http {
    client: reqwest::Client,
    delay: Duration,
    max_bytes: usize,
    /// Per host: the earliest time the next request may start.
    slots: Arc<Mutex<HashMap<String, Instant>>>,
}

impl Http {
    pub fn new(limits: &CollectLimits) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(limits.request_timeout)
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .context("building the HTTP client")?;
        Ok(Self { client, delay: limits.host_delay, max_bytes: limits.max_bytes, slots: Arc::default() })
    }

    /// `GET url`; a non-2xx status is [`FetchError::Status`].
    pub async fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response, FetchError> {
        let url = checked_url(url)?;
        let mut request = self.client.get(url.clone());
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(&url, request).await
    }

    /// `POST url` with a JSON body.
    pub async fn post_json(&self, url: &str, headers: &[(&str, &str)], body: &serde_json::Value) -> Result<Response, FetchError> {
        let url = checked_url(url)?;
        let mut request = self.client.post(url.clone()).json(body);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        self.send(&url, request).await
    }

    async fn send(&self, url: &url::Url, request: reqwest::RequestBuilder) -> Result<Response, FetchError> {
        self.wait_turn(url.host_str().unwrap_or_default()).await;
        let mut resp = request.send().await.map_err(transport)?;
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
        Some(url::Host::Ipv6(ip)) => {
            let first = ip.segments()[0];
            let unique_local = first & 0xfe00 == 0xfc00;
            let link_local = first & 0xffc0 == 0xfe80;
            !(ip.is_loopback() || ip.is_unspecified() || unique_local || link_local)
                && ip.to_ipv4_mapped().is_none_or(ipv4_is_public)
        }
        None => false,
    }
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

    fn limits(delay_ms: u64) -> CollectLimits {
        CollectLimits { host_delay: Duration::from_millis(delay_ms), ..CollectLimits::default() }
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
}
