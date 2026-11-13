//! Anthropic client over raw HTTP (there is no Rust SDK): `POST {base}/v1/messages`,
//! `GET {base}/v1/models`.
//!
//! Request shape (Claude Opus 5.5 family): adaptive thinking, structured output through
//! `output_config.format`, a large `max_tokens`, and none of the removed parameters
//! (`temperature`, `budget_tokens`, assistant prefill). The request is non-streaming with a long
//! timeout; adaptive thinking on a 16k budget finishes well inside it.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::http::{build_client, join_url, send_json, COMPLETION_TIMEOUT, MODELS_TIMEOUT};
use super::{complete_validated, schema, Llm, LlmFuture};

pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
pub const API_VERSION: &str = "2023-06-01";
pub const MAX_TOKENS: u32 = 16_000;

pub struct AnthropicClient {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(name: &str, base_url: &str, model: &str, api_key: Option<String>) -> Self {
        let base = base_url.trim();
        let model = model.trim();
        Self {
            name: if name.trim().is_empty() { "Anthropic".into() } else { name.trim().into() },
            base_url: if base.is_empty() { DEFAULT_BASE_URL.into() } else { base.into() },
            model: if model.is_empty() { DEFAULT_MODEL.into() } else { model.into() },
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            http: build_client(),
        }
    }

    fn who(&self) -> &str {
        &self.name
    }

    fn url(&self, path: &str) -> String {
        messages_url(&self.base_url, path)
    }

    fn headers(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let req = req.header("anthropic-version", API_VERSION).header("content-type", "application/json");
        match &self.api_key {
            Some(key) => req.header("x-api-key", key),
            None => req,
        }
    }

    async fn raw_complete(&self, system: &str, user: &str, schema: &Value) -> Result<String> {
        let body = build_request(&self.model, system, user, schema);
        let req = self.headers(self.http.post(self.url("messages")).timeout(COMPLETION_TIMEOUT)).json(&body);
        let resp = send_json(req, self.who(), self.api_key.as_deref()).await?;
        parse_completion(&resp).map_err(|e| anyhow!("{}: {e}", self.who()))
    }

    async fn raw_models(&self) -> Result<Vec<String>> {
        let req = self.headers(self.http.get(self.url("models")).timeout(MODELS_TIMEOUT)).query(&[("limit", "1000")]);
        let resp = send_json(req, self.who(), self.api_key.as_deref()).await?;
        Ok(parse_models(&resp))
    }
}

impl Llm for AnthropicClient {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        Box::pin(async move {
            complete_validated(user, schema, |u: String| async move { self.raw_complete(system, &u, schema).await })
                .await
        })
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(self.raw_models())
    }
}

/// `{base}/v1/{path}`; a base that already ends in `/v1` is not doubled.
pub(crate) fn messages_url(base: &str, path: &str) -> String {
    let base = base.trim().trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    join_url(&format!("{base}/v1"), path)
}

/// Request body of `POST /v1/messages`.
pub(crate) fn build_request(model: &str, system: &str, user: &str, schema: &Value) -> Value {
    json!({
        "model": model,
        "max_tokens": MAX_TOKENS,
        "system": system,
        "messages": [{"role": "user", "content": user}],
        "thinking": {"type": "adaptive"},
        "output_config": {"format": {"type": "json_schema", "schema": schema::for_anthropic(schema)}},
    })
}

/// Extract the answer text from a `/v1/messages` response (first `text` block; thinking blocks
/// are skipped). `refusal` and `max_tokens` stop reasons are errors.
pub(crate) fn parse_completion(resp: &Value) -> Result<String> {
    match resp.get("stop_reason").and_then(Value::as_str) {
        Some("refusal") => {
            let category = resp.pointer("/stop_details/category").and_then(Value::as_str);
            let explanation = resp.pointer("/stop_details/explanation").and_then(Value::as_str);
            let mut msg = String::from("the model declined to answer (safety refusal)");
            if let Some(c) = category {
                msg.push_str(&format!(" [{c}]"));
            }
            if let Some(e) = explanation.filter(|e| !e.is_empty()) {
                msg.push_str(&format!(": {e}"));
            }
            bail!(msg);
        }
        Some("max_tokens") => bail!(
            "the answer was cut off at the {MAX_TOKENS}-token limit (thinking plus output); shorten the input or simplify the schema"
        ),
        _ => {}
    }
    let text = resp
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    if text.trim().is_empty() {
        bail!("the response has no text content");
    }
    Ok(text.to_string())
}

