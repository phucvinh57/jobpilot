//! OpenAI Codex through the user's ChatGPT sign-in (`ProviderKind::Codex`).
//!
//! ```text
//! core.codex_sign_in(provider_id).await?;      // browser sign-in (loopback :1455 + PKCE) -> email
//! core.codex_import_cli_login(provider_id)?;   // or reuse the Codex CLI's ~/.codex/auth.json
//! let llm = core.llm_for(provider_id)?;        // CodexClient: POST {BASE_URL}/responses, streamed
//! ```
//!
//! The tokens are one JSON blob in the keyring under [`super::codex_tokens_secret`] (never in the
//! database, logs or errors); the access token is refreshed before it expires and once after a 401.
//! [`super::from_provider`] has no keyring, so it gives a Codex provider a client that says "not
//! signed in"; [`Core::llm_for`] and [`Core::llm_for_form`] build the real [`CodexClient`].

mod auth;
mod client;
mod tokens;

#[cfg(test)]
mod tests;

use std::path::Path;

use anyhow::{bail, Context, Result};

pub use client::CodexClient;
use tokens::Tokens;

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
        tokens.save(&self.secrets, provider_id)?;
        tracing::info!(provider = provider_id, "codex: signed in");
        Ok(tokens.display_email())
    }

    /// Abort a sign-in that waits for the browser.
    pub fn codex_cancel_sign_in(&self) {
        auth::CANCEL.notify_waiters();
    }

    /// Copy the Codex CLI's login into the keyring for provider `provider_id`. Returns the account's email.
    pub fn codex_import_cli_login(&self, provider_id: i64) -> Result<String> {
        let path = auth::cli_auth_path().context("no home directory to find the Codex CLI's login in")?;
        self.codex_import_from(provider_id, &path)
    }

    pub(crate) fn codex_import_from(&self, provider_id: i64, path: &Path) -> Result<String> {
        self.require_codex(provider_id)?;
        let tokens = auth::read_cli_login(path)?;
        tokens.save(&self.secrets, provider_id)?;
        tracing::info!(provider = provider_id, "codex: imported CLI login");
        Ok(tokens.display_email())
    }

    /// The email of the account provider `provider_id` is signed in with (`None`: not signed in).
    /// Reads the keyring only (no network).
    pub fn codex_account(&self, provider_id: i64) -> Result<Option<String>> {
        match Tokens::load(&self.secrets, provider_id) {
            Ok(tokens) => Ok(tokens.map(|t| t.display_email())),
            Err(e) => {
                tracing::warn!(provider = provider_id, "codex: {e:#}");
                Ok(None)
            }
        }
    }

    /// Provider `provider_id` is a saved Codex provider.
    fn require_codex(&self, provider_id: i64) -> Result<()> {
        if provider_id <= 0 {
            bail!("save the provider first, then sign in with ChatGPT");
        }
        let provider = self
            .store
            .get_provider(provider_id)?
            .with_context(|| format!("AI provider {provider_id} no longer exists"))?;
        if provider.kind != ProviderKind::Codex {
            bail!("{} is not a Codex provider", provider.name);
        }
        Ok(())
    }
}
