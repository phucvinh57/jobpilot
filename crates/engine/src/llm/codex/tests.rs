//! Codex tests: sign-in pieces, the token blob, the CLI login, SSE parsing, and the client
//! against a throwaway loopback server (no real service is contacted).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::auth;
use super::client::{build_request, parse_sse, CodexClient};
use super::tokens::{jwt_exp, IdClaims, Login, Tokens};
use super::*;
use crate::llm::{codex_tokens_secret, Llm};
use crate::store::Provider;
use crate::{Paths, Secrets};

// ---------------------------------------------------------------- helpers

fn b64(v: &Value) -> String {
    URL_SAFE_NO_PAD.encode(v.to_string())
}

fn jwt(payload: Value) -> String {
    format!("{}.{}.c2lnbmF0dXJl", b64(&json!({"alg": "RS256"})), b64(&payload))
}

fn id_token(email: &str, account: &str) -> String {
    jwt(json!({"email": email, "https://api.openai.com/auth": {"chatgpt_account_id": account}}))
}

fn access_token(exp: i64) -> String {
    jwt(json!({"exp": exp, "sub": "user-123"}))
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn core() -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
    (dir, core)
}

fn codex_provider(core: &Core) -> Provider {
    let draft = Provider {
        id: 0,
        name: "Codex".into(),
        kind: ProviderKind::Codex,
        base_url: BASE_URL.into(),
        model: DEFAULT_MODEL.into(),
        json_mode: "json_schema".into(),
        is_default: false,
    };
    core.save_provider(&draft, None).unwrap()
}

fn schema() -> Value {
    json!({"type": "object", "properties": {"ok": {"type": "boolean"}}, "required": ["ok"]})
}

fn sse(events: &[Value]) -> String {
    events.iter().map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or(""))).collect()
}

fn sse_ok(text: &str) -> (u16, String) {
    let (a, b) = text.split_at(text.len() / 2);
    (
        200,
        sse(&[
            json!({"type": "response.created", "response": {"id": "r1"}}),
            json!({"type": "response.output_text.delta", "delta": a}),
            json!({"type": "response.output_text.delta", "delta": b}),
            json!({"type": "response.completed", "response": {"id": "r1", "output": []}}),
        ]),
    )
}

#[derive(Clone, Debug)]
struct Captured {
    path: String,
    headers: HashMap<String, String>,
    body: String,
}

type Requests = Arc<Mutex<Vec<Captured>>>;

/// Serve `responses` (status, body) in order, one per connection; the last one repeats.
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
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (head_end, len) = loop {
                let read = sock.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    break (buf.len(), 0);
                }
                buf.extend_from_slice(&chunk[..read]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..pos]).to_lowercase();
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse().unwrap_or(0)))
                        .unwrap_or(0);
                    break (pos + 4, len);
                }
            };
            while buf.len() < head_end + len {
                let read = sock.read(&mut chunk).await.unwrap_or(0);
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
            }
            let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
            let mut lines = head.lines();
            let path = lines.next().unwrap_or_default().split_whitespace().nth(1).unwrap_or_default().to_string();
            let headers = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                .collect();
            let body = String::from_utf8_lossy(&buf[head_end..]).to_string();
            log.lock().unwrap().push(Captured { path, headers, body });
            let kind = if reply_body.starts_with('{') { "application/json" } else { "text/event-stream" };
            let reply = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply_body}",
                reply_body.len()
            );
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        }
    });
    (format!("http://{addr}"), captured)
}

// ---------------------------------------------------------------- sign-in pieces

