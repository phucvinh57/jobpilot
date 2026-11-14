//! The HTTP side of the API adapters: one small trait, so tests answer from saved fixture JSON
//! and never touch the network. The real implementation sits on the shared safe client
//! ([`crate::engine::net::http`]): public hosts only, hop-by-hop redirects, a size cap while streaming and
//! the per-host delay.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::engine::net::http::{checked_url, Http as Client};

/// GET (or POST to) a URL and parse the JSON answer.
#[async_trait]
pub trait Http: Send + Sync {
    /// `Err` for a transport failure, a non-2xx status (the text names the status, so it is
    /// classified like every other network error) or a body that is not JSON.
    async fn get_json(&self, url: &str) -> Result<Value>;

    /// `POST url` with a JSON `body`, failing like [`Http::get_json`]. Never redirected: the body
    /// is not re-sent anywhere else. Only Workday's job search needs it.
    async fn post_json(&self, url: &str, body: &Value) -> Result<Value>;
}

/// The real client: the shared [`Client`] asking for JSON.
pub struct NetHttp {
    client: Client,
}

impl NetHttp {
    pub fn new(client: Client) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Http for NetHttp {
    async fn get_json(&self, url: &str) -> Result<Value> {
        let parsed = checked_url(url)?;
        let host = parsed.host_str().unwrap_or_default().to_string();
        let resp = self.client.get(url, &[("Accept", "application/json")]).await.with_context(|| format!("fetching {host}"))?;
        serde_json::from_slice(&resp.body).map_err(|_| anyhow!("{host} did not return JSON"))
    }

    async fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
        let parsed = checked_url(url)?;
        let host = parsed.host_str().unwrap_or_default().to_string();
        let resp = self.client.post_json(url, &[("Accept", "application/json")], body).await.with_context(|| format!("asking {host}"))?;
        serde_json::from_slice(&resp.body).map_err(|_| anyhow!("{host} did not return JSON"))
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Answers GETs from a URL -> JSON map and POSTs from a (URL, body) -> JSON list, and records
    /// every request (a POST also with its body).
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        answers: HashMap<String, Value>,
        post_answers: Vec<(String, Value, Value)>,
        requested: Mutex<Vec<String>>,
        posted: Mutex<Vec<(String, Value)>>,
    }

    impl FakeHttp {
        pub(crate) fn with(answers: impl IntoIterator<Item = (&'static str, &'static str)>) -> Arc<Self> {
            let answers = answers
                .into_iter()
                .map(|(url, body)| (url.to_string(), serde_json::from_str(body).expect("fixture is valid JSON")))
                .collect();
            Arc::new(Self { answers, ..Self::default() })
        }

        /// [`FakeHttp::with`] plus POST answers: `(url, request body, answer)`.
        pub(crate) fn with_posts(
            answers: impl IntoIterator<Item = (&'static str, &'static str)>,
            posts: impl IntoIterator<Item = (&'static str, Value, &'static str)>,
        ) -> Arc<Self> {
            let mut fake = Arc::into_inner(Self::with(answers)).expect("a new fake has one owner");
            fake.post_answers = posts
                .into_iter()
                .map(|(url, body, answer)| (url.to_string(), body, serde_json::from_str(answer).expect("fixture is valid JSON")))
                .collect();
            Arc::new(fake)
        }

