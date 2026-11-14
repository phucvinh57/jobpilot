//! The HTTP side of the API adapters: one small trait, so tests answer from saved fixture JSON
//! and never touch the network. The real implementation sits on the shared safe client
//! ([`crate::engine::net::http`]): public hosts only, hop-by-hop redirects, a size cap while streaming and
//! the per-host delay.

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use serde_json::Value;

use crate::engine::net::http::{checked_url, Http as Client};

/// GET a URL and parse the JSON answer.
#[async_trait]
pub trait Http: Send + Sync {
    /// `Err` for a transport failure, a non-2xx status (the text names the status, so it is
    /// classified like every other network error) or a body that is not JSON.
    async fn get_json(&self, url: &str) -> Result<Value>;

    /// GET a URL and return the body as text (an XML feed). `Err` as for [`Http::get_json`].
    async fn get_text(&self, url: &str, accept: &str) -> Result<String>;
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

    async fn get_text(&self, url: &str, accept: &str) -> Result<String> {
        let parsed = checked_url(url)?;
        let host = parsed.host_str().unwrap_or_default().to_string();
        let resp = self.client.get(url, &[("Accept", accept)]).await.with_context(|| format!("fetching {host}"))?;
        Ok(resp.text())
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// Answers from a URL -> body map (JSON, or the text of an XML feed), fails the URLs given a
    /// status, and records every request.
    #[derive(Default)]
    pub(crate) struct FakeHttp {
        answers: HashMap<String, String>,
        statuses: HashMap<String, u16>,
        requested: Mutex<Vec<String>>,
    }

    impl FakeHttp {
        pub(crate) fn with(answers: impl IntoIterator<Item = (&'static str, &'static str)>) -> Arc<Self> {
            Self::with_statuses(answers, [])
        }

        /// [`FakeHttp::with`], plus URLs that answer with a non-2xx `status` (`HTTP 503`).
        pub(crate) fn with_statuses(
            answers: impl IntoIterator<Item = (&'static str, &'static str)>,
            statuses: impl IntoIterator<Item = (&'static str, u16)>,
        ) -> Arc<Self> {
            let answers = answers.into_iter().map(|(url, body)| (url.to_string(), body.to_string())).collect();
            let statuses = statuses.into_iter().map(|(url, status)| (url.to_string(), status)).collect();
            Arc::new(Self { answers, statuses, requested: Mutex::default() })
        }

        fn answer(&self, url: &str) -> Result<&str> {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).push(url.to_string());
            if let Some(status) = self.statuses.get(url) {
                return Err(anyhow!("fetching fixture: HTTP {status}"));
            }
            self.answers.get(url).map(String::as_str).ok_or_else(|| anyhow!("fetching fixture: HTTP 404 Not Found ({url})"))
        }

        pub(crate) fn requested(&self) -> Vec<String> {
            self.requested.lock().unwrap_or_else(|e| e.into_inner()).clone()
        }
    }

    #[async_trait]
    impl Http for FakeHttp {
        async fn get_json(&self, url: &str) -> Result<Value> {
            serde_json::from_str(self.answer(url)?).map_err(|_| anyhow!("fixture did not return JSON"))
        }

        async fn get_text(&self, url: &str, _accept: &str) -> Result<String> {
            self.answer(url).map(str::to_string)
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
    async fn text_comes_back_with_the_accept_header_asked_for() {
        let fx = Fixtures::start().await;
        fx.route("/feed.xml", 200, "application/xml", "<jobs/>").route("/busy", 503, "text/plain", "busy");
        let (http, base) = (local(1 << 20), fx.base());
        assert_eq!(http.get_text(&format!("{base}/feed.xml"), "application/xml").await.unwrap(), "<jobs/>");
        assert_eq!(fx.hits_on("/feed.xml")[0].header("accept"), Some("application/xml"));
        let busy = format!("{:#}", http.get_text(&format!("{base}/busy"), "application/xml").await.unwrap_err());
        assert_eq!(crate::engine::health::classify(&busy), crate::engine::health::Problem::Network, "{busy}");
        assert!(http.get_text("file:///etc/passwd", "text/xml").await.is_err());
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
