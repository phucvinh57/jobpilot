//! Gmail account state on `Core`: client credentials, connect/disconnect, access tokens.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Utc};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use super::gmail::GmailClient;
use super::oauth;
use super::sync::ReportSummary;
use crate::engine::Core;

/// kv: the Google Cloud OAuth client id (not secret).
pub const KV_CLIENT_ID: &str = "gmail_client_id";
/// kv: the connected account's address; its presence means "connected".
pub const KV_ACCOUNT: &str = "gmail_account";
/// kv: `historyId` to continue the incremental sync from.
pub const KV_HISTORY_ID: &str = "gmail_history_id";
/// kv: RFC 3339 time of the last successful sync.
pub const KV_LAST_SYNC: &str = "gmail_last_sync";
/// kv: JSON [`ReportSummary`] of the last sync attempt.
pub const KV_LAST_REPORT: &str = "gmail_last_report";
/// kv: minutes between polls (default 10).
pub const KV_POLL_MINUTES: &str = "gmail_poll_minutes";
/// keyring: the OAuth refresh token.
pub const SECRET_REFRESH_TOKEN: &str = "gmail:refresh_token";
/// keyring: the OAuth client secret.
pub const SECRET_CLIENT_SECRET: &str = "gmail:client_secret";

const DEFAULT_POLL_MINUTES: u64 = 10;

static CANCEL: Notify = Notify::const_new();

/// What the settings screen shows (kv reads only, safe on the UI thread).
#[derive(Clone, Debug, Default)]
pub struct GmailState {
    pub client_id: String,
    /// Connected account address.
    pub account: Option<String>,
    pub last_sync: Option<DateTime<Utc>>,
    pub last_report: Option<ReportSummary>,
}

impl Core {
    /// Fast kv check (no keyring access).
    pub fn gmail_connected(&self) -> bool {
        self.store.kv_get(KV_ACCOUNT).ok().flatten().is_some_and(|a| !a.is_empty())
    }

