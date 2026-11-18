//! Test helpers of the trend adapters: the local fixture server ([`crate::net::fixtures`]), a PDF
//! maker and an adapter context pointed at the fixtures.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::config::{CollectLimits, Endpoints, TrendConfig};
use super::http::Http;
use super::period::Period;
use super::source::Ctx;

pub use crate::net::fixtures::Fixtures;

/// A PDF made from Typst markup (the `typst` CLI is a prerequisite of the project).
pub fn typst_pdf(markup: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("temp dir");
    let (typ, pdf) = (dir.path().join("r.typ"), dir.path().join("r.pdf"));
    std::fs::write(&typ, markup).expect("write markup");
    let status = std::process::Command::new(crate::pipeline::render::typst_binary())
        .arg("compile")
        .arg(&typ)
        .arg(&pdf)
        .status()
        .expect("typst is needed for tests");
    assert!(status.success(), "typst failed");
    std::fs::read(&pdf).expect("read pdf")
}

/// A local server for a paged API (Algolia's `page` query parameter, 0-based): `pages[n]` answers
/// `?page=n` on any path, a page past the list answers 404, and every request target is recorded.
/// The fixture server routes by path alone, so it cannot serve several pages of one path.
pub struct Paged {
    base: String,
    targets: Arc<Mutex<Vec<String>>>,
}

impl Paged {
    pub async fn start(pages: Vec<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind a local port");
        let base = format!("http://{}", listener.local_addr().expect("local address"));
        let targets: Arc<Mutex<Vec<String>>> = Arc::default();
        let (pages, seen) = (Arc::new(pages), targets.clone());
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let (pages, seen) = (pages.clone(), seen.clone());
                tokio::spawn(async move {
                    let mut raw = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !raw.windows(4).any(|w| w == b"\r\n\r\n") {
                        match sock.read(&mut buf).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => raw.extend_from_slice(&buf[..n]),
                        }
                    }
                    let head = String::from_utf8_lossy(&raw).into_owned();
                    let target = head.lines().next().unwrap_or_default().split_whitespace().nth(1).unwrap_or_default().to_string();
                    let page = url::Url::parse(&format!("http://local{target}"))
                        .ok()
                        .and_then(|u| u.query_pairs().find(|(k, _)| k == "page").and_then(|(_, v)| v.parse::<usize>().ok()))
                        .unwrap_or(0);
                    seen.lock().unwrap().push(target);
                    let (status, body) = match pages.get(page) {
                        Some(body) => (200, body.as_str()),
                        None => (404, "no such page"),
                    };
                    let head = format!(
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = sock.write_all(head.as_bytes()).await;
                    let _ = sock.write_all(body.as_bytes()).await;
                    let _ = sock.shutdown().await;
                });
            }
        });
        Self { base, targets }
    }

    /// `http://127.0.0.1:<port>`
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Every request target (path and query) so far, oldest first.
    pub fn targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }
}

/// An adapter context for the period 2026-W46 whose endpoints are all on `fx` (`/hn`, `/yc`), with
/// no delay between requests, `known` already collected and no settled hiring thread.
pub fn ctx(fx: &Fixtures, known: &[&str]) -> Ctx {
    let limits = CollectLimits { host_delay: Duration::ZERO, allow_private_hosts: true, ..CollectLimits::default() };
    let config = TrendConfig {
        endpoints: Endpoints {
            hackernews: format!("{}/hn", fx.base()),
            yc: format!("{}/yc", fx.base()),
        },
        limits,
    };
    Ctx {
        http: Http::new(&config.limits.http()).expect("http client"),
        config: Arc::new(config),
        known: Arc::new(known.iter().map(|s| s.to_string()).collect::<HashSet<_>>()),
        period: Period::parse("2026-W46").expect("period"),
    }
}