#[test]
fn builds_the_authorize_url() {
    let url = auth::authorize_url(auth::ISSUER, auth::REDIRECT_URI, "chal", "st&te").unwrap();
    assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"), "{url}");
    let q: HashMap<_, _> = url::Url::parse(&url).unwrap().query_pairs().into_owned().collect();
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], "app_EMoamEEZ73f0CkXaXp7hrann");
    assert_eq!(q["redirect_uri"], "http://localhost:1455/auth/callback");
    assert_eq!(q["scope"], "openid profile email offline_access");
    assert_eq!(q["code_challenge"], "chal");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["id_token_add_organizations"], "true");
    assert_eq!(q["codex_cli_simplified_flow"], "true");
    assert_eq!(q["state"], "st&te");
    assert_eq!(q["originator"], "codex_cli_rs");
    assert!(url.contains("redirect_uri=http%3A%2F%2Flocalhost%3A1455%2Fauth%2Fcallback"));
}

#[test]
fn decodes_id_token_claims() {
    let claims = IdClaims::decode(&id_token("ada@example.com", "acct-42"));
    assert_eq!(claims.email.as_deref(), Some("ada@example.com"));
    assert_eq!(claims.account_id.as_deref(), Some("acct-42"));
    assert_eq!(IdClaims::decode("not-a-jwt"), IdClaims::default());
    assert_eq!(IdClaims::decode("a.!!!.c"), IdClaims::default());
    assert_eq!(jwt_exp(&access_token(1_900_000_000)), Some(1_900_000_000));
    assert_eq!(jwt_exp("opaque-token"), None);
}

#[test]
fn token_blob_roundtrips_and_debug_hides_the_tokens() {
    let secrets = Secrets::memory();
    let access = access_token(now() + 3600);
    let t = Tokens::from_raw(&access, "refresh-token-abcdef", &id_token("ada@example.com", "acct-42"), None);
    assert_eq!(t.account_id, "acct-42");
    assert_eq!(t.email, "ada@example.com");
    assert!(t.expires_at.is_some() && !t.expires_soon(now()));
    // An explicit account id wins over the claim.
    assert_eq!(Tokens::from_raw(&access, "r", "", Some("acct-9")).account_id, "acct-9");

    assert_eq!(Login::load(&secrets, 7).unwrap(), None);
    Login::Own(t.clone()).save(&secrets, 7).unwrap();
    assert_eq!(Login::load(&secrets, 7).unwrap(), Some(Login::Own(t.clone())));
    let raw = secrets.get(&codex_tokens_secret(7)).unwrap().unwrap();
    assert!(raw.contains("refresh-token-abcdef"), "stored as one JSON blob");

    let dbg = format!("{t:?}");
    assert!(!dbg.contains(&access) && !dbg.contains("refresh-token") && !dbg.contains("acct-42"), "{dbg}");
    // Every token is scrubbed from logs.
    assert!(!crate::redact::redact(&format!("x {access} y refresh-token-abcdef")).contains("refresh-token-abcdef"));

    let soon = Tokens::from_raw(&access_token(now() + 60), "r-12345678", "", None);
    assert!(soon.expires_soon(now()));

    secrets.set(&codex_tokens_secret(8), "not json").unwrap();
    assert!(Login::load(&secrets, 8).is_err());
    secrets.set(&codex_tokens_secret(8), "{\"source\": \"cli\"}").unwrap();
    assert!(Login::load(&secrets, 8).is_err(), "a linked login names its file");
}

