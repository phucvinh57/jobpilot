//! Google OAuth for installed apps: loopback redirect + PKCE (RFC 7636), done by hand.
//!
//! Flow: bind `127.0.0.1:0`, open [`auth_url`] in the system browser, wait for Google to redirect
//! to `http://127.0.0.1:<port>/?code=..&state=..` ([`wait_for_redirect`]), then exchange the code
//! ([`exchange_code`]). Tokens never go to logs or the database.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::distr::Alphanumeric;
use rand::RngExt;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

pub const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub const SCOPE: &str = "https://www.googleapis.com/auth/gmail.readonly";
/// How long the browser sign-in may take.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

/// Refresh the access token this long before it really expires.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);
/// A browser connection that sends nothing is dropped after this long.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- PKCE

/// `BASE64URL(SHA256(verifier))`, no padding (method `S256`).
pub fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[derive(Clone, Debug)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    pub fn generate() -> Self {
        let verifier = random_string(64);
        let challenge = pkce_challenge(&verifier);
        Self { verifier, challenge }
    }
}

/// Random URL-safe string (PKCE verifier, `state`).
pub fn random_string(len: usize) -> String {
    rand::rng().sample_iter(Alphanumeric).take(len).map(char::from).collect()
}

/// The consent-screen URL to open in the browser.
pub fn auth_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let mut url = url::Url::parse(AUTH_URL).expect("AUTH_URL is valid");
    url.query_pairs_mut()
        .append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        .append_pair("prompt", "consent");
    url.into()
}

// ---------------------------------------------------------------- loopback redirect

/// What one browser request to the loopback listener means.
#[derive(Debug, PartialEq, Eq)]
pub enum Redirect {
    /// Authorization code, `state` verified.
    Code(String),
    /// The user (or Google) refused: the `error` parameter.
    Denied(String),
    /// Not the redirect (favicon, probe, no query): keep waiting.
    Ignore,
}

/// Parse the request line of the redirect, e.g. `GET /?code=abc&state=xyz HTTP/1.1`.
/// `Err` when a `code`/`error` arrives with the wrong `state`.
pub fn parse_redirect(request: &str, expected_state: &str) -> Result<Redirect> {
    parse_redirect_at(request, "/", expected_state)
}

/// [`parse_redirect`] for a redirect that arrives at `path` (e.g. `/auth/callback`).
pub(crate) fn parse_redirect_at(request: &str, path: &str, expected_state: &str) -> Result<Redirect> {
    let line = request.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else { return Ok(Redirect::Ignore) };
    let Ok(url) = url::Url::parse("http://127.0.0.1").and_then(|base| base.join(target)) else {
        return Ok(Redirect::Ignore);
    };
    if url.path() != path {
        return Ok(Redirect::Ignore);
    }
    let params: HashMap<String, String> = url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
    if !params.contains_key("code") && !params.contains_key("error") {
        return Ok(Redirect::Ignore);
    }
    if params.get("state").map(String::as_str) != Some(expected_state) {
        bail!("the redirect carried an unexpected `state`");
    }
    if let Some(err) = params.get("error") {
        return Ok(Redirect::Denied(err.clone()));
    }
    match params.get("code").filter(|c| !c.is_empty()) {
        Some(code) => Ok(Redirect::Code(code.clone())),
        None => Ok(Redirect::Ignore),
    }
}

/// The small page the browser shows after the redirect, and who is named in a denial error.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RedirectSite<'a> {
    /// Path the redirect arrives at (`/` for Gmail).
    pub path: &'a str,
    /// Heading of the success page (`Gmail connected`).
    pub done: &'a str,
    /// Who refused, in the denial error (`Google`).
    pub issuer: &'a str,
}

const GMAIL_SITE: RedirectSite<'static> = RedirectSite { path: "/", done: "Gmail connected", issuer: "Google" };

