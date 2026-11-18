//! Sign in with ChatGPT the way the Codex CLI does: the system browser opens the issuer's
//! authorize page, the issuer redirects to `http://localhost:1455/auth/callback` (loopback + PKCE),
//! and the code is swapped for tokens. Also: refreshing the access token, and the Codex CLI's own
//! login (`auth.json`): read on every call of a linked provider, and its tokens written back after
//! JobPilot refreshed them (see the module doc of [`super`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::tokens::Tokens;
use super::SIGN_IN_INVALID;
use crate::llm::http::{describe_net_error, http_error};
use crate::mail::oauth::{self, RedirectSite};

/// OpenAI's sign-in issuer.
pub(crate) const ISSUER: &str = "https://auth.openai.com";
/// The Codex CLI's public OAuth client (no secret: PKCE).
pub(crate) const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
/// The only port the issuer redirects to for this client.
pub(crate) const PORT: u16 = 1455;
pub(crate) const CALLBACK_PATH: &str = "/auth/callback";
pub(crate) const REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const SCOPE: &str = "openid profile email offline_access";
const REFRESH_SCOPE: &str = "openid profile email";
const ORIGINATOR: &str = "codex_cli_rs";
const TOKEN_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) const PORT_IN_USE: &str = "Port 1455 is in use (a Codex CLI sign-in may be open): close it and try again";

/// Wakes a sign-in that waits for the browser ([`crate::Core::codex_cancel_sign_in`]).
pub(crate) static CANCEL: Notify = Notify::const_new();

const SITE: RedirectSite<'static> = RedirectSite { path: CALLBACK_PATH, done: "Signed in with ChatGPT", issuer: "ChatGPT" };

/// `{issuer}/oauth/authorize?...`, the page the browser opens.
pub(crate) fn authorize_url(issuer: &str, redirect_uri: &str, challenge: &str, state: &str) -> Result<String> {
    let mut url = url::Url::parse(&format!("{}/oauth/authorize", issuer.trim_end_matches('/')))
        .context("the sign-in address is not valid")?;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", CLIENT_ID)
        .append_pair("redirect_uri", redirect_uri)
        .append_pair("scope", SCOPE)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("id_token_add_organizations", "true")
        .append_pair("codex_cli_simplified_flow", "true")
        .append_pair("state", state)
        .append_pair("originator", ORIGINATOR);
    Ok(url.into())
}

fn token_url(issuer: &str) -> String {
    format!("{}/oauth/token", issuer.trim_end_matches('/'))
}

/// Bind the fixed callback port on loopback.
pub(crate) async fn bind_callback() -> Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", PORT)).await.map_err(|_| anyhow!(PORT_IN_USE))
}

/// The whole browser sign-in on `listener`: `open(url)` shows the authorize page, the redirect to
/// `redirect_uri` brings the code, which is swapped at `issuer` for tokens. Cancelled by [`CANCEL`].
pub(crate) async fn sign_in(
    http: &reqwest::Client,
    listener: &TcpListener,
    issuer: &str,
    redirect_uri: &str,
    open: impl FnOnce(&str) -> Result<()>,
) -> Result<Tokens> {
    let pkce = oauth::Pkce::generate();
    let state = oauth::random_string(32);
    let url = authorize_url(issuer, redirect_uri, &pkce.challenge, &state)?;
    open(&url).map_err(|e| anyhow!("{e:#}. Open this address in your browser to continue: {url}"))?;
    let code = tokio::select! {
        r = oauth::wait_for_redirect_at(listener, SITE, &state, oauth::SIGN_IN_TIMEOUT) => r?,
        _ = CANCEL.notified() => bail!("sign-in cancelled"),
    };
    exchange_code(http, issuer, &code, redirect_uri, &pkce.verifier).await
}

/// Swap the authorization code (+ PKCE verifier) for tokens.
pub(crate) async fn exchange_code(
    http: &reqwest::Client,
    issuer: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<Tokens> {
    let req = http.post(token_url(issuer)).timeout(TOKEN_TIMEOUT).form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", CLIENT_ID),
        ("code_verifier", verifier),
    ]);
    let v = post_token(req).await?;
    let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let (access, refresh, id) = (field("access_token"), field("refresh_token"), field("id_token"));
    if access.is_empty() || refresh.is_empty() {
        bail!("ChatGPT sign-in: the token answer has no access or refresh token");
    }
    Ok(Tokens::from_raw(&access, &refresh, &id, None))
}

/// A refresh failed because the sign-in itself is no longer valid (revoked, expired, reused).
#[derive(Debug)]
pub(crate) struct SignInExpired(pub String);

impl std::fmt::Display for SignInExpired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SignInExpired {}

/// New tokens from the refresh token: a new access token, and a new id / refresh token when the
/// issuer sends them (the old ones are kept otherwise).
pub(crate) async fn refresh(http: &reqwest::Client, issuer: &str, old: &Tokens) -> Result<Tokens> {
    let req = http.post(token_url(issuer)).timeout(TOKEN_TIMEOUT).json(&json!({
        "client_id": CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": old.refresh_token,
        "scope": REFRESH_SCOPE,
    }));
    let v = post_token(req).await?;
    let field = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let access = field("access_token").context("ChatGPT sign-in: the refresh answer has no access token")?;
    let refresh = field("refresh_token").unwrap_or_else(|| old.refresh_token.clone());
    let id = field("id_token").unwrap_or_else(|| old.id_token.clone());
    let mut fresh = Tokens::from_raw(&access, &refresh, &id, Some(&old.account_id));
    if fresh.email.is_empty() {
        fresh.email = old.email.clone();
    }
    Ok(fresh)
}