#[test]
fn imports_the_cli_login() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    assert!(!auth::cli_login_at(&path));
    assert!(auth::read_cli_login(&path).unwrap_err().to_string().contains("No Codex CLI login"));

    std::fs::write(&path, json!({"OPENAI_API_KEY": "sk-should-be-ignored-123456"}).to_string()).unwrap();
    assert!(!auth::cli_login_at(&path));
    assert_eq!(auth::read_cli_login(&path).unwrap_err().to_string(), "The Codex CLI is not signed in with ChatGPT");
    std::fs::write(&path, json!({"OPENAI_API_KEY": null, "tokens": null}).to_string()).unwrap();
    assert_eq!(auth::read_cli_login(&path).unwrap_err().to_string(), "The Codex CLI is not signed in with ChatGPT");

    let access = access_token(now() + 3600);
    let auth_json = json!({
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token("ada@example.com", "acct-from-claim"),
            "access_token": access,
            "refresh_token": "cli-refresh-token-1",
            "account_id": "acct-cli",
        },
        "last_refresh": "2026-11-01T10:00:00Z",
    });
    std::fs::write(&path, auth_json.to_string()).unwrap();
    assert!(auth::cli_login_at(&path));
    let t = auth::read_cli_login(&path).unwrap();
    assert_eq!(t.access_token, access);
    assert_eq!(t.refresh_token, "cli-refresh-token-1");
    assert_eq!(t.account_id, "acct-cli");
    assert_eq!(t.email, "ada@example.com");

    let (_d, core) = core();
    let p = codex_provider(&core);
    assert_eq!(core.codex_account(p.id).unwrap(), None);
    assert_eq!(core.codex_link(p.id, path.clone()).unwrap(), "ada@example.com");
    assert_eq!(core.codex_account(p.id).unwrap().as_deref(), Some("ada@example.com"));
    // Linking does not touch the CLI's file...
    assert_eq!(std::fs::read_to_string(&path).unwrap(), auth_json.to_string());
    // ...and the keyring holds where the login is, not a copy of its tokens.
    let blob: Value = serde_json::from_str(&core.secrets.get(&codex_tokens_secret(p.id)).unwrap().unwrap()).unwrap();
    assert_eq!(blob, json!({"source": "cli", "path": path, "email": "ada@example.com"}));
    assert_eq!(Login::load(&core.secrets, p.id).unwrap(), Some(Login::Linked { path: path.clone(), email: "ada@example.com".into() }));

    // The account follows the file; once it is gone, the email saved with the link.
    let mut other = auth_json.clone();
    other["tokens"]["id_token"] = json!(id_token("grace@example.com", "acct-cli"));
    std::fs::write(&path, other.to_string()).unwrap();
    assert_eq!(core.codex_account(p.id).unwrap().as_deref(), Some("grace@example.com"));
    std::fs::remove_file(&path).unwrap();
    assert_eq!(core.codex_account(p.id).unwrap().as_deref(), Some("ada@example.com"));
}

#[test]
fn sign_in_needs_a_saved_codex_provider() {
    let (_d, core) = core();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    assert_eq!(core.codex_link(0, path.clone()).unwrap_err().to_string(), "Save the provider, then sign in.");
    assert_eq!(core.codex_import_cli_login(0).unwrap_err().to_string(), "Save the provider, then sign in.");
    assert!(core.codex_link(99, path.clone()).unwrap_err().to_string().contains("no longer exists"));
    let mut other = codex_provider(&core);
    other.id = 0;
    other.kind = ProviderKind::OpenAi;
    other.name = "OpenAI".into();
    let other = core.save_provider(&other, None).unwrap();
    assert_eq!(
        core.codex_link(other.id, path).unwrap_err().to_string(),
        "Only a Codex provider signs in with ChatGPT."
    );
}

// ---------------------------------------------------------------- request + SSE

#[test]
fn builds_the_request_body() {
    let body = build_request("gpt-5-codex", "SYS", "USER", &schema());
    assert_eq!(
        body,
        json!({
            "model": "gpt-5-codex",
            "instructions": "SYS",
            "input": [{"type": "message", "role": "user", "content": [{"type": "input_text", "text": "USER"}]}],
            "store": false,
            "stream": true,
            "text": {"format": {"type": "json_schema", "name": "response", "schema": schema(), "strict": false}},
        })
    );
}

