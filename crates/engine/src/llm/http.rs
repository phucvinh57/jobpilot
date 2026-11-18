//! Shared HTTP plumbing for the provider clients: one `reqwest::Client`, URL joining, and
//! human-readable errors. Error text never contains the API key.

use std::error::Error as _;
use std::time::Duration;

use anyhow::{anyhow, Result};
use reqwest::{RequestBuilder, StatusCode};
use serde_json::Value;

/// Completions may think for minutes (Anthropic `max_tokens` 16000, adaptive thinking), so the
/// non-streaming request gets a generous timeout.
pub(crate) const COMPLETION_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// Listing models is a cheap call.
pub(crate) const MODELS_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .tcp_keepalive(Duration::from_secs(30))
        .user_agent(concat!("jobpilot/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// A non-2xx answer from a provider. `message` is the whole human-readable text
/// (`"{who}: HTTP 400 Bad Request: ..."`, key redacted); callers that need the status (the JSON
/// mode fallback) get it with `err.downcast_ref::<HttpError>()`.
#[derive(Debug)]
pub(crate) struct HttpError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for HttpError {}

/// `err` is an HTTP 400/422 from `who` whose provider message mentions one of `words`
/// (lowercase): the endpoint refused a request feature, so a simpler request may work.
pub(crate) fn refused_feature(err: &anyhow::Error, who: &str, words: &[&str]) -> bool {
    let Some(http) = err.downcast_ref::<HttpError>() else { return false };
    if !matches!(http.status, 400 | 422) {
        return false;
    }
    let text = http.message.strip_prefix(who).unwrap_or(&http.message).to_lowercase();
    words.iter().any(|w| text.contains(w))
}

/// `base` + `/` + `path`, tolerating a trailing slash on `base`.
pub(crate) fn join_url(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim().trim_end_matches('/'), path.trim_start_matches('/'))
}

/// Send `req` and return the parsed JSON body of a 2xx response.
///
/// Failures become `"{who}: HTTP 401 Unauthorized: invalid x-api-key"`-style messages built from
/// the provider's own error body; network failures say which endpoint could not be reached.
pub(crate) async fn send_json(req: RequestBuilder, who: &str, secret: Option<&str>) -> Result<Value> {
    let resp = req.send().await.map_err(|e| anyhow!("{who}: {}", redact(&describe_net_error(&e), secret)))?;
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| anyhow!("{who}: reading the response failed: {}", redact(&describe_net_error(&e), secret)))?;
    if !status.is_success() {
        return Err(HttpError {
            status: status.as_u16(),
            message: format!("{who}: {}", redact(&http_error(status, &body), secret)),
        }
        .into());
    }
    serde_json::from_str(&body).map_err(|_| {
        anyhow!("{who}: the server did not return JSON (HTTP {}): {}", status.as_u16(), redact(&snippet(&body), secret))
    })
}

pub(crate) fn describe_net_error(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        return "the request timed out".into();
    }
    let mut parts = vec![];
    let mut cur: Option<&(dyn std::error::Error + 'static)> = e.source();
    while let Some(err) = cur {
        parts.push(err.to_string());
        cur = err.source();
    }
    let reason = if parts.is_empty() { "network error".to_string() } else { parts.join(": ") };
    if e.is_connect() {
        format!("cannot connect ({reason})")
    } else {
        reason
    }
}

/// `HTTP 401 Unauthorized: invalid x-api-key`.
pub(crate) fn http_error(status: StatusCode, body: &str) -> String {
    let reason = status.canonical_reason().unwrap_or("error");
    match error_message(body) {
        Some(m) => format!("HTTP {} {reason}: {m}", status.as_u16()),
        None => {
            let s = snippet(body);
            if s.is_empty() {
                format!("HTTP {} {reason}", status.as_u16())
            } else {
                format!("HTTP {} {reason}: {s}", status.as_u16())
            }
        }
    }
}

