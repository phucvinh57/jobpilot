//! OpenAI Codex through the user's ChatGPT sign-in (`ProviderKind::Codex`).
//!
//! Only the backend's base URL lives here so far; the sign-in flow and the client (`POST
//! {BASE_URL}/responses`, streamed) come next. Until then `llm::from_provider` gives a Codex
//! provider a client whose calls fail with a plain message. The sign-in tokens are kept in the
//! keyring under [`super::codex_tokens_secret`].

/// The Codex backend of ChatGPT (fixed: a Codex provider has no user-entered base URL).
pub const BASE_URL: &str = "https://chatgpt.com/backend-api/codex";
/// Default Codex model.
pub const DEFAULT_MODEL: &str = "gpt-5-codex";

// ---------------------------------------------------------------- Core API (stubs: filled next)

use anyhow::{bail, Result};

use crate::Core;

/// A Codex CLI login can be reused (`$CODEX_HOME/auth.json`, default `~/.codex/auth.json`, holds tokens).
pub fn cli_login_available() -> bool {
    false
}

impl Core {
    /// Sign in with ChatGPT in the system browser (loopback + PKCE) for Codex provider
    /// `provider_id`; the tokens go to the keyring. Returns the account's email.
    pub async fn codex_sign_in(&self, provider_id: i64) -> Result<String> {
        let _ = provider_id;
        bail!("Codex sign-in is not available yet")
    }

    /// Abort a sign-in that waits for the browser.
    pub fn codex_cancel_sign_in(&self) {}

    /// Copy the Codex CLI's login into the keyring for provider `provider_id`. Returns the account's email.
    pub fn codex_import_cli_login(&self, provider_id: i64) -> Result<String> {
        let _ = provider_id;
        bail!("Codex sign-in is not available yet")
    }

    /// The email of the account provider `provider_id` is signed in with (`None`: not signed in).
    pub fn codex_account(&self, provider_id: i64) -> Result<Option<String>> {
        let _ = provider_id;
        Ok(None)
    }
}
