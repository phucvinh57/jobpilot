//! OpenAI-compatible client: `POST {base}/chat/completions`, `GET {base}/models`.
//! Covers OpenAI, OpenRouter, Groq, DeepSeek, Ollama, LM Studio, ...

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};

use super::http::{build_client, join_url, refused_feature, send_json, COMPLETION_TIMEOUT, MODELS_TIMEOUT};
use super::learned::Learned;
use super::{complete_validated, schema, Llm, LlmFuture};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
/// Default model of the official OpenAI API.
pub const DEFAULT_MODEL: &str = "gpt-5";

/// `Provider::json_mode` value of [`JsonMode::Schema`].
pub const JSON_MODE_SCHEMA: &str = "json_schema";
/// `Provider::json_mode` value of [`JsonMode::Object`].
pub const JSON_MODE_OBJECT: &str = "json_object";
/// `Provider::json_mode` value of [`JsonMode::Prompt`].
pub const JSON_MODE_PROMPT: &str = "prompt";

/// How a `/chat/completions` request asks for JSON. A compatible endpoint that refuses one mode
/// is retried in the next, simpler one (Schema -> Object -> Prompt).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsonMode {
    /// `response_format: {type: "json_schema", ...}`.
    Schema,
    /// `response_format: {type: "json_object"}`, schema in the system prompt.
    Object,
    /// No `response_format`; schema in the system prompt (the answer is validated anyway).
    Prompt,
}

impl JsonMode {
    /// From the stored string; unknown or empty = `Schema`.
    pub fn parse(s: &str) -> Self {
        match s.trim() {
            JSON_MODE_OBJECT => JsonMode::Object,
            JSON_MODE_PROMPT => JsonMode::Prompt,
            _ => JsonMode::Schema,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            JsonMode::Schema => JSON_MODE_SCHEMA,
            JsonMode::Object => JSON_MODE_OBJECT,
            JsonMode::Prompt => JSON_MODE_PROMPT,
        }
    }

    /// The next, simpler mode (`None` after `Prompt`).
    fn simpler(self) -> Option<Self> {
        match self {
            JsonMode::Schema => Some(JsonMode::Object),
            JsonMode::Object => Some(JsonMode::Prompt),
            JsonMode::Prompt => None,
        }
    }
}

/// Words in a 400/422 error that mean "this endpoint does not take that `response_format`" (no
/// bare "schema" or "invalid": those also name errors a simpler request cannot fix).
const REFUSAL_WORDS: &[&str] =
    &["response_format", "json_schema", "json_object", "structured", "not supported", "unsupported"];

pub struct OpenAiClient {
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    /// The mode the first call starts in (the provider's stored, learned mode).
    pub json_mode: JsonMode,
    /// Step down to a simpler [`JsonMode`] when the endpoint refuses one (on by default: an
    /// official model may refuse `json_schema` too).
    pub fallback: bool,
    /// The mode that last worked; later calls start from it.
    learned: Learned<JsonMode>,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(name: &str, base_url: &str, model: &str, api_key: Option<String>, json_mode: &str) -> Self {
        let base = base_url.trim();
        Self {
            name: if name.trim().is_empty() { "OpenAI-compatible".into() } else { name.trim().into() },
            base_url: if base.is_empty() { DEFAULT_BASE_URL.into() } else { base.into() },
            model: model.trim().into(),
            api_key: api_key.filter(|k| !k.trim().is_empty()),
            json_mode: JsonMode::parse(json_mode),
            fallback: true,
            learned: Learned::default(),
            http: build_client(),
        }
    }

    /// Turn the JSON mode fallback on or off.
    pub fn with_fallback(mut self, fallback: bool) -> Self {
        self.fallback = fallback;
        self
    }

    /// The mode that worked on the last successful call (`None` before one).
    pub fn learned_mode(&self) -> Option<JsonMode> {
        self.learned.get()
    }

