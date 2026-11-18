//! The Codex sign-in tokens: one JSON blob in the keyring under
//! [`crate::llm::codex_tokens_secret`], plus the few JWT claims JobPilot reads from them.

use std::fmt;

use anyhow::{anyhow, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::llm::codex_tokens_secret;
use crate::Secrets;

/// Refresh the access token when it expires within this many seconds.
pub(crate) const REFRESH_MARGIN_SECS: i64 = 5 * 60;

/// The claim object of OpenAI's id token that carries the ChatGPT account.
const AUTH_CLAIM: &str = "https://api.openai.com/auth";

/// What a Codex provider is signed in with. Never `Debug`-printed with its values.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Tokens {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(default)]
    pub id_token: String,
    /// ChatGPT account id (the `chatgpt-account-id` header).
    #[serde(default)]
    pub account_id: String,
    /// The account's email, from the id token.
    #[serde(default)]
    pub email: String,
    /// Expiry of the access token (its JWT `exp`, unix seconds), when it has one.
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl fmt::Debug for Tokens {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tokens").field("expires_at", &self.expires_at).finish_non_exhaustive()
    }
}

impl Tokens {
    /// Build from raw tokens: the email and account id come from the id token's claims (an
    /// explicit `account_id` wins), the expiry from the access token's `exp`.
    pub fn from_raw(access_token: &str, refresh_token: &str, id_token: &str, account_id: Option<&str>) -> Self {
        let claims = IdClaims::decode(id_token);
        let account_id = account_id
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string)
            .or(claims.account_id)
            .unwrap_or_default();
        let tokens = Self {
            access_token: access_token.trim().to_string(),
            refresh_token: refresh_token.trim().to_string(),
            id_token: id_token.trim().to_string(),
            account_id,
            email: claims.email.unwrap_or_default(),
            expires_at: jwt_exp(access_token),
        };
        tokens.register();
        tokens
    }

    /// The access token expires within [`REFRESH_MARGIN_SECS`] of `now` (unix seconds).
    pub fn expires_soon(&self, now: i64) -> bool {
        self.expires_at.is_some_and(|exp| exp - now <= REFRESH_MARGIN_SECS)
    }

    /// The email to show (a fallback when the id token named none).
    pub fn display_email(&self) -> String {
        if self.email.trim().is_empty() {
            "ChatGPT account".to_string()
        } else {
            self.email.clone()
        }
    }

    /// Scrub every token from logs and error text from now on.
    fn register(&self) {
        for value in [&self.access_token, &self.refresh_token, &self.id_token] {
            crate::redact::register(value);
        }
    }

    /// Read provider `provider_id`'s tokens from the keyring (`None`: not signed in).
    pub fn load(secrets: &Secrets, provider_id: i64) -> Result<Option<Self>> {
        let Some(raw) = secrets.get(&codex_tokens_secret(provider_id))?.filter(|s| !s.trim().is_empty()) else {
            return Ok(None);
        };
        // The serde error never quotes the value; the blob itself is already registered with redact.
        let tokens: Self = serde_json::from_str(&raw)
            .map_err(|_| anyhow!("the stored ChatGPT sign-in of AI provider {provider_id} is unreadable"))?;
        tokens.register();
        Ok((!tokens.access_token.is_empty()).then_some(tokens))
    }

    /// Write the tokens to the keyring.
    pub fn save(&self, secrets: &Secrets, provider_id: i64) -> Result<()> {
        self.register();
        let raw = serde_json::to_string(self).context("encoding the ChatGPT sign-in")?;
        secrets.set(&codex_tokens_secret(provider_id), &raw)
    }
}

/// The id-token claims JobPilot uses.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct IdClaims {
    pub email: Option<String>,
    pub account_id: Option<String>,
}

impl IdClaims {
    /// Read the claims of an id token. The signature is NOT checked: the token comes straight
    /// from the issuer's token endpoint over TLS (or from the Codex CLI's own login file), and it
    /// is only used to show the account and to address it, never to grant anything.
    pub fn decode(id_token: &str) -> Self {
        let Some(payload) = jwt_payload(id_token) else { return Self::default() };
        let text = |v: Option<&Value>| v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
        Self {
            email: text(payload.get("email")),
            account_id: text(payload.get(AUTH_CLAIM).and_then(|a| a.get("chatgpt_account_id"))),
        }
    }
}

/// The JSON payload (middle part) of a JWT, unverified.
fn jwt_payload(jwt: &str) -> Option<Value> {
    let mut parts = jwt.trim().split('.');
    let (Some(_header), Some(payload), Some(_sig)) = (parts.next(), parts.next(), parts.next()) else { return None };
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// `exp` of a JWT (unix seconds); `None` when it is not a JWT or has none.
pub(crate) fn jwt_exp(jwt: &str) -> Option<i64> {
    jwt_payload(jwt)?.get("exp")?.as_i64()
}