        /// Every POST so far: its URL and body.
        pub(crate) fn posted(&self) -> Vec<(String, Value)> {
            self.posted.lock().unwrap_or_else(|e| e.into_inner()).clone()
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

        async fn post_json(&self, url: &str, body: &Value) -> Result<Value> {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).push(url.to_string());
            self.posted.lock().unwrap_or_else(|e| e.into_inner()).push((url.to_string(), body.clone()));
            self.post_answers
                .iter()
                .find(|(u, b, _)| u == url && b == body)
                .map(|(_, _, answer)| answer.clone())
                .ok_or_else(|| anyhow!("asking fixture: HTTP 404 Not Found ({url})"))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::net::fixtures::Fixtures;
    use crate::engine::net::http::HttpLimits;

    use super::*;

    fn local(max_bytes: usize) -> NetHttp {
        NetHttp::new(Client::new(&HttpLimits { host_delay: std::time::Duration::ZERO, max_bytes, allow_private_hosts: true, ..HttpLimits::default() }).unwrap())
    }

    #[tokio::test]
    async fn json_comes_back_and_failures_name_their_status() {
        let fx = Fixtures::start().await;
        fx.route("/a", 200, "application/json", r#"{"jobs": []}"#).route("/b", 503, "text/plain", "busy").route("/c", 200, "text/html", "<html>");
        let (http, base) = (local(1 << 20), fx.base());
        assert_eq!(http.get_json(&format!("{base}/a")).await.unwrap(), serde_json::json!({"jobs": []}));
        assert_eq!(fx.hits_on("/a")[0].header("accept"), Some("application/json"));
        let unavailable = format!("{:#}", http.get_json(&format!("{base}/b")).await.unwrap_err());
        assert!(unavailable.contains("HTTP 503"), "{unavailable}");
        assert_eq!(crate::engine::health::classify(&unavailable), crate::engine::health::Problem::Network, "so the UI offers Retry");
        let html = format!("{:#}", http.get_json(&format!("{base}/c")).await.unwrap_err());
        assert!(html.contains("did not return JSON"), "{html}");
        let missing = format!("{:#}", http.get_json(&format!("{base}/nope")).await.unwrap_err());
        assert!(missing.contains("HTTP 404") && crate::engine::health::classify(&missing) == crate::engine::health::Problem::Other, "{missing}");
    }

    #[tokio::test]
    async fn a_post_sends_its_body_and_fails_like_a_get() {
        let fx = Fixtures::start().await;
        fx.route("/search", 200, "application/json", r#"{"total": 0}"#).route("/busy", 503, "text/plain", "busy");
        fx.redirect("/moved", 307, "/search");
        let (http, base) = (local(1 << 20), fx.base());
        let body = serde_json::json!({"searchText": "rust"});
        assert_eq!(http.post_json(&format!("{base}/search"), &body).await.unwrap(), serde_json::json!({"total": 0}));
        let hit = &fx.hits_on("/search")[0];
        assert_eq!((hit.method.as_str(), hit.header("accept")), ("POST", Some("application/json")));
        assert_eq!(serde_json::from_str::<Value>(&hit.body).unwrap(), body);
        let busy = format!("{:#}", http.post_json(&format!("{base}/busy"), &body).await.unwrap_err());
        assert!(busy.contains("HTTP 503"), "{busy}");
        assert_eq!(crate::engine::health::classify(&busy), crate::engine::health::Problem::Network, "so the UI offers Retry");
        assert!(http.post_json(&format!("{base}/moved"), &body).await.is_err(), "a POST is never redirected");
        assert_eq!(fx.hits_on("/search").len(), 1);
    }

    #[tokio::test]
    async fn only_http_urls_are_fetched_and_a_dead_host_is_a_network_error() {
        let http = local(1 << 20);
        assert!(http.get_json("file:///etc/passwd").await.unwrap_err().to_string().contains("only http(s)"));
        assert!(http.get_json("not a url").await.is_err());
        // Nothing listens on this port: the connection is refused at once.
        let err = format!("{:#}", http.get_json("http://127.0.0.1:1/x").await.unwrap_err());
        assert_eq!(crate::engine::health::classify(&err), crate::engine::health::Problem::Network, "{err}");
    }

    #[tokio::test]
    async fn a_private_host_and_an_oversized_answer_are_refused() {
        let fx = Fixtures::start().await;
        fx.route("/big", 200, "application/json", format!("[{}]", "1,".repeat(2000) + "1")).route("/ok", 200, "application/json", "[1]");
        // The fixture is on 127.0.0.1: the default (strict) client must not ask it.
        let strict = NetHttp::new(Client::new(&HttpLimits::default()).unwrap());
        let err = format!("{:#}", strict.get_json(&format!("{}/ok", fx.base())).await.unwrap_err());
        assert!(err.contains("private network"), "{err}");
        assert!(fx.hits().is_empty(), "nothing was requested");

        let capped = local(1000);
        let err = format!("{:#}", capped.get_json(&format!("{}/big", fx.base())).await.unwrap_err());
        assert!(err.contains("larger than"), "{err}");
        assert_eq!(capped.get_json(&format!("{}/ok", fx.base())).await.unwrap(), serde_json::json!([1]));
    }
}
