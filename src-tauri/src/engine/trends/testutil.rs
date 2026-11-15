//! Test helpers of the trend adapters: the local fixture server ([`crate::engine::net::fixtures`]), a PDF
//! maker and an adapter context pointed at the fixtures.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use super::config::{CollectLimits, Endpoints, TrendConfig};
use super::http::Http;
use super::period::Period;
use super::source::Ctx;

pub use crate::engine::net::fixtures::{Fixtures, Hit};

/// A PDF made from Typst markup (the `typst` CLI is a prerequisite of the project).
pub fn typst_pdf(markup: &str) -> Vec<u8> {
    let dir = tempfile::tempdir().expect("temp dir");
    let (typ, pdf) = (dir.path().join("r.typ"), dir.path().join("r.pdf"));
    std::fs::write(&typ, markup).expect("write markup");
    let status = std::process::Command::new(crate::engine::pipeline::render::typst_binary())
        .arg("compile")
        .arg(&typ)
        .arg(&pdf)
        .status()
        .expect("typst is needed for tests");
    assert!(status.success(), "typst failed");
    std::fs::read(&pdf).expect("read pdf")
}

/// An adapter context for the period 2026-W46 whose endpoints are all on `fx` (`/hn`, `/yc`, and
/// the search APIs at the root), with no delay between requests, `known` already collected and no
/// settled hiring thread.
pub fn ctx(fx: &Fixtures, known: &[&str]) -> Ctx {
    let limits = CollectLimits { host_delay: Duration::ZERO, allow_private_hosts: true, ..CollectLimits::default() };
    let config = TrendConfig {
        endpoints: Endpoints {
            hackernews: format!("{}/hn", fx.base()),
            yc: format!("{}/yc", fx.base()),
            brave: fx.base().to_string(),
            tavily: fx.base().to_string(),
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