#[test]
fn parses_the_event_stream() {
    assert_eq!(parse_sse(&sse_ok("{\"ok\": true}").1).unwrap(), "{\"ok\": true}");

    let completed_only = sse(&[json!({"type": "response.completed", "response": {"output": [
        {"type": "reasoning", "summary": []},
        {"type": "message", "content": [{"type": "output_text", "text": "{\"ok\":"}, {"type": "output_text", "text": " true}"}]},
    ]}})]);
    assert_eq!(parse_sse(&completed_only).unwrap(), "{\"ok\": true}");

    let failed = sse(&[json!({"type": "response.failed", "response": {"error": {"code": "server_error", "message": "boom"}}})]);
    assert_eq!(parse_sse(&failed).unwrap_err().to_string(), "the request failed: boom");
    let error = sse(&[json!({"type": "error", "code": "rate_limit_exceeded", "message": "slow down"})]);
    assert_eq!(parse_sse(&error).unwrap_err().to_string(), "the request failed: slow down");
    let cut = sse(&[json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}})]);
    assert!(parse_sse(&cut).unwrap_err().to_string().contains("max_output_tokens"));
    let early = "the answer stream ended early (connection closed)";
    assert_eq!(parse_sse("data: [DONE]\n").unwrap_err().to_string(), early);
    // Deltas without `response.completed` are a partial answer: an error, never the text.
    let partial = sse(&[json!({"type": "response.output_text.delta", "delta": "{\"ok\": tr"})]);
    assert_eq!(parse_sse(&partial).unwrap_err().to_string(), early);
    // CRLF lines, comments and unknown events are fine.
    let crlf = ": keepalive\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"x\"}\r\n\r\n\
                data: {\"type\":\"response.completed\",\"response\":{}}\r\n\r\n";
    assert_eq!(parse_sse(crlf).unwrap(), "x");
}

// ---------------------------------------------------------------- against a fake backend

fn signed_in(core: &Core, provider_id: i64, access: &str) -> Tokens {
    let t = Tokens::from_raw(access, "refresh-token-old-1", &id_token("ada@example.com", "acct-42"), None);
    Login::Own(t.clone()).save(&core.secrets, provider_id).unwrap();
    t
}

fn own_tokens(core: &Core, provider_id: i64) -> Tokens {
    match Login::load(&core.secrets, provider_id).unwrap() {
        Some(Login::Own(t)) => t,
        other => panic!("not an own sign-in: {other:?}"),
    }
}

#[tokio::test]
async fn complete_json_streams_the_answer() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let access = access_token(now() + 3600);
    signed_in(&core, p.id, &access);
    let (base, seen) = serve(vec![sse_ok("```json\n{\"ok\": true}\n```")]).await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&format!("{base}/"), &base);
    assert_eq!(client.complete_json("SYS", "USER", &schema()).await.unwrap(), json!({"ok": true}));
    assert_eq!(client.list_models().await.unwrap(), ["gpt-5-codex", "gpt-5", "gpt-5.1-codex", "gpt-5.1"]);

    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!(reqs[0].path, "/responses");
    let h = &reqs[0].headers;
    assert_eq!(h["authorization"], format!("Bearer {access}"));
    assert_eq!(h["chatgpt-account-id"], "acct-42");
    assert_eq!(h["openai-beta"], "responses=experimental");
    assert_eq!(h["originator"], "codex_cli_rs");
    assert_eq!(h["accept"], "text/event-stream");
    let body: Value = serde_json::from_str(&reqs[0].body).unwrap();
    assert_eq!(body["model"], "gpt-5-codex");
    assert_eq!(body["instructions"], "SYS");
    assert_eq!(body["input"][0]["content"][0]["text"], "USER");
}

#[tokio::test]
async fn refreshes_after_a_401_and_saves_the_new_tokens() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    signed_in(&core, p.id, "old-access-token-1");
    let new_access = access_token(now() + 3600);
    let (base, seen) = serve(vec![
        (401, json!({"error": {"message": "token expired"}}).to_string()),
        (200, json!({"access_token": new_access, "refresh_token": "refresh-token-new-2"}).to_string()),
        sse_ok("{\"ok\": true}"),
    ])
    .await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    assert_eq!(client.complete_json("S", "U", &schema()).await.unwrap(), json!({"ok": true}));

    let reqs = seen.lock().unwrap();
    assert_eq!(reqs.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), ["/responses", "/oauth/token", "/responses"]);
    let refresh: Value = serde_json::from_str(&reqs[1].body).unwrap();
    assert_eq!(
        refresh,
        json!({"client_id": "app_EMoamEEZ73f0CkXaXp7hrann", "grant_type": "refresh_token",
               "refresh_token": "refresh-token-old-1", "scope": "openid profile email"})
    );
    assert_eq!(reqs[2].headers["authorization"], format!("Bearer {new_access}"));

    let saved = own_tokens(&core, p.id);
    assert_eq!(saved.access_token, new_access);
    assert_eq!(saved.refresh_token, "refresh-token-new-2");
    assert_eq!(saved.account_id, "acct-42", "kept when the refresh sends no id token");
    assert_eq!(saved.email, "ada@example.com");
}

