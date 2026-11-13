//! The HTTP side of the API adapters: one small trait, so tests answer from saved fixture JSON
//! and never touch the network.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::discover::limits::HostGate;

/// Largest response an adapter reads (a "Who is hiring" thread is several MB of JSON).
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("jobpilot/", env!("CARGO_PKG_VERSION"));

/// GET a URL and parse the JSON answer.
#[async_trait]
pub trait Http: Send + Sync {
    /// `Err` for a transport failure, a non-2xx status (the text names the status, so it is
    /// classified like every other network error) or a body that is not JSON.
    async fn get_json(&self, url: &str) -> Result<Value>;
}

/// The real client: `reqwest`, a timeout, a size cap and the per-host delay.
pub struct ReqwestHttp {
    client: reqwest::Client,
    gate: Arc<HostGate>,
}

impl ReqwestHttp {
    pub fn new(gate: Arc<HostGate>) -> Self {
        let client = reqwest::Client::builder().timeout(TIMEOUT).user_agent(USER_AGENT).build().unwrap_or_default();
        Self { client, gate }
    }
}

#[async_trait]
impl Http for ReqwestHttp {
    async fn get_json(&self, url: &str) -> Result<Value> {
        let parsed = url::Url::parse(url).with_context(|| format!("`{url}` is not a valid URL"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            bail!("only http(s) sources are supported, got `{url}`");
        }
        let host = parsed.host_str().unwrap_or_default().to_string();
        self.gate.wait(&host).await;
        let resp = self
            .client
            .get(parsed)
            .header("Accept", "application/json")
            .send()
            .await
            .with_context(|| format!("fetching {host}"))?;
        let status = resp.status();
        if !status.is_success() {
            bail!("fetching {host}: HTTP {} {}", status.as_u16(), status.canonical_reason().unwrap_or("error"));
        }
        let bytes = resp.bytes().await.with_context(|| format!("reading the answer of {host}"))?;
        if bytes.len() > MAX_BODY_BYTES {
            bail!("the answer of {host} is larger than {} MB", MAX_BODY_BYTES / 1024 / 1024);
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("{host} did not return JSON (HTTP {})", status.as_u16()))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    /// Answers from a URL -> JSON map and records every request.
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        answers: HashMap<String, Value>,
        requested: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub(crate) fn with(answers: impl IntoIterator<Item = (&'static str, &'static str)>) -> Arc<Self> {
            let answers = answers
                .into_iter()
                .map(|(url, body)| (url.to_string(), serde_json::from_str(body).expect("fixture is valid JSON")))
                .collect();
            Arc::new(Self { answers, requested: Mutex::default() })
        }

        pub(crate) fn requested(&self) -> Vec<String> {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    #[async_trait]
    impl Http for FakeHttp {
        async fn get_json(&self, url: &str) -> Result<Value> {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).push(url.to_string());
            self.answers.get(url).cloned().ok_or_else(|| anyhow!("fetching fixture: HTTP 404 Not Found ({url})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    /// Answer each connection with the next `(status line, body)`; returns the base URL.
    async fn serve(answers: Vec<(&'static str, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            for (status, body) in answers {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn json_comes_back_and_failures_name_their_status() {
        let base = serve(vec![("200 OK", r#"{"jobs": []}"#), ("503 Service Unavailable", "busy"), ("200 OK", "<html>"), ("404 Not Found", "")]).await;
        let http = ReqwestHttp::new(Arc::new(HostGate::new(Duration::ZERO)));
        assert_eq!(http.get_json(&format!("{base}/a")).await.unwrap(), serde_json::json!({"jobs": []}));
        let unavailable = format!("{:#}", http.get_json(&format!("{base}/b")).await.unwrap_err());
        assert!(unavailable.contains("HTTP 503"), "{unavailable}");
        assert_eq!(crate::health::classify(&unavailable), crate::health::Problem::Network, "so the UI offers Retry");
        let html = format!("{:#}", http.get_json(&format!("{base}/c")).await.unwrap_err());
        assert!(html.contains("did not return JSON"), "{html}");
        let missing = format!("{:#}", http.get_json(&format!("{base}/d")).await.unwrap_err());
        assert!(missing.contains("HTTP 404") && crate::health::classify(&missing) == crate::health::Problem::Other, "{missing}");
    }

    #[tokio::test]
    async fn only_http_urls_are_fetched_and_a_dead_host_is_a_network_error() {
        let http = ReqwestHttp::new(Arc::new(HostGate::new(Duration::ZERO)));
        assert!(http.get_json("file:///etc/passwd").await.unwrap_err().to_string().contains("only http(s)"));
        assert!(http.get_json("not a url").await.is_err());
        // Nothing listens on this port: the connection is refused at once.
        let err = format!("{:#}", http.get_json("http://127.0.0.1:1/x").await.unwrap_err());
        assert_eq!(crate::health::classify(&err), crate::health::Problem::Network, "{err}");
    }
}