fn page(heading: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>JobPilot</title>\
<body style=\"font-family:sans-serif;margin:4em auto;max-width:28em\">\
<h2>{heading}</h2><p>You can close this tab and return to JobPilot.</p></body>"
    )
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// Read up to the end of the request line (or 8 KiB / the read timeout).
async fn read_request_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    let read = async {
        while buf.len() < 8192 && !buf.windows(2).any(|w| w == b"\r\n") {
            match stream.read(&mut chunk).await {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
        }
    };
    tokio::time::timeout(READ_TIMEOUT, read).await.ok()?;
    (!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned())
}

/// Accept connections until the redirect arrives (other requests get a 404), reply with a small
/// "you can close this tab" page and return the authorization code.
pub async fn wait_for_redirect(listener: &TcpListener, state: &str, timeout: Duration) -> Result<String> {
    wait_for_redirect_at(listener, GMAIL_SITE, state, timeout).await
}

/// [`wait_for_redirect`] for a redirect to `site.path`, with `site`'s wording.
pub(crate) async fn wait_for_redirect_at(
    listener: &TcpListener,
    site: RedirectSite<'_>,
    state: &str,
    timeout: Duration,
) -> Result<String> {
    let wait = async {
        loop {
            let (mut stream, _) = listener.accept().await.context("the local sign-in listener failed")?;
            let Some(head) = read_request_head(&mut stream).await else { continue };
            match parse_redirect_at(&head, site.path, state) {
                Ok(Redirect::Code(code)) => {
                    respond(&mut stream, "200 OK", &page(site.done)).await;
                    return Ok(code);
                }
                Ok(Redirect::Denied(reason)) => {
                    respond(&mut stream, "200 OK", &page("Sign-in was not completed")).await;
                    bail!("{} sign-in was not completed ({reason})", site.issuer);
                }
                Ok(Redirect::Ignore) => respond(&mut stream, "404 Not Found", "").await,
                Err(_) => respond(&mut stream, "400 Bad Request", "").await,
            }
        }
    };
    tokio::time::timeout(timeout, wait)
        .await
        .map_err(|_| anyhow!("timed out waiting for the browser sign-in ({} minutes)", timeout.as_secs() / 60))?
}

/// Open `url` in the system browser (`xdg-open` / `open` / `rundll32`).
pub fn open_browser(url: &str) -> Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else if cfg!(target_os = "windows") {
        let mut c = Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("could not start the system browser")?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

// ---------------------------------------------------------------- token endpoint

/// Google's `{"error": .., "error_description": ..}` answer.
#[derive(Debug, Clone)]
pub struct OAuthError {
    pub code: String,
    pub description: String,
}

impl OAuthError {
    /// The refresh token was revoked or expired: the user must connect again.
    pub fn is_invalid_grant(&self) -> bool {
        self.code == "invalid_grant"
    }
}

impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.description.is_empty() {
            write!(f, "Google rejected the request: {}", self.code)
        } else {
            write!(f, "Google rejected the request: {} ({})", self.code, self.description)
        }
    }
}

impl std::error::Error for OAuthError {}

#[derive(Clone, PartialEq, Eq)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    /// Present on the first exchange (with `access_type=offline`), not on refreshes.
    pub refresh_token: Option<String>,
}

impl fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenResponse").field("expires_in", &self.expires_in).finish_non_exhaustive()
    }
}

/// Parse a token-endpoint body (success or error).
pub fn parse_token_response(body: &str) -> Result<TokenResponse> {
    let v: Value = serde_json::from_str(body).map_err(|_| anyhow!("Google's token endpoint did not return JSON"))?;
    if let Some(code) = v.get("error").and_then(Value::as_str) {
        let description = v.get("error_description").and_then(Value::as_str).unwrap_or("").to_string();
        return Err(OAuthError { code: code.to_string(), description }.into());
    }
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| anyhow!("Google's token response has no access_token"))?
        .to_string();
    Ok(TokenResponse {
        access_token,
        expires_in: v.get("expires_in").and_then(Value::as_u64).unwrap_or(3600),
        refresh_token: v.get("refresh_token").and_then(Value::as_str).filter(|t| !t.is_empty()).map(str::to_string),
    })
}

