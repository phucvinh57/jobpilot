//! The Codex client: `POST {BASE_URL}/responses` (streamed, server-sent events) with the
//! ChatGPT sign-in's access token, refreshed before it expires and once after a 401. The tokens
//! come from the keyring (own sign-in) or from the Codex CLI's `auth.json` (linked login), read
//! again on every call.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use super::auth::{self, SignInExpired};
use super::tokens::{Login, Tokens};
use super::{BASE_URL, CLI_LOGIN_GONE, DEFAULT_MODEL, MODELS, NOT_SIGNED_IN};
use crate::llm::http::{build_client, describe_net_error, http_error, join_url, HttpError, COMPLETION_TIMEOUT};
use crate::llm::{complete_validated, Llm, LlmFuture};
use crate::store::Provider;
use crate::Secrets;

/// One refresh at a time across clients: the issuer rotates refresh tokens, so two refreshes
/// with the same token would sign the user out. (Against the Codex CLI itself, the file is read
/// again before a refresh: see [`CodexClient::refresh`].)
static REFRESH_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub struct CodexClient {
    name: String,
    model: String,
    provider_id: i64,
    secrets: Secrets,
    base_url: String,
    issuer: String,
    http: reqwest::Client,
}

impl CodexClient {
    /// Client for the saved Codex provider `provider` (tokens read from `secrets` on each call).
    /// The base URL is always [`BASE_URL`]: the ChatGPT token is never sent anywhere else.
    pub fn new(provider: &Provider, secrets: Secrets) -> Self {
        let name = provider.name.trim();
        let model = provider.model.trim();
        Self {
            name: if name.is_empty() { "Codex".into() } else { name.into() },
            model: if model.is_empty() { DEFAULT_MODEL.into() } else { model.into() },
            provider_id: provider.id,
            secrets,
            base_url: BASE_URL.into(),
            issuer: auth::ISSUER.into(),
            http: build_client(),
        }
    }

    /// Other backend and issuer URLs (tests: a loopback fake).
    #[cfg(test)]
    pub(crate) fn with_urls(mut self, base_url: &str, issuer: &str) -> Self {
        self.base_url = base_url.into();
        self.issuer = issuer.into();
        self
    }

    fn not_signed_in(&self) -> anyhow::Error {
        anyhow!("{}: {NOT_SIGNED_IN}", self.name)
    }

    fn login(&self) -> Result<Login> {
        Login::load(&self.secrets, self.provider_id)?.ok_or_else(|| self.not_signed_in())
    }

    /// The current tokens of `login`: a linked CLI login reads them from its file now.
    fn tokens_of(&self, login: &Login) -> Result<Tokens> {
        match login {
            Login::Own(tokens) => Ok(tokens.clone()),
            Login::Linked { path, .. } => {
                auth::read_cli_login(path).map_err(|_| anyhow!("{}: {CLI_LOGIN_GONE}", self.name))
            }
        }
    }

    fn tokens(&self) -> Result<Tokens> {
        self.tokens_of(&self.login()?)
    }

    /// Fresh tokens in place of `stale`: another call (or the Codex CLI, for a linked login) may
    /// have refreshed already, then those are used; else the refresh token is spent and the new
    /// tokens are saved where they came from (the keyring, or the CLI's file).
    pub(super) async fn refresh(&self, stale: &Tokens) -> Result<Tokens> {
        let _one_at_a_time = REFRESH_LOCK.lock().await;
        let login = self.login()?;
        let current = self.tokens_of(&login)?;
        if current.access_token != stale.access_token {
            return Ok(current);
        }
        let fresh = auth::refresh(&self.http, &self.issuer, &current).await.map_err(|e| {
            if e.downcast_ref::<SignInExpired>().is_some() {
                anyhow!("{}: {e}: press Sign in with ChatGPT in Settings > AI providers", self.name)
            } else {
                anyhow!("{}: {e:#}", self.name)
            }
        })?;
        match login {
            Login::Own(_) => Login::Own(fresh.clone()).save(&self.secrets, self.provider_id)?,
            Login::Linked { path, .. } => {
                auth::write_cli_login(&path, &fresh).map_err(|e| anyhow!("{}: {e:#}", self.name))?
            }
        }
        tracing::info!(provider = self.provider_id, "codex: token refreshed");
        Ok(fresh)
    }

    async fn raw_complete(&self, system: &str, user: &str, schema: &Value) -> Result<String> {
        let body = build_request(&self.model, system, user, schema);
        let mut tokens = self.tokens()?;
        if tokens.expires_soon(chrono::Utc::now().timestamp()) {
            tokens = self.refresh(&tokens).await?;
        }
        match self.send(&tokens, &body).await {
            Err(e) if e.downcast_ref::<HttpError>().is_some_and(|h| h.status == 401) => {
                let tokens = self.refresh(&tokens).await?;
                self.send(&tokens, &body).await
            }
            other => other,
        }
    }

