//! OpenAI Codex through the user's ChatGPT sign-in (`ProviderKind::Codex`).
//!
//! ```text
//! core.codex_sign_in(provider_id).await?;      // browser sign-in (loopback :1455 + PKCE) -> email
//! core.codex_import_cli_login(provider_id)?;   // or link the Codex CLI's ~/.codex/auth.json
//! let llm = core.llm_for(provider_id)?;        // CodexClient: POST {BASE_URL}/responses, streamed
//! ```
//!
//! What a provider is signed in with is one JSON blob in the keyring under
//! [`super::codex_tokens_secret`] (never in the database, logs or errors):
//!
//! - its own sign-in ([`Core::codex_sign_in`]): the tokens themselves;
//! - a *linked* Codex CLI login ([`Core::codex_import_cli_login`]): `{"source": "cli", "path": ...}`
//!   and the account's email, no tokens. The issuer rotates refresh tokens, so a copy would sign
//!   out whichever of JobPilot and the CLI refreshed second. Instead every call reads the current
//!   tokens from the CLI's `auth.json` (a refresh done by the CLI is picked up), and a refresh done
//!   by JobPilot is written back into that file the way the CLI does it: only `tokens.*` and
//!   `last_refresh` change, every other field is kept, the file is replaced atomically (temp file +
//!   rename in the same folder) and keeps its permissions (0600). The file is read again under the
//!   refresh lock first, in case the CLI refreshed meanwhile.
//!
//! The access token is refreshed before it expires and once after a 401. [`super::from_provider`]
//! has no keyring, so it gives a Codex provider a client that says "not signed in";
//! [`Core::llm_for`] and [`Core::llm_for_form`] build the real [`CodexClient`].

mod auth;
mod client;
mod tokens;

#[cfg(test)]
mod tests;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};

pub use client::CodexClient;
use tokens::Login;

use crate::store::ProviderKind;
use crate::Core;

/// The Codex backend of ChatGPT (fixed: a Codex provider has no user-entered base URL).
pub const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Default Codex model.
pub const DEFAULT_MODEL: &str = "gpt-5-codex";
/// The models offered for a ChatGPT sign-in (the backend lists none).
pub const MODELS: [&str; 4] = ["gpt-5-codex", "gpt-5", "gpt-5.1-codex", "gpt-5.1"];
/// Why a Codex provider without tokens cannot answer.
pub const NOT_SIGNED_IN: &str = "not signed in: press Sign in with ChatGPT in Settings > AI providers";
/// The issuer refused the refresh: the sign-in was revoked, expired or reused.
pub const SIGN_IN_INVALID: &str = "the ChatGPT sign-in is no longer valid";
/// A linked Codex CLI login whose `auth.json` no longer holds a ChatGPT login.
pub const CLI_LOGIN_GONE: &str = "The Codex CLI login is gone: sign in again";
/// The start of every error that only a new sign-in fixes (`health::classify` looks for them).
pub const SIGN_IN_ERRORS: [&str; 3] = [NOT_SIGNED_IN, SIGN_IN_INVALID, CLI_LOGIN_GONE];

/// A Codex CLI login can be reused (`$CODEX_HOME/auth.json`, default `~/.codex/auth.json`, holds tokens).
pub fn cli_login_available() -> bool {
    auth::cli_auth_path().is_some_and(|p| auth::cli_login_at(&p))
}

impl Core {
    /// Sign in with ChatGPT in the system browser (loopback + PKCE) for Codex provider
    /// `provider_id`; the tokens go to the keyring. Returns the account's email.
    pub async fn codex_sign_in(&self, provider_id: i64) -> Result<String> {
        self.require_codex(provider_id)?;
        let listener = auth::bind_callback().await?;
        let http = crate::mail::oauth::http_client();
        let tokens = auth::sign_in(&http, &listener, auth::ISSUER, auth::REDIRECT_URI, |url| {
            crate::mail::oauth::open_browser(url)
        })
        .await?;
        drop(listener);
        let email = tokens.display_email();
        Login::Own(tokens).save(&self.secrets, provider_id)?;
        tracing::info!(provider = provider_id, "codex: signed in");
        Ok(email)
    }

    /// Abort a sign-in that waits for the browser.
    pub fn codex_cancel_sign_in(&self) {
        auth::CANCEL.notify_waiters();
    }

    /// Link provider `provider_id` to the Codex CLI's login (see the module doc: the tokens stay
    /// in the CLI's `auth.json`, shared with the CLI). Returns the account's email.
    pub fn codex_import_cli_login(&self, provider_id: i64) -> Result<String> {
        self.require_codex(provider_id)?;
        let path = auth::cli_auth_path().context("no home directory to find the Codex CLI's login in")?;
        self.codex_link(provider_id, path)
    }

    pub(crate) fn codex_link(&self, provider_id: i64, path: PathBuf) -> Result<String> {
        self.require_codex(provider_id)?;
        let email = auth::read_cli_login(&path)?.display_email();
        Login::Linked { path, email: email.clone() }.save(&self.secrets, provider_id)?;
        tracing::info!(provider = provider_id, "codex: linked the CLI login");
        Ok(email)
    }

    /// The email of the account provider `provider_id` is signed in with (`None`: not signed in).
    /// Reads the keyring, and for a linked CLI login its `auth.json` (no network); when that file
    /// is gone, the email saved with the link (the calls then say to sign in again).
    pub fn codex_account(&self, provider_id: i64) -> Result<Option<String>> {
        match Login::load(&self.secrets, provider_id) {
            Ok(None) => Ok(None),
            Ok(Some(Login::Own(tokens))) => Ok(Some(tokens.display_email())),
            Ok(Some(Login::Linked { path, email })) => {
                Ok(Some(auth::read_cli_login(&path).map(|t| t.display_email()).unwrap_or(email)))
            }
            Err(e) => {
                tracing::warn!(provider = provider_id, "codex: {e:#}");
                Ok(None)
            }
        }
    }

    /// Provider `provider_id` is a saved Codex provider.
    fn require_codex(&self, provider_id: i64) -> Result<()> {
        if provider_id <= 0 {
            bail!("Save the provider, then sign in.");
        }
        let provider = self
            .store
            .get_provider(provider_id)?
            .with_context(|| format!("AI provider {provider_id} no longer exists"))?;
        if provider.kind != ProviderKind::Codex {
            bail!("Only a Codex provider signs in with ChatGPT.");
        }
        Ok(())
    }
}