#[tokio::test]
async fn refreshes_before_the_token_expires() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    signed_in(&core, p.id, &access_token(now() + 30));
    let new_access = access_token(now() + 3600);
    let (base, seen) = serve(vec![
        (200, json!({"access_token": new_access, "id_token": id_token("new@example.com", "acct-42")}).to_string()),
        sse_ok("{\"ok\": true}"),
    ])
    .await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    client.complete_json("S", "U", &schema()).await.unwrap();
    let reqs = seen.lock().unwrap();
    assert_eq!(reqs[0].path, "/oauth/token");
    assert_eq!(reqs[1].headers["authorization"], format!("Bearer {new_access}"));
    let saved = own_tokens(&core, p.id);
    assert_eq!(saved.refresh_token, "refresh-token-old-1", "kept when none is sent");
    assert_eq!(core.codex_account(p.id).unwrap().as_deref(), Some("new@example.com"));
}

#[tokio::test]
async fn a_refused_refresh_asks_to_sign_in_again_without_leaking_tokens() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    signed_in(&core, p.id, "old-access-token-1");
    let (base, _seen) = serve(vec![
        (401, json!({"error": {"message": "bad token old-access-token-1"}}).to_string()),
        (400, json!({"error": "invalid_grant", "error_description": "refresh-token-old-1 was reused"}).to_string()),
    ])
    .await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    let err = client.complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert!(err.starts_with("Codex: the ChatGPT sign-in is no longer valid"), "{err}");
    assert!(err.contains("Sign in with ChatGPT"), "{err}");
    assert!(!err.contains("old-access-token-1") && !err.contains("refresh-token-old-1"), "{err}");
}

#[tokio::test]
async fn http_errors_are_readable_and_redacted() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let access = access_token(now() + 3600);
    signed_in(&core, p.id, &access);
    let (base, _seen) =
        serve(vec![(400, json!({"error": {"message": format!("model unsupported for {access}")}}).to_string())]).await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    let err = client.complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert!(err.starts_with("Codex: HTTP 400 Bad Request: model unsupported for"), "{err}");
    assert!(!err.contains(&access), "{err}");
}

#[tokio::test]
async fn not_signed_in_says_what_to_do() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let want = "Codex: not signed in: press Sign in with ChatGPT in Settings > AI providers";
    let err = core.llm_for(p.id).unwrap().complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert_eq!(err, want);
    let err = core.llm_for_form(&p, None).complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert_eq!(err, want);
    // An unsaved form has no sign-in either.
    let unsaved = Provider { id: 0, ..p };
    let err = core.llm_for_form(&unsaved, None).complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert_eq!(err, want);
}

