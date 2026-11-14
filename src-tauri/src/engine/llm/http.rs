//! Shared HTTP plumbing for the provider clients: one `reqwest::Client`, URL joining, and
//! human-readable errors. Error text never contains the API key.

use std::error::Error as _;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
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
        bail!("{who}: {}", redact(&http_error(status, &body), secret));
    }
    serde_json::from_str(&body).map_err(|_| {
        anyhow!("{who}: the server did not return JSON (HTTP {}): {}", status.as_u16(), redact(&snippet(&body), secret))
    })
}

fn describe_net_error(e: &reqwest::Error) -> String {
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
    fn redacts_keys() {
        assert_eq!(redact("bad key sk-secret-123 given", Some("sk-secret-123")), "bad key *** given");
        assert_eq!(redact("nothing", None), "nothing");
        // Very short "keys" are not redacted (would mangle ordinary words).
        assert_eq!(redact("abc", Some("a")), "abc");
    }
}
