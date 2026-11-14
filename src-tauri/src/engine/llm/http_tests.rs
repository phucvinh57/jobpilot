//! End-to-end client tests against a throwaway local HTTP server (no real API is contacted).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

#[derive(Clone, Debug)]
struct Captured {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

impl Captured {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

type Requests = Arc<Mutex<Vec<Captured>>>;

/// Serve `responses` (status, body) in order, one per connection; the last one repeats.
/// Returns the server's base URL (`http://127.0.0.1:PORT`) and the captured requests.
async fn serve(responses: Vec<(u16, String)>) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let captured: Requests = Arc::default();
    let log = captured.clone();
    tokio::spawn(async move {
        let mut n = 0usize;
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (status, reply_body) = responses[n.min(responses.len() - 1)].clone();
            n += 1;
            let log = log.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (head_end, content_len) = loop {
                    let read = sock.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        break (pos + 4, len);
                    }
                };
                while buf.len() < head_end + content_len {
                    let read = sock.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..read]);
                }
                let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                let mut lines = head.lines();
                let mut first = lines.next().unwrap_or_default().split_whitespace();
                let method = first.next().unwrap_or_default().to_string();
                let path = first.next().unwrap_or_default().to_string();
                let headers = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                    .collect();
                let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
                log.lock().unwrap().push(Captured { method, path, headers, body });

                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply_body}",
                    reply_body.len()
                );
                let _ = sock.write_all(reply.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (format!("http://{addr}"), captured)
}

fn ok_schema() -> Value {
    json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]})
}

fn openai_reply(text: &str) -> (u16, String) {
    (
        200,
        json!({"choices": [{"finish_reason": "stop", "message": {"role": "assistant", "content": text}}]}).to_string(),
    )
}

fn anthropic_reply(blocks: Value) -> (u16, String) {
    (200, json!({"type": "message", "stop_reason": "end_turn", "content": blocks}).to_string())
}

#[tokio::test]
async fn openai_completion_request_and_headers() {
    let (base, seen) = serve(vec![openai_reply("```json\n{\"ok\": true}\n```")]).await;
    let client =
        OpenAiClient::new("OpenAI", &format!("{base}/v1/"), "gpt-4o", Some("sk-test-123456".into()), "json_schema");
    let out = client.complete_json("SYS", "USER", &ok_schema()).await.unwrap();
    assert_eq!(out, json!({"ok": true}));

    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].path, "/v1/chat/completions");
    assert_eq!(reqs[0].headers["authorization"], "Bearer sk-test-123456");
    let body = reqs[0].json();
    assert_eq!(body["model"], "gpt-4o");
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["messages"][1]["content"], "USER");
}