pub(crate) fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(60))
        .user_agent(concat!("jobpilot/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

async fn post_token(http: &reqwest::Client, url: &str, form: &[(&str, &str)]) -> Result<TokenResponse> {
    let resp = http
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|e| anyhow!("cannot reach Google's token endpoint: {}", e.without_url()))?;
    let body = resp.text().await.map_err(|e| anyhow!("reading Google's token response failed: {}", e.without_url()))?;
    parse_token_response(&body)
}

/// Swap the authorization code (+ PKCE verifier) for tokens.
pub async fn exchange_code(
    http: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<TokenResponse> {
    post_token(
        http,
        token_url,
        &[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("redirect_uri", redirect_uri),
            ("code_verifier", verifier),
        ],
    )
    .await
}

/// Get a fresh access token from the refresh token.
pub async fn refresh_access_token(
    http: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<TokenResponse> {
    post_token(
        http,
        token_url,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", client_id),
            ("client_secret", client_secret),
        ],
    )
    .await
}

/// Best-effort revoke; failures are ignored by the caller.
pub async fn revoke(http: &reqwest::Client, url: &str, token: &str) -> Result<()> {
    let resp = http
        .post(url)
        .timeout(Duration::from_secs(10))
        .form(&[("token", token)])
        .send()
        .await
        .map_err(|e| anyhow!("{}", e.without_url()))?;
    anyhow::ensure!(resp.status().is_success(), "revoke answered HTTP {}", resp.status().as_u16());
    Ok(())
}

// ---------------------------------------------------------------- in-memory access-token cache

struct Cached {
    access_token: String,
    expires: Instant,
}

/// Keyed by a hash of the refresh token, so different accounts never share a token.
fn cache() -> &'static Mutex<HashMap<String, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Cached>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn cache_key(refresh_token: &str) -> String {
    Sha256::digest(refresh_token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn cache_get(refresh_token: &str) -> Option<String> {
    let map = cache().lock().unwrap_or_else(|e| e.into_inner());
    let hit = map.get(&cache_key(refresh_token))?;
    (hit.expires > Instant::now()).then(|| hit.access_token.clone())
}

pub(crate) fn cache_put(refresh_token: &str, access_token: &str, expires_in: u64) {
    let expires = Instant::now() + Duration::from_secs(expires_in).saturating_sub(EXPIRY_MARGIN);
    cache()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(cache_key(refresh_token), Cached { access_token: access_token.to_string(), expires });
}

pub(crate) fn cache_clear(refresh_token: &str) {
    cache().lock().unwrap_or_else(|e| e.into_inner()).remove(&cache_key(refresh_token));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_pkce_is_valid() {
        let p = Pkce::generate();
        assert!((43..=128).contains(&p.verifier.len()));
        assert_eq!(p.challenge, pkce_challenge(&p.verifier));
        assert_ne!(Pkce::generate().verifier, p.verifier);
    }

    #[test]
    fn builds_the_auth_url() {
        let url = auth_url("cid.apps.googleusercontent.com", "http://127.0.0.1:5555", "chal", "st&te");
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
        let parsed = url::Url::parse(&url).unwrap();
        let q: HashMap<_, _> = parsed.query_pairs().into_owned().collect();
        assert_eq!(q["client_id"], "cid.apps.googleusercontent.com");
        assert_eq!(q["redirect_uri"], "http://127.0.0.1:5555");
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["scope"], "https://www.googleapis.com/auth/gmail.readonly");
        assert_eq!(q["code_challenge"], "chal");
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], "st&te");
        assert_eq!(q["access_type"], "offline");
        assert_eq!(q["prompt"], "consent");
        // The raw URL must have the redirect and state percent-encoded.
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5555"));
        assert!(url.contains("state=st%26te"));
    }

    #[test]
    fn parses_redirect_requests() {
        let ok = "GET /?code=4%2F0Abc&state=xyz&scope=x HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
        assert_eq!(parse_redirect(ok, "xyz").unwrap(), Redirect::Code("4/0Abc".into()));

        let denied = "GET /?error=access_denied&state=xyz HTTP/1.1\r\n\r\n";
        assert_eq!(parse_redirect(denied, "xyz").unwrap(), Redirect::Denied("access_denied".into()));

        assert!(parse_redirect("GET /?code=abc&state=other HTTP/1.1\r\n\r\n", "xyz").is_err());
        assert!(parse_redirect("GET /?code=abc HTTP/1.1\r\n\r\n", "xyz").is_err());

        assert_eq!(parse_redirect("GET /favicon.ico HTTP/1.1\r\n\r\n", "xyz").unwrap(), Redirect::Ignore);
        assert_eq!(parse_redirect("GET / HTTP/1.1\r\n\r\n", "xyz").unwrap(), Redirect::Ignore);
        assert_eq!(parse_redirect("POST /?code=a&state=xyz HTTP/1.1\r\n\r\n", "xyz").unwrap(), Redirect::Ignore);
        assert_eq!(parse_redirect("", "xyz").unwrap(), Redirect::Ignore);

        // Another callback path: only that path counts.
        let cb = "GET /auth/callback?code=c1&state=xyz HTTP/1.1\r\n\r\n";
        assert_eq!(parse_redirect_at(cb, "/auth/callback", "xyz").unwrap(), Redirect::Code("c1".into()));
        assert_eq!(parse_redirect(cb, "xyz").unwrap(), Redirect::Ignore);
        assert_eq!(parse_redirect_at(ok, "/auth/callback", "xyz").unwrap(), Redirect::Ignore);
    }

    async fn fetch(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[tokio::test]
    async fn loopback_listener_returns_the_code() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_redirect(&listener, "xyz", Duration::from_secs(10)).await });

        // Noise first: a favicon request and a redirect with a forged state.
        assert!(fetch(port, "GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n").await.starts_with("HTTP/1.1 404"));
        assert!(fetch(port, "GET /?code=evil&state=nope HTTP/1.1\r\n\r\n").await.starts_with("HTTP/1.1 400"));
        let page = fetch(port, "GET /?code=good&state=xyz HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(page.starts_with("HTTP/1.1 200"));
        assert!(page.contains("close this tab"));

        assert_eq!(waiter.await.unwrap().unwrap(), "good");
    }

    #[tokio::test]
    async fn loopback_listener_reports_denial_and_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let waiter = tokio::spawn(async move { wait_for_redirect(&listener, "xyz", Duration::from_secs(10)).await });
        fetch(port, "GET /?error=access_denied&state=xyz HTTP/1.1\r\n\r\n").await;
        let err = waiter.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("access_denied"), "{err}");

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let err = wait_for_redirect(&listener, "xyz", Duration::from_millis(50)).await.unwrap_err();
        assert!(err.to_string().contains("timed out"), "{err}");
    }

    #[test]
    fn parses_token_responses() {
        let ok = r#"{"access_token":"ya29.a","expires_in":3599,"refresh_token":"1//r","scope":"x","token_type":"Bearer"}"#;
        let t = parse_token_response(ok).unwrap();
        assert_eq!(t.access_token, "ya29.a");
        assert_eq!(t.expires_in, 3599);
        assert_eq!(t.refresh_token.as_deref(), Some("1//r"));
        // The Debug output must not leak tokens.
        assert!(!format!("{t:?}").contains("ya29"));

        let refreshed = parse_token_response(r#"{"access_token":"ya29.b","expires_in":10}"#).unwrap();
        assert_eq!(refreshed.refresh_token, None);

        let err = parse_token_response(r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#)
            .unwrap_err();
        let oauth = err.downcast_ref::<OAuthError>().unwrap();
        assert!(oauth.is_invalid_grant());
        assert!(err.to_string().contains("Token has been expired"));

        assert!(parse_token_response("<html>").is_err());
        assert!(parse_token_response(r#"{"expires_in":1}"#).is_err());
    }

    #[test]
    fn caches_access_tokens_until_expiry() {
        cache_put("refresh-a", "access-a", 3600);
        assert_eq!(cache_get("refresh-a").as_deref(), Some("access-a"));
        assert_eq!(cache_get("refresh-other"), None);
        // Within the safety margin of expiry counts as expired.
        cache_put("refresh-b", "access-b", 30);
        assert_eq!(cache_get("refresh-b"), None);
        cache_clear("refresh-a");
        assert_eq!(cache_get("refresh-a"), None);
    }
}