    fn who(&self) -> &str {
        &self.name
    }

    async fn raw_complete(&self, system: &str, user: &str, schema: &Value) -> Result<String> {
        if self.model.is_empty() {
            bail!("{}: no model is set (pick one with \"Load models\" in Settings)", self.who());
        }
        let mut mode = self.learned_mode().unwrap_or(self.json_mode);
        loop {
            match self.raw_complete_in(mode, system, user, schema).await {
                Ok(text) => {
                    self.learned.set(mode);
                    return Ok(text);
                }
                Err(e) => match mode.simpler() {
                    Some(next) if self.fallback && refused_feature(&e, self.who(), REFUSAL_WORDS) => {
                        tracing::info!(from = mode.as_str(), to = next.as_str(), "llm: endpoint refused the JSON mode, retrying");
                        mode = next;
                    }
                    _ => return Err(e),
                },
            }
        }
    }

    async fn raw_complete_in(&self, mode: JsonMode, system: &str, user: &str, schema: &Value) -> Result<String> {
        let body = build_request(&self.model, system, user, schema, mode);
        let mut req =
            self.http.post(join_url(&self.base_url, "chat/completions")).timeout(COMPLETION_TIMEOUT).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = send_json(req, self.who(), self.api_key.as_deref()).await?;
        parse_completion(&resp).map_err(|e| anyhow!("{}: {e}", self.who()))
    }

    async fn raw_models(&self) -> Result<Vec<String>> {
        let mut req = self.http.get(join_url(&self.base_url, "models")).timeout(MODELS_TIMEOUT);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }
        let resp = send_json(req, self.who(), self.api_key.as_deref()).await?;
        Ok(parse_models(&resp))
    }
}

impl Llm for OpenAiClient {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        Box::pin(async move {
            complete_validated(user, schema, |u: String| async move { self.raw_complete(system, &u, schema).await })
                .await
        })
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(self.raw_models())
    }

    fn learned_json_mode(&self) -> Option<&'static str> {
        self.learned_mode().map(JsonMode::as_str)
    }
}

/// Request body of `POST /chat/completions` in `mode`.
pub(crate) fn build_request(model: &str, system: &str, user: &str, schema: &Value, mode: JsonMode) -> Value {
    let system = match mode {
        JsonMode::Schema => system.to_string(),
        JsonMode::Object | JsonMode::Prompt => format!("{system}\n\n{}", schema::schema_instructions(schema)),
    };
    let mut body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let response_format = match mode {
        JsonMode::Schema => {
            Some(json!({"type": "json_schema", "json_schema": {"name": "response", "schema": schema, "strict": false}}))
        }
        JsonMode::Object => Some(json!({"type": "json_object"})),
        JsonMode::Prompt => None,
    };
    if let (Some(rf), Some(obj)) = (response_format, body.as_object_mut()) {
        obj.insert("response_format".into(), rf);
    }
    body
}

/// Extract the assistant text from a `/chat/completions` response.
pub(crate) fn parse_completion(resp: &Value) -> Result<String> {
    let choice = resp.pointer("/choices/0").ok_or_else(|| anyhow!("the response has no choices"))?;
    if let Some(refusal) = choice.pointer("/message/refusal").and_then(Value::as_str).filter(|r| !r.is_empty()) {
        bail!("the model refused the request: {refusal}");
    }
    match choice.get("finish_reason").and_then(Value::as_str) {
        Some("length") => bail!("the answer was cut off (token limit reached); try a model with a larger output limit"),
        Some("content_filter") => bail!("the answer was blocked by the provider's content filter"),
        _ => {}
    }
    let content = choice.pointer("/message/content");
    let text = match content {
        Some(Value::String(s)) => s.clone(),
        // Some gateways return content parts.
        Some(Value::Array(parts)) => {
            parts.iter().filter_map(|p| p.get("text").and_then(Value::as_str)).collect::<Vec<_>>().join("")
        }
        _ => String::new(),
    };
    if text.trim().is_empty() {
        bail!("the response has no text content");
    }
    Ok(text)
}