/// `GET /v1/models` -> ids in the API's order (newest first).
pub(crate) fn parse_models(resp: &Value) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    resp.get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").and_then(Value::as_str))
        .filter(|id| seen.insert(*id))
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({"type": "object", "properties": {"ok": {"type": "boolean"}, "n": {"type": "integer", "minimum": 1}}, "required": ["ok"]})
    }

    #[test]
    fn request_body() {
        let body = build_request("claude-opus-5-5", "SYS", "USER", &schema());
        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["max_tokens"], 16000);
        assert_eq!(body["system"], "SYS");
        assert_eq!(body["messages"], json!([{"role": "user", "content": "USER"}]));
        assert_eq!(body["thinking"], json!({"type": "adaptive"}));
        let format = &body["output_config"]["format"];
        assert_eq!(format["type"], "json_schema");
        // Normalized for Anthropic: additionalProperties false, `minimum` stripped.
        assert_eq!(format["schema"]["additionalProperties"], false);
        assert!(format["schema"]["properties"]["n"].get("minimum").is_none());
        // Removed parameters must never be sent.
        for key in ["temperature", "top_p", "top_k", "budget_tokens"] {
            assert!(body.get(key).is_none(), "{key} must not be sent");
        }
        assert!(body["thinking"].get("budget_tokens").is_none());
        // No assistant prefill: the only message is the user's.
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn urls() {
        assert_eq!(messages_url("https://api.anthropic.com", "messages"), "https://api.anthropic.com/v1/messages");
        assert_eq!(messages_url("https://api.anthropic.com/", "models"), "https://api.anthropic.com/v1/models");
        assert_eq!(messages_url("https://proxy.example/v1", "messages"), "https://proxy.example/v1/messages");
        assert_eq!(
            messages_url("http://localhost:8080/anthropic/", "messages"),
            "http://localhost:8080/anthropic/v1/messages"
        );
    }

    #[test]
    fn parses_first_text_block_skipping_thinking() {
        let resp = json!({
            "stop_reason": "end_turn",
            "content": [
                {"type": "thinking", "thinking": "", "signature": "abc"},
                {"type": "text", "text": "{\"ok\": true}"},
                {"type": "text", "text": "ignored"}
            ]
        });
        assert_eq!(parse_completion(&resp).unwrap(), "{\"ok\": true}");
    }

    #[test]
    fn refusal_and_max_tokens_are_errors() {
        let refusal = json!({"stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber", "explanation": "nope"}, "content": []});
        let err = parse_completion(&refusal).unwrap_err().to_string();
        assert!(err.contains("declined") && err.contains("cyber") && err.contains("nope"), "{err}");

        let bare_refusal = json!({"stop_reason": "refusal", "stop_details": null, "content": []});
        assert!(parse_completion(&bare_refusal).unwrap_err().to_string().contains("declined"));

        let cut = json!({"stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"ok\":"}]});
        assert!(parse_completion(&cut).unwrap_err().to_string().contains("cut off"));

        let empty = json!({"stop_reason": "end_turn", "content": [{"type": "thinking", "thinking": ""}]});
        assert!(parse_completion(&empty).unwrap_err().to_string().contains("no text"));
    }

    #[test]
    fn parses_models() {
        let resp = json!({"data": [{"id": "claude-opus-5-5", "type": "model"}, {"id": "claude-sonnet-5-5"}, {"id": "claude-opus-5-5"}], "has_more": false});
        assert_eq!(parse_models(&resp), vec!["claude-opus-5-5", "claude-sonnet-5-5"]);
    }
}