/// POST to the token endpoint; a 2xx JSON body, else a readable error (a 400/401 is
/// [`SignInExpired`]). Error text never carries a token: only the issuer's message, redacted.
async fn post_token(req: reqwest::RequestBuilder) -> Result<Value> {
    let resp = req
        .send()
        .await
        .map_err(|e| anyhow!("ChatGPT sign-in: cannot reach the token endpoint: {}", describe_net_error(&e)))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| anyhow!("ChatGPT sign-in: reading the token answer failed: {}", describe_net_error(&e)))?;
    if !status.is_success() {
        let detail = crate::redact::redact(&http_error(status, &body)).into_owned();
        if matches!(status, StatusCode::BAD_REQUEST | StatusCode::UNAUTHORIZED) {
            return Err(SignInExpired(format!("{SIGN_IN_INVALID} ({detail})")).into());
        }
        bail!("ChatGPT sign-in: {detail}");
    }
    serde_json::from_str(&body).map_err(|_| anyhow!("ChatGPT sign-in: the token endpoint did not return JSON"))
}

// ---------------------------------------------------------------- the Codex CLI's login

/// `$CODEX_HOME/auth.json`, else `~/.codex/auth.json`.
pub(crate) fn cli_auth_path() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").filter(|h| !h.is_empty()) {
        Some(home) => Some(PathBuf::from(home).join("auth.json")),
        None => dirs::home_dir().map(|h| h.join(".codex").join("auth.json")),
    }
}

/// Read the Codex CLI's ChatGPT login from `path`. An `OPENAI_API_KEY` in the file is ignored.
pub(crate) fn read_cli_login(path: &Path) -> Result<Tokens> {
    let raw = std::fs::read_to_string(path)
        .map_err(|_| anyhow!("No Codex CLI login was found ({}): run `codex login` first", path.display()))?;
    let v: Value = serde_json::from_str(&raw).map_err(|_| anyhow!("The Codex CLI's login file is not valid JSON"))?;
    let tokens = v.get("tokens").filter(|t| t.is_object()).context("The Codex CLI is not signed in with ChatGPT")?;
    let field = |k: &str| tokens.get(k).and_then(Value::as_str).unwrap_or("");
    if field("access_token").trim().is_empty() {
        bail!("The Codex CLI is not signed in with ChatGPT");
    }
    Ok(Tokens::from_raw(field("access_token"), field("refresh_token"), field("id_token"), Some(field("account_id"))))
}

/// The CLI's login file exists and holds a ChatGPT access token.
pub(crate) fn cli_login_at(path: &Path) -> bool {
    let Ok(raw) = std::fs::read_to_string(path) else { return false };
    serde_json::from_str::<Value>(&raw)
        .ok()
        .and_then(|v| v.pointer("/tokens/access_token").and_then(Value::as_str).map(|t| !t.trim().is_empty()))
        .unwrap_or(false)
}

/// Write refreshed `tokens` back into the Codex CLI's login file at `path`, as the CLI itself does:
/// read-modify-write of `tokens.*` and `last_refresh` only (every other field is kept), replaced
/// atomically through a temp file in the same folder, keeping the file's permissions (0600 on unix).
pub(crate) fn write_cli_login(path: &Path, tokens: &Tokens) -> Result<()> {
    let gone = || anyhow!(super::CLI_LOGIN_GONE);
    let raw = std::fs::read_to_string(path).map_err(|_| gone())?;
    let mut v: Value = serde_json::from_str(&raw).map_err(|_| anyhow!("The Codex CLI's login file is not valid JSON"))?;
    let root = v.as_object_mut().ok_or_else(gone)?;
    let entry = root.entry("tokens").or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
    }
    if let Some(t) = entry.as_object_mut() {
        t.insert("access_token".into(), json!(tokens.access_token));
        t.insert("refresh_token".into(), json!(tokens.refresh_token));
        if !tokens.id_token.is_empty() {
            t.insert("id_token".into(), json!(tokens.id_token));
        }
        if !tokens.account_id.is_empty() {
            t.insert("account_id".into(), json!(tokens.account_id));
        }
    }
    root.insert("last_refresh".into(), json!(chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true)));
    let text = serde_json::to_string_pretty(&v).context("encoding the Codex CLI's login")?;
    write_atomically(path, text.as_bytes()).context("updating the Codex CLI's login file")
}

/// Replace `path` with `bytes`: a temp file in the same folder (owner-only from the start on unix),
/// given the old file's permissions, then renamed over it.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "auth.json".into());
    let tmp = dir.join(format!(".{name}.jobpilot-{}.tmp", std::process::id()));
    let permissions = std::fs::metadata(path).map(|m| m.permissions()).ok();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let result = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(permissions) = permissions {
            std::fs::set_permissions(&tmp, permissions)?;
        }
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}