#[tokio::test]
async fn exchanges_the_code_for_tokens() {
    let (base, seen) = serve(vec![(
        200,
        json!({"access_token": access_token(now() + 3600), "refresh_token": "refresh-token-x1",
               "id_token": id_token("ada@example.com", "acct-42")})
        .to_string(),
    )])
    .await;
    let http = reqwest::Client::new();
    let t = auth::exchange_code(&http, &base, "the-code", auth::REDIRECT_URI, "the-verifier").await.unwrap();
    assert_eq!((t.email.as_str(), t.account_id.as_str()), ("ada@example.com", "acct-42"));
    let req = seen.lock().unwrap()[0].clone();
    let reqs = [req];
    assert_eq!(reqs[0].path, "/oauth/token");
    assert!(reqs[0].headers["content-type"].starts_with("application/x-www-form-urlencoded"));
    let form: HashMap<_, _> = url::form_urlencoded::parse(reqs[0].body.as_bytes()).into_owned().collect();
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["code"], "the-code");
    assert_eq!(form["redirect_uri"], "http://localhost:1455/auth/callback");
    assert_eq!(form["client_id"], "app_EMoamEEZ73f0CkXaXp7hrann");
    assert_eq!(form["code_verifier"], "the-verifier");

    let (base, _) = serve(vec![(200, json!({"id_token": "x"}).to_string())]).await;
    assert!(auth::exchange_code(&http, &base, "c", "r", "v").await.unwrap_err().to_string().contains("no access"));
}

#[tokio::test]
async fn browser_sign_in_end_to_end() {
    let (issuer, seen) = serve(vec![(
        200,
        json!({"access_token": access_token(now() + 3600), "refresh_token": "refresh-token-x1",
               "id_token": id_token("ada@example.com", "acct-42")})
        .to_string(),
    )])
    .await;
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let redirect = format!("http://127.0.0.1:{port}/auth/callback");
    let opened = Arc::new(Mutex::new(String::new()));
    let seen_url = opened.clone();
    let open = move |url: &str| -> anyhow::Result<()> {
        *seen_url.lock().unwrap() = url.to_string();
        let parsed = url::Url::parse(url)?;
        let state = parsed.query_pairs().find(|(k, _)| k == "state").map(|(_, v)| v.into_owned()).unwrap_or_default();
        // The "browser": a favicon request, then the issuer's redirect.
        tokio::spawn(async move {
            for target in ["/favicon.ico".to_string(), format!("/auth/callback?code=code-1&state={state}")] {
                let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                s.write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes()).await.unwrap();
                let mut out = String::new();
                s.read_to_string(&mut out).await.unwrap();
            }
        });
        Ok(())
    };
    let http = reqwest::Client::new();
    let t = tokio::time::timeout(Duration::from_secs(10), auth::sign_in(&http, &listener, &issuer, &redirect, open))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(t.email, "ada@example.com");
    assert!(opened.lock().unwrap().starts_with(&format!("{issuer}/oauth/authorize?")));
    let form: HashMap<_, _> = url::form_urlencoded::parse(seen.lock().unwrap()[0].body.as_bytes()).into_owned().collect();
    assert_eq!(form["code"], "code-1");
    assert_eq!(form["redirect_uri"], redirect);
}

// ---------------------------------------------------------------- a linked Codex CLI login

/// A Codex CLI `auth.json` in a temp CODEX_HOME, with fields JobPilot must keep.
fn cli_home(access: &str) -> (tempfile::TempDir, std::path::PathBuf, Value) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("auth.json");
    let auth_json = json!({
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": id_token("ada@example.com", "acct-cli"),
            "access_token": access,
            "refresh_token": "cli-refresh-token-1",
            "account_id": "acct-cli",
            "future_field": "kept",
        },
        "last_refresh": "2026-11-01T10:00:00Z",
        "agent_identity": {"kept": true},
    });
    std::fs::write(&path, auth_json.to_string()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    (dir, path, auth_json)
}