#[tokio::test]
async fn openai_without_key_sends_no_auth_and_lists_models() {
    let (base, seen) = serve(vec![(200, json!({"data": [{"id": "llama3"}, {"id": "qwen"}]}).to_string())]).await;
    let client = OpenAiClient::new("Ollama", &format!("{base}/v1"), "llama3", None, "json_object");
    assert_eq!(client.list_models().await.unwrap(), vec!["llama3", "qwen"]);
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs[0].method, "GET");
    assert_eq!(reqs[0].path, "/v1/models");
    assert!(!reqs[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn openai_json_object_mode_over_http() {
    let (base, seen) = serve(vec![openai_reply(r#"{"ok": true}"#)]).await;
    let client = OpenAiClient::new("Ollama", &format!("{base}/v1"), "llama3", None, "json_object");
    client.complete_json("SYS", "USER", &ok_schema()).await.unwrap();
    let body = seen.lock().unwrap()[0].json();
    assert_eq!(body["response_format"], json!({"type": "json_object"}));
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("JSON Schema"));
}

#[tokio::test]
async fn openai_invalid_answer_is_retried_with_errors_in_the_user_message() {
    let (base, seen) = serve(vec![openai_reply(r#"{"ok": "yes"}"#), openai_reply(r#"{"ok": true}"#)]).await;
    let client = OpenAiClient::new("P", &format!("{base}/v1"), "m", None, "json_schema");
    let out = client.complete_json("SYS", "USER", &ok_schema()).await.unwrap();
    assert_eq!(out, json!({"ok": true}));

    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    let retry_user = reqs[1].json()["messages"][1]["content"].as_str().unwrap().to_string();
    assert!(retry_user.starts_with("USER"), "{retry_user}");
    assert!(retry_user.contains("rejected") && retry_user.contains("/ok"), "{retry_user}");
}

#[tokio::test]
async fn openai_http_error_is_readable_and_never_leaks_the_key() {
    let key = "sk-very-secret-key";
    let body =
        json!({"error": {"message": format!("Incorrect API key provided: {key}"), "type": "invalid_request_error"}});
    let (base, _) = serve(vec![(401, body.to_string())]).await;
    let client = OpenAiClient::new("OpenAI", &format!("{base}/v1"), "gpt-4o", Some(key.into()), "json_schema");
    let err = client.complete_json("S", "U", &ok_schema()).await.unwrap_err().to_string();
    assert!(err.contains("HTTP 401 Unauthorized"), "{err}");
    assert!(err.contains("Incorrect API key provided"), "{err}");
    assert!(!err.contains(key), "{err}");
}

#[tokio::test]
async fn openai_without_a_model_explains_what_to_do() {
    let client = OpenAiClient::new("P", "http://127.0.0.1:1/v1", "", None, "json_schema");
    let err = client.complete_json("S", "U", &ok_schema()).await.unwrap_err().to_string();
    assert!(err.contains("no model is set"), "{err}");
}

#[tokio::test]
async fn unreachable_server_is_reported_plainly() {
    // Port 1 on localhost: connection refused.
    let client = OpenAiClient::new("Local", "http://127.0.0.1:1/v1", "m", None, "json_schema");
    let err = client.list_models().await.unwrap_err().to_string();
    assert!(err.starts_with("Local: "), "{err}");
    assert!(err.contains("cannot connect"), "{err}");
}

#[tokio::test]
async fn anthropic_completion_request_and_headers() {
    let (base, seen) = serve(vec![anthropic_reply(json!([
        {"type": "thinking", "thinking": "", "signature": "sig"},
        {"type": "text", "text": "{\"ok\": true}"}
    ]))])
    .await;
    let client = AnthropicClient::new("Claude", &base, "", Some("sk-ant-test-123456".into()));
    assert_eq!(client.model, "claude-opus-5-5");
    let out = client.complete_json("SYS", "USER", &ok_schema()).await.unwrap();
    assert_eq!(out, json!({"ok": true}));

    let reqs = seen.lock().unwrap();
    assert_eq!(reqs[0].method, "POST");
    assert_eq!(reqs[0].path, "/v1/messages");
    assert_eq!(reqs[0].headers["x-api-key"], "sk-ant-test-123456");
    assert_eq!(reqs[0].headers["anthropic-version"], "2023-06-01");
    assert_eq!(reqs[0].headers["content-type"], "application/json");
    let body = reqs[0].json();
    assert_eq!(body["model"], "claude-opus-5-5");
    assert_eq!(body["max_tokens"], 16000);
    assert_eq!(body["thinking"], json!({"type": "adaptive"}));
    assert_eq!(body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(body["output_config"]["format"]["schema"]["additionalProperties"], false);
    assert!(body.get("temperature").is_none());
}

#[tokio::test]
async fn anthropic_invalid_answer_is_retried() {
    let (base, seen) = serve(vec![
        anthropic_reply(json!([{"type": "text", "text": "Here is the JSON: {\"ok\": 1}"}])),
        anthropic_reply(json!([{"type": "text", "text": "{\"ok\": false}"}])),
    ])
    .await;
    let client = AnthropicClient::new("Claude", &base, "claude-opus-5-5", Some("sk-ant-test-123456".into()));
    assert_eq!(client.complete_json("S", "USER", &ok_schema()).await.unwrap(), json!({"ok": false}));
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 2);
    let retry_user = reqs[1].json()["messages"][0]["content"].as_str().unwrap().to_string();
    assert!(retry_user.contains("rejected") && retry_user.contains("not valid JSON"), "{retry_user}");
}

#[tokio::test]
async fn anthropic_bad_key_shows_the_provider_message() {
    let body = json!({"type": "error", "error": {"type": "authentication_error", "message": "invalid x-api-key"}});
    let (base, _) = serve(vec![(401, body.to_string())]).await;
    let client = AnthropicClient::new("Claude", &base, "claude-opus-5-5", Some("sk-ant-bad-key".into()));
    let err = client.complete_json("S", "U", &ok_schema()).await.unwrap_err().to_string();
    assert_eq!(err, "Claude: HTTP 401 Unauthorized: invalid x-api-key");
}

#[tokio::test]
async fn anthropic_refusal_and_max_tokens_are_errors() {
    let refusal = json!({"type": "message", "stop_reason": "refusal", "stop_details": {"type": "refusal", "category": "cyber"}, "content": []});
    let (base, _) = serve(vec![(200, refusal.to_string())]).await;
    let client = AnthropicClient::new("Claude", &base, "m", Some("k-123456".into()));
    let err = client.complete_json("S", "U", &ok_schema()).await.unwrap_err().to_string();
    assert!(err.contains("declined"), "{err}");

    let cut =
        json!({"type": "message", "stop_reason": "max_tokens", "content": [{"type": "text", "text": "{\"ok\":"}]});
    let (base, seen) = serve(vec![(200, cut.to_string())]).await;
    let client = AnthropicClient::new("Claude", &base, "m", Some("k-123456".into()));
    let err = client.complete_json("S", "U", &ok_schema()).await.unwrap_err().to_string();
    assert!(err.contains("cut off"), "{err}");
    assert_eq!(seen.lock().unwrap().len(), 1, "a truncated answer is an API-level error, not retried");
}

#[tokio::test]
async fn anthropic_lists_models() {
    let (base, seen) = serve(vec![(
        200,
        json!({"data": [{"id": "claude-opus-5-5"}, {"id": "claude-haiku-5-5"}], "has_more": false}).to_string(),
    )])
    .await;
    let client = AnthropicClient::new("Claude", &format!("{base}/v1"), "m", Some("k-123456".into()));
    assert_eq!(client.list_models().await.unwrap(), vec!["claude-opus-5-5", "claude-haiku-5-5"]);
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs[0].method, "GET");
    assert!(reqs[0].path.starts_with("/v1/models"), "{}", reqs[0].path);
    assert_eq!(reqs[0].headers["x-api-key"], "k-123456");
}

#[tokio::test]
async fn from_provider_builds_the_right_client() {
    let (base, seen) = serve(vec![openai_reply(r#"{"ok": true}"#)]).await;
    let provider = Provider {
        id: 1,
        name: "Local".into(),
        kind: ProviderKind::OpenAi,
        base_url: format!("{base}/v1"),
        model: "m".into(),
        json_mode: "json_schema".into(),
        is_default: true,
    };
    test_llm(from_provider(&provider, None).as_ref()).await.unwrap();
    assert_eq!(seen.lock().unwrap()[0].path, "/v1/chat/completions");
}
