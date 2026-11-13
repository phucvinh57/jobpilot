//! What a discovery run needs from the outside, as values the caller can replace: the model, the
//! source adapters, the posting fetch, the challenge detection, the insight step and the
//! notification. [`DiscoverDeps::new`] has the real defaults; tests swap in fakes, and later parts
//! of P10 and P11 plug into the hooks without touching the runner.

use std::sync::Arc;

use anyhow::Result;
use futures::future::BoxFuture;
use futures::FutureExt;

use super::limits::{DiscoverLimits, HostGate};
use super::source::{adapter_for, Candidate, JobSource, ReqwestHttp};
use crate::graph::nodes::ChallengeKind;
use crate::llm::Llm;
use crate::pipeline::ingest::{ingest, JdInput};
use crate::store::Source;
use crate::Core;

/// Where in a source run a challenge check happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckStage {
    /// After a search page: `url` is the source's own URL.
    Search,
    /// After a posting was fetched: `url` is the posting, `text` what was fetched.
    Posting,
}

/// Asks "is there a login, captcha or block page in the way?".
#[derive(Clone, Debug)]
pub struct ChallengeCheck {
    pub source: Source,
    pub stage: CheckStage,
    pub url: String,
    /// The start of the fetched text for [`CheckStage::Posting`]; empty for a search.
    pub text: String,
}

/// The challenge hook: `Some(kind)` when the user has to step in. The API adapters of part A
/// never meet one, so the default answers `None`; `browser/challenge.rs` (part B) fills it in for
/// browser-driven sources. An `Err` fails the source's thread.
pub type ChallengeProbe =
    Arc<dyn Fn(ChallengeCheck) -> BoxFuture<'static, Result<Option<ChallengeKind>>> + Send + Sync>;

/// A posting to fetch when its adapter has no text for it.
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub source: Source,
    pub candidate: Candidate,
}

/// The posting fetch hook: the page's readable text. An error that [`classify`]s as a network
/// problem fails the thread (Retry); any other error skips that posting. The default is a plain
/// HTTP fetch (`pipeline::ingest`), whose `NeedsBrowser` answer skips the posting until a browser
/// fallback is plugged in here.
///
/// [`classify`]: crate::health::classify
pub type PostingFetcher = Arc<dyn Fn(FetchRequest) -> BoxFuture<'static, Result<String>> + Send + Sync>;

/// A lead that passed the hard filters, for the insight step (P11).
#[derive(Clone, Debug)]
pub struct InsightRequest {
    pub url: String,
    pub company: String,
    pub role: String,
    pub jd_text: String,
}

/// The insight hook: the lead's job insight as JSON text (stored in `leads.insight_json`), or
/// `None`. Runs next to the scoring. An error is logged and the lead is saved without insight.
/// The default returns `None`; P11 wires `pipeline::insight` here.
pub type InsightHook = Arc<dyn Fn(InsightRequest) -> BoxFuture<'static, Result<Option<String>>> + Send + Sync>;

/// Builds the adapter of a `sources` row.
pub type SourceFactory = Arc<dyn Fn(&Source) -> Result<Arc<dyn JobSource>> + Send + Sync>;

/// Shows a notification: title, body (ids and labels only).
pub type Notifier = Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Everything a run is made of. Cheap to clone; the fields are public so a caller can replace one.
#[derive(Clone)]
pub struct DiscoverDeps {
    pub llm: Arc<dyn Llm>,
    pub sources: SourceFactory,
    pub fetch: PostingFetcher,
    pub challenge: ChallengeProbe,
    pub insight: InsightHook,
    pub notify: Notifier,
    /// Per-host delay shared by the adapters' requests and the posting fetch.
    pub gate: Arc<HostGate>,
    pub limits: DiscoverLimits,
}

impl DiscoverDeps {
    /// The real adapters and fetch with `llm` and `limits`; no challenge detection and no insight
    /// yet (the hooks are below), desktop notifications.
    pub fn new(llm: Arc<dyn Llm>, limits: DiscoverLimits) -> Self {
        let gate = Arc::new(HostGate::new(limits.host_delay));
        let http = Arc::new(ReqwestHttp::new(gate.clone()));
        Self {
            llm,
            sources: Arc::new(move |source| adapter_for(source, http.clone())),
            fetch: Arc::new(|req| {
                async move { Ok(ingest(JdInput::Url(req.candidate.url)).await?.text) }.boxed()
            }),
            challenge: Arc::new(|_| async { Ok(None) }.boxed()),
            insight: Arc::new(|_| async { Ok(None) }.boxed()),
            notify: Arc::new(crate::notify::desktop_notify),
            gate,
            limits,
        }
    }

    /// The defaults for the running app: the default AI provider (`Err` with the "no AI provider"
    /// message, which the UI shows as the setup problem, when there is none) and default limits.
    pub fn live(core: &Core) -> Result<Self> {
        Ok(Self::new(Arc::from(core.default_llm()?), DiscoverLimits::default()))
    }
}