    /// One streamed request; the answer's text.
    async fn send(&self, tokens: &Tokens, body: &Value) -> Result<String> {
        let who = self.name.as_str();
        let scrub = |text: &str| crate::redact::redact(text).into_owned();
        let mut req = self
            .http
            .post(join_url(&self.base_url, "responses"))
            .timeout(COMPLETION_TIMEOUT)
            .bearer_auth(&tokens.access_token)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "codex_cli_rs")
            .header("accept", "text/event-stream")
            .json(body);
        if !tokens.account_id.is_empty() {
            req = req.header("chatgpt-account-id", &tokens.account_id);
        }
        let mut resp = req.send().await.map_err(|e| anyhow!("{who}: {}", scrub(&describe_net_error(&e))))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(HttpError { status: status.as_u16(), message: format!("{who}: {}", scrub(&http_error(status, &text))) }
                .into());
        }
        let mut sse = Sse::default();
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let chunk = resp
                .chunk()
                .await
                .map_err(|e| anyhow!("{who}: reading the answer failed: {}", scrub(&describe_net_error(&e))))?;
            let Some(chunk) = chunk else { break };
            pending.extend_from_slice(&chunk);
            while let Some(pos) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=pos).collect();
                if let Some(done) = sse.line(&String::from_utf8_lossy(&line)) {
                    return done.map_err(|e| anyhow!("{who}: {}", scrub(&e.to_string())));
                }
            }
        }
        if !pending.is_empty() {
            if let Some(done) = sse.line(&String::from_utf8_lossy(&pending)) {
                return done.map_err(|e| anyhow!("{who}: {}", scrub(&e.to_string())));
            }
        }
        sse.finish().map_err(|e| anyhow!("{who}: {e}"))
    }
}

impl Llm for CodexClient {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        Box::pin(async move {
            complete_validated(user, schema, |u: String| async move { self.raw_complete(system, &u, schema).await })
                .await
        })
    }

    /// The backend has no model list for a ChatGPT sign-in: a fixed list.
    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(async { Ok(MODELS.iter().map(|m| m.to_string()).collect()) })
    }
}

/// Body of `POST {base}/responses`.
pub(crate) fn build_request(model: &str, system: &str, user: &str, schema: &Value) -> Value {
    json!({
        "model": model,
        "instructions": system,
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": user}],
        }],
        "store": false,
        "stream": true,
        "text": {"format": {"type": "json_schema", "name": "response", "schema": schema, "strict": false}},
    })
}

/// Reads the server-sent events of a `/responses` stream, one line at a time.
#[derive(Default)]
pub(crate) struct Sse {
    text: String,
}

impl Sse {
    /// Feed one line; `Some` once the answer is complete (its text) or failed.
    pub fn line(&mut self, line: &str) -> Option<Result<String>> {
        let data = line.trim_end_matches(['\r', '\n']).strip_prefix("data:")?.trim();
        if data.is_empty() || data == "[DONE]" {
            return None;
        }
        let event: Value = serde_json::from_str(data).ok()?;
        match event.get("type").and_then(Value::as_str).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(delta) = event.get("delta").and_then(Value::as_str) {
                    self.text.push_str(delta);
                }
                None
            }
            "response.completed" => {
                if self.text.is_empty() {
                    self.text = output_text(event.pointer("/response/output"));
                }
                Some(if self.text.trim().is_empty() {
                    Err(anyhow!("the model returned an empty answer"))
                } else {
                    Ok(std::mem::take(&mut self.text))
                })
            }
            "response.incomplete" => {
                let reason = event
                    .pointer("/response/incomplete_details/reason")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown reason");
                Some(Err(anyhow!("the answer was cut off ({reason})")))
            }
            "response.failed" => Some(Err(anyhow!(
                "the request failed: {}",
                event_message(event.pointer("/response/error")).unwrap_or_else(|| "no reason given".into())
            ))),
            "error" => Some(Err(anyhow!(
                "the request failed: {}",
                event_message(event.get("error"))
                    .or_else(|| event_message(Some(&event)))
                    .unwrap_or_else(|| "no reason given".into())
            ))),
            _ => None,
        }
    }

    /// The stream ended without `response.completed` / `response.incomplete`: whatever text
    /// arrived is partial, so it is an error, never an answer.
    pub fn finish(self) -> Result<String> {
        Err(anyhow!("the answer stream ended early (connection closed)"))
    }
}

/// `message` (or `code`) of an error object.
fn event_message(v: Option<&Value>) -> Option<String> {
    let v = v?;
    v.get("message").or_else(|| v.get("code")).and_then(Value::as_str).map(str::to_string)
}

/// The `output_text` parts of `response.output`, joined.
fn output_text(output: Option<&Value>) -> String {
    let Some(items) = output.and_then(Value::as_array) else { return String::new() };
    items
        .iter()
        .filter_map(|item| item.get("content").and_then(Value::as_array))
        .flatten()
        .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

/// SSE parsing of a whole body (tests).
#[cfg(test)]
pub(crate) fn parse_sse(body: &str) -> Result<String> {
    let mut sse = Sse::default();
    for line in body.lines() {
        if let Some(done) = sse.line(line) {
            return done;
        }
    }
    sse.finish()
}