    pub fn gmail_state(&self) -> GmailState {
        let kv = |k: &str| self.store.kv_get(k).ok().flatten().filter(|v| !v.is_empty());
        GmailState {
            client_id: kv(KV_CLIENT_ID).unwrap_or_default(),
            account: kv(KV_ACCOUNT),
            last_sync: kv(KV_LAST_SYNC)
                .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc)),
            last_report: kv(KV_LAST_REPORT).and_then(|s| serde_json::from_str(&s).ok()),
        }
    }

    /// Time between polls: kv `gmail_poll_minutes`, default 10, at least 1.
    pub fn gmail_poll_interval(&self) -> Duration {
        let minutes = self
            .store
            .kv_get(KV_POLL_MINUTES)
            .ok()
            .flatten()
            .and_then(|m| m.trim().parse::<u64>().ok())
            .unwrap_or(DEFAULT_POLL_MINUTES)
            .max(1);
        Duration::from_secs(minutes * 60)
    }

    /// Store the OAuth client id (kv) and, when given and non-empty, the secret (keyring).
    pub fn gmail_save_client(&self, client_id: &str, client_secret: Option<&str>) -> Result<()> {
        self.store.kv_set(KV_CLIENT_ID, client_id.trim())?;
        if let Some(secret) = client_secret.map(str::trim).filter(|s| !s.is_empty()) {
            self.secrets.set(SECRET_CLIENT_SECRET, secret)?;
        }
        Ok(())
    }

    /// Abort a running [`Core::gmail_connect`] (it returns an error).
    pub fn gmail_cancel_connect(&self) {
        CANCEL.notify_waiters();
    }

    fn gmail_client_credentials(&self) -> Result<(String, String)> {
        let id = self.store.kv_get(KV_CLIENT_ID)?.unwrap_or_default();
        if id.trim().is_empty() {
            bail!("enter the Google OAuth client id first");
        }
        let secret = self.secrets.get(SECRET_CLIENT_SECRET)?.unwrap_or_default();
        if secret.trim().is_empty() {
            bail!("enter the Google OAuth client secret first");
        }
        Ok((id.trim().to_string(), secret.trim().to_string()))
    }

    /// Sign in through the system browser (loopback + PKCE), keep the refresh token in the
    /// keyring and return the account address. Takes at most 5 minutes.
    pub async fn gmail_connect(&self) -> Result<String> {
        let (client_id, client_secret) = self.gmail_client_credentials()?;
        let listener = TcpListener::bind("127.0.0.1:0").await.context("could not open a local port for the sign-in")?;
        let redirect_uri = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
        let pkce = oauth::Pkce::generate();
        let state = oauth::random_string(32);
        let url = oauth::auth_url(&client_id, &redirect_uri, &pkce.challenge, &state);
        oauth::open_browser(&url).map_err(|e| anyhow!("{e:#}. Open this address in your browser to continue: {url}"))?;

        let code = tokio::select! {
            r = oauth::wait_for_redirect(&listener, &state, oauth::SIGN_IN_TIMEOUT) => r?,
            _ = CANCEL.notified() => bail!("sign-in cancelled"),
        };
        drop(listener);

        let http = oauth::http_client();
        let tokens = oauth::exchange_code(
            &http,
            oauth::TOKEN_URL,
            &client_id,
            &client_secret,
            &code,
            &redirect_uri,
            &pkce.verifier,
        )
        .await?;
        let refresh = tokens.refresh_token.clone().ok_or_else(|| {
            anyhow!(
                "Google returned no refresh token. Remove JobPilot at myaccount.google.com/permissions and connect again"
            )
        })?;
        let profile = GmailClient::new(&tokens.access_token).profile().await?;

        self.secrets.set(SECRET_REFRESH_TOKEN, &refresh)?;
        oauth::cache_put(&refresh, &tokens.access_token, tokens.expires_in);
        // A new sign-in starts a fresh sync (seen messages are skipped by id).
        self.store.kv_delete(KV_HISTORY_ID)?;
        self.store.kv_delete(KV_LAST_SYNC)?;
        self.store.kv_delete(KV_LAST_REPORT)?;
        self.store.kv_set(KV_ACCOUNT, &profile.email_address)?;
        tracing::info!("gmail: connected");
        Ok(profile.email_address)
    }

    /// Revoke the token at Google (best effort) and forget the account. The OAuth client id and
    /// secret stay, so connecting again is one click.
    pub async fn gmail_disconnect(&self) -> Result<()> {
        if let Some(refresh) = self.secrets.get(SECRET_REFRESH_TOKEN).ok().flatten() {
            if let Err(e) = oauth::revoke(&oauth::http_client(), oauth::REVOKE_URL, &refresh).await {
                tracing::debug!("revoking the Gmail token failed (ignored): {e}");
            }
        }
        self.gmail_forget_account()
    }

    /// Local half of disconnect: token, cache and kv entries.
    fn gmail_forget_account(&self) -> Result<()> {
        self.gmail_forget_access_token();
        self.secrets.delete(SECRET_REFRESH_TOKEN)?;
        for key in [KV_ACCOUNT, KV_HISTORY_ID, KV_LAST_SYNC, KV_LAST_REPORT] {
            self.store.kv_delete(key)?;
        }
        Ok(())
    }

    /// A valid access token: cached, or refreshed from the keyring's refresh token.
    pub(crate) async fn gmail_access_token(&self) -> Result<String> {
        let refresh = self
            .secrets
            .get(SECRET_REFRESH_TOKEN)?
            .filter(|t| !t.is_empty())
            .ok_or_else(|| anyhow!("Gmail is not connected"))?;
        if let Some(token) = oauth::cache_get(&refresh) {
            return Ok(token);
        }
        let (client_id, client_secret) = self.gmail_client_credentials()?;
        match oauth::refresh_access_token(&oauth::http_client(), oauth::TOKEN_URL, &client_id, &client_secret, &refresh)
            .await
        {
            Ok(t) => {
                oauth::cache_put(&refresh, &t.access_token, t.expires_in);
                Ok(t.access_token)
            }
            Err(e) if e.downcast_ref::<oauth::OAuthError>().is_some_and(|o| o.is_invalid_grant()) => {
                bail!("Google no longer accepts the saved sign-in (revoked or expired). Disconnect and connect Gmail again")
            }
            Err(e) => Err(e),
        }
    }

    /// Drop the cached access token (after an HTTP 401).
    pub(crate) fn gmail_forget_access_token(&self) {
        if let Some(refresh) = self.secrets.get(SECRET_REFRESH_TOKEN).ok().flatten() {
            oauth::cache_clear(&refresh);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::{Core, Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    #[test]
    fn client_credentials_and_state() {
        let (_d, core) = core();
        assert!(!core.gmail_connected());
        assert_eq!(core.gmail_state().client_id, "");

        core.gmail_save_client(" abc.apps.googleusercontent.com ", Some(" s3cret ")).unwrap();
        assert_eq!(core.gmail_state().client_id, "abc.apps.googleusercontent.com");
        assert_eq!(core.secrets.get(super::SECRET_CLIENT_SECRET).unwrap().as_deref(), Some("s3cret"));
        // Empty secret keeps the stored one.
        core.gmail_save_client("abc.apps.googleusercontent.com", Some("")).unwrap();
        assert_eq!(core.secrets.get(super::SECRET_CLIENT_SECRET).unwrap().as_deref(), Some("s3cret"));
        // The secret never lands in the database.
        assert_eq!(core.store.kv_get(super::SECRET_CLIENT_SECRET).unwrap(), None);
        assert_eq!(core.gmail_client_credentials().unwrap().1, "s3cret");
    }

    #[test]
    fn connect_requires_credentials() {
        let (_d, core) = core();
        let err = crate::engine::runtime::block_on(core.gmail_connect()).unwrap_err();
        assert!(err.to_string().contains("client id"), "{err}");
        core.gmail_save_client("id", None).unwrap();
        let err = crate::engine::runtime::block_on(core.gmail_connect()).unwrap_err();
        assert!(err.to_string().contains("client secret"), "{err}");
    }

    #[test]
    fn poll_interval_defaults_and_clamps() {
        let (_d, core) = core();
        assert_eq!(core.gmail_poll_interval().as_secs(), 600);
        core.store.kv_set(super::KV_POLL_MINUTES, "3").unwrap();
        assert_eq!(core.gmail_poll_interval().as_secs(), 180);
        core.store.kv_set(super::KV_POLL_MINUTES, "0").unwrap();
        assert_eq!(core.gmail_poll_interval().as_secs(), 60);
        core.store.kv_set(super::KV_POLL_MINUTES, "nonsense").unwrap();
        assert_eq!(core.gmail_poll_interval().as_secs(), 600);
    }

    #[test]
    fn forgetting_the_account_keeps_the_client() {
        let (_d, core) = core();
        core.gmail_save_client("id", Some("secret")).unwrap();
        core.secrets.set(super::SECRET_REFRESH_TOKEN, "1//refresh").unwrap();
        core.store.kv_set(super::KV_ACCOUNT, "me@gmail.com").unwrap();
        core.store.kv_set(super::KV_HISTORY_ID, "42").unwrap();
        assert!(core.gmail_connected());
        assert_eq!(core.gmail_state().account.as_deref(), Some("me@gmail.com"));

        core.gmail_forget_account().unwrap();
        assert!(!core.gmail_connected());
        assert_eq!(core.secrets.get(super::SECRET_REFRESH_TOKEN).unwrap(), None);
        assert_eq!(core.store.kv_get(super::KV_HISTORY_ID).unwrap(), None);
        assert_eq!(core.gmail_state().client_id, "id");
        assert_eq!(core.secrets.get(super::SECRET_CLIENT_SECRET).unwrap().as_deref(), Some("secret"));
    }
}