/// Best-effort extraction of the provider's error message from a JSON error body:
/// OpenAI `{"error":{"message"}}`, Anthropic `{"type":"error","error":{"message"}}`,
/// `{"error":"text"}`, `{"message":..}`, `{"detail":..}`.
pub(crate) fn error_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let pick = |v: &Value| v.as_str().map(str::to_string);
    let msg = v
        .pointer("/error/message")
        .and_then(pick)
        .or_else(|| v.get("error").and_then(pick))
        .or_else(|| v.get("message").and_then(pick))
        .or_else(|| v.get("detail").and_then(pick))?;
    let msg = msg.trim();
    (!msg.is_empty()).then(|| msg.to_string())
}

fn snippet(body: &str) -> String {
    let t = body.trim();
    if t.chars().count() > 300 {
        format!("{}...", t.chars().take(300).collect::<String>())
    } else {
        t.to_string()
    }
}

/// Replace the API key (if a provider echoes it back) with `***`.
pub(crate) fn redact(text: &str, secret: Option<&str>) -> String {
    match secret {
        Some(s) if s.len() >= 6 => text.replace(s, "***"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_urls() {
        assert_eq!(
            join_url("https://api.openai.com/v1/", "/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(join_url("http://localhost:11434/v1", "models"), "http://localhost:11434/v1/models");
    }

    #[test]
    fn extracts_provider_error_messages() {
        let anthropic = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(http_error(StatusCode::UNAUTHORIZED, anthropic), "HTTP 401 Unauthorized: invalid x-api-key");

        let openai = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","code":"invalid_api_key"}}"#;
        assert_eq!(http_error(StatusCode::UNAUTHORIZED, openai), "HTTP 401 Unauthorized: Incorrect API key provided");

        assert_eq!(
            http_error(StatusCode::BAD_REQUEST, r#"{"error":"model not found"}"#),
            "HTTP 400 Bad Request: model not found"
        );
        assert_eq!(
            http_error(StatusCode::BAD_GATEWAY, "<html>upstream down</html>"),
            "HTTP 502 Bad Gateway: <html>upstream down</html>"
        );
        assert_eq!(http_error(StatusCode::NOT_FOUND, ""), "HTTP 404 Not Found");
    }

    #[test]
    fn http_error_keeps_the_text_and_the_status() {
        let err: anyhow::Error = HttpError { status: 400, message: "P: HTTP 400 Bad Request: no".into() }.into();
        assert_eq!(err.to_string(), "P: HTTP 400 Bad Request: no");
        assert_eq!(err.downcast_ref::<HttpError>().map(|e| e.status), Some(400));
    }

    #[test]
    fn refused_feature_needs_400_or_422_and_a_keyword() {
        let err = |status, message: &str| -> anyhow::Error { HttpError { status, message: message.into() }.into() };
        let words = ["response_format"];
        assert!(refused_feature(&err(400, "P: HTTP 400 Bad Request: response_format is invalid"), "P", &words));
        assert!(refused_feature(&err(422, "P: HTTP 422 Unprocessable Entity: Response_Format"), "P", &words));
        assert!(!refused_feature(&err(401, "P: HTTP 401 Unauthorized: response_format"), "P", &words));
        assert!(!refused_feature(&err(400, "P: HTTP 400 Bad Request: model not found"), "P", &words));
        // The provider's own name is not part of the message that is searched.
        assert!(!refused_feature(&err(400, "response_format proxy: HTTP 400 Bad Request: x"), "response_format proxy", &words));
        assert!(!refused_feature(&anyhow::anyhow!("response_format"), "P", &words));
    }

    #[test]
    fn redacts_keys() {
        assert_eq!(redact("bad key sk-secret-123 given", Some("sk-secret-123")), "bad key *** given");
        assert_eq!(redact("nothing", None), "nothing");
        // Very short "keys" are not redacted (would mangle ordinary words).
        assert_eq!(redact("abc", Some("a")), "abc");
    }
}