#[tokio::test]
async fn a_linked_login_reads_the_cli_file_on_every_call() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let first = access_token(now() + 3600);
    let (_home, path, mut auth_json) = cli_home(&first);
    core.codex_link(p.id, path.clone()).unwrap();
    let (base, seen) = serve(vec![sse_ok("{\"ok\": true}")]).await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    client.complete_json("S", "U", &schema()).await.unwrap();

    // The CLI refreshed meanwhile: the next call uses its new token.
    let second = access_token(now() + 7200);
    auth_json["tokens"]["access_token"] = json!(second);
    std::fs::write(&path, auth_json.to_string()).unwrap();
    client.complete_json("S", "U", &schema()).await.unwrap();
    {
        let reqs = seen.lock().unwrap();
        assert_eq!(reqs[0].headers["authorization"], format!("Bearer {first}"));
        assert_eq!(reqs[1].headers["authorization"], format!("Bearer {second}"));
        assert_eq!(reqs[1].headers["chatgpt-account-id"], "acct-cli");
    }
    assert!(core.secrets.get(&codex_tokens_secret(p.id)).unwrap().is_some_and(|b| !b.contains(&second)));

    // The CLI logged out: sign in again.
    std::fs::remove_file(&path).unwrap();
    let err = client.complete_json("S", "U", &schema()).await.unwrap_err().to_string();
    assert_eq!(err, format!("Codex: {CLI_LOGIN_GONE}"));
    assert_eq!(crate::health::classify(&err), crate::health::Problem::ProviderSignIn);
}

#[tokio::test]
async fn a_linked_login_writes_a_refresh_back_into_the_cli_file() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let (_home, path, before) = cli_home("old-access-token-1");
    core.codex_link(p.id, path.clone()).unwrap();
    let new_access = access_token(now() + 3600);
    let (base, seen) = serve(vec![
        (401, json!({"error": {"message": "token expired"}}).to_string()),
        (200, json!({"access_token": new_access, "refresh_token": "cli-refresh-token-2"}).to_string()),
        sse_ok("{\"ok\": true}"),
    ])
    .await;
    let client = CodexClient::new(&p, core.secrets.clone()).with_urls(&base, &base);
    assert_eq!(client.complete_json("S", "U", &schema()).await.unwrap(), json!({"ok": true}));
    {
        let reqs = seen.lock().unwrap();
        let refresh: Value = serde_json::from_str(&reqs[1].body).unwrap();
        assert_eq!(refresh["refresh_token"], "cli-refresh-token-1", "the CLI's refresh token is spent");
        assert_eq!(reqs[2].headers["authorization"], format!("Bearer {new_access}"));
    }

    // The new tokens are in the CLI's file, every other field is as it was.
    let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(after["tokens"]["access_token"], new_access.as_str());
    assert_eq!(after["tokens"]["refresh_token"], "cli-refresh-token-2");
    assert_eq!(after["tokens"]["id_token"], before["tokens"]["id_token"]);
    assert_eq!(after["tokens"]["account_id"], "acct-cli");
    assert_eq!(after["tokens"]["future_field"], "kept");
    assert_eq!(after["agent_identity"], json!({"kept": true}));
    assert_eq!(after["OPENAI_API_KEY"], Value::Null);
    assert_ne!(after["last_refresh"], before["last_refresh"]);
    assert!(chrono::DateTime::parse_from_rfc3339(after["last_refresh"].as_str().unwrap()).is_ok());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // No temp file is left behind, and the keyring still only links to the file.
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names, [std::ffi::OsString::from("auth.json")]);
    assert!(matches!(Login::load(&core.secrets, p.id).unwrap(), Some(Login::Linked { .. })));
}

#[tokio::test]
async fn a_linked_login_uses_a_refresh_the_cli_made_meanwhile() {
    let (_d, core) = core();
    let p = codex_provider(&core);
    let (_home, path, mut auth_json) = cli_home("old-access-token-1");
    core.codex_link(p.id, path.clone()).unwrap();
    let client = CodexClient::new(&p, core.secrets.clone());
    let stale = auth::read_cli_login(&path).unwrap();
    // The CLI refreshed after this call read the file: no second refresh with a spent token.
    let cli_access = access_token(now() + 3600);
    auth_json["tokens"]["access_token"] = json!(cli_access);
    auth_json["tokens"]["refresh_token"] = json!("cli-refresh-token-2");
    std::fs::write(&path, auth_json.to_string()).unwrap();
    let fresh = client.refresh(&stale).await.unwrap();
    assert_eq!(fresh.access_token, cli_access);
    assert_eq!(fresh.refresh_token, "cli-refresh-token-2");
}