/// `GET /models` -> sorted, de-duplicated ids. Accepts `{"data": [{"id"}]}` and a bare array.
pub(crate) fn parse_models(resp: &Value) -> Vec<String> {
    let items = resp.get("data").and_then(Value::as_array).or_else(|| resp.as_array());
    let mut ids: Vec<String> = items
        .into_iter()
        .flatten()
        .filter_map(|m| m.get("id").or_else(|| m.get("name")).and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema() -> Value {
        json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]})
    }

    #[test]
    fn json_schema_request_body() {
        let body = build_request("gpt-4o", "SYS", "USER", &schema(), JsonMode::Schema);
        assert_eq!(body["model"], "gpt-4o");
        assert_eq!(body["messages"][0], json!({"role": "system", "content": "SYS"}));
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "USER"}));
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(body["response_format"]["json_schema"]["name"], "response");
        assert_eq!(body["response_format"]["json_schema"]["strict"], false);
        assert_eq!(body["response_format"]["json_schema"]["schema"], schema());
    }

    #[test]
    fn json_object_request_body_appends_schema_to_system_prompt() {
        let body = build_request("llama3", "SYS", "USER", &schema(), JsonMode::Object);
        assert_eq!(body["response_format"], json!({"type": "json_object"}));
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("SYS\n\n"));
        assert!(system.contains("JSON Schema"));
        assert!(system.contains("\"ok\""));
        assert_eq!(body["messages"][1]["content"], "USER");
    }

    #[test]
    fn prompt_request_body_has_no_response_format() {
        let body = build_request("m", "SYS", "USER", &schema(), JsonMode::Prompt);
        assert!(body.get("response_format").is_none());
        let system = body["messages"][0]["content"].as_str().unwrap();
        assert!(system.starts_with("SYS\n\n") && system.contains("JSON Schema"));
    }

    #[test]
    fn json_modes_parse_from_stored_strings() {
        for mode in [JsonMode::Schema, JsonMode::Object, JsonMode::Prompt] {
            assert_eq!(JsonMode::parse(mode.as_str()), mode);
        }
        assert_eq!(JsonMode::parse(""), JsonMode::Schema);
        assert_eq!(JsonMode::parse("bogus"), JsonMode::Schema);
    }

    #[test]
    fn parses_completions() {
        let ok = json!({"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": "{\"ok\":true}"}}]});
        assert_eq!(parse_completion(&ok).unwrap(), "{\"ok\":true}");

        let parts = json!({"choices": [{"message": {"content": [{"type": "text", "text": "{\"ok\":"}, {"type": "text", "text": "true}"}]}}]});
        assert_eq!(parse_completion(&parts).unwrap(), "{\"ok\":true}");

        let cut = json!({"choices": [{"finish_reason": "length", "message": {"content": "{\"ok\":"}}]});
        assert!(parse_completion(&cut).unwrap_err().to_string().contains("cut off"));

        let refused = json!({"choices": [{"message": {"content": null, "refusal": "I can't help with that"}}]});
        assert!(parse_completion(&refused).unwrap_err().to_string().contains("refused"));

        assert!(parse_completion(&json!({"choices": []})).is_err());
        assert!(parse_completion(&json!({"choices": [{"message": {"content": ""}}]})).is_err());
    }

    #[test]
    fn parses_models() {
        let resp = json!({"object": "list", "data": [{"id": "gpt-4o"}, {"id": "gpt-3.5"}, {"id": "gpt-4o"}]});
        assert_eq!(parse_models(&resp), vec!["gpt-3.5", "gpt-4o"]);
        assert_eq!(parse_models(&json!([{"id": "b"}, {"name": "a"}])), vec!["a", "b"]);
        assert!(parse_models(&json!({"unexpected": 1})).is_empty());
    }
}
