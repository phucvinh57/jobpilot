//! What a discovery run needs from the outside, as values the caller can replace: the model, the
//! source adapters, the posting fetch, the challenge detection, the insight step and the
//! notification. [`DiscoverDeps::new`] has the real defaults without a browser; [`DiscoverDeps::live`]
//! adds the managed browser (page adapters, posting fallback) and the insight; tests swap in fakes.

use std::sync::Arc;

use anyhow::{Context, Result};
use futures::future::BoxFuture;
use futures::FutureExt;

use super::fetch::{self, MAX_POSTING_BYTES};
use super::handoff::text_probe;
use super::limits::DiscoverLimits;
use super::source::{adapter_for_pages, Candidate, JobSource, NetHttp, NetPages, Pages};
use crate::engine::browser::render::Renderer;
use crate::engine::graph::nodes::ChallengeKind;
use crate::engine::llm::Llm;
use crate::engine::net::http::{Http, HttpLimits};
use crate::engine::store::Source;
use crate::engine::Core;

/// Largest JSON answer an adapter reads (a "Who is hiring" thread is several MB).
const JSON_MAX_BYTES: usize = 32 * 1024 * 1024;

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

/// The challenge hook: `Some(kind)` when the user has to step in. The default is
/// [`text_probe`](super::handoff::text_probe): after a posting was fetched it reads the start of the
/// text for a block, captcha or login page. A page rendered in the browser reports its own challenge
/// (the adapter or the fetch returns `ChallengeFound`), which the runner treats the same way. An
/// `Err` fails the source's thread.
pub type ChallengeProbe =
    Arc<dyn Fn(ChallengeCheck) -> BoxFuture<'static, Result<Option<ChallengeKind>>> + Send + Sync>;

/// A posting to fetch when its adapter has no text for it.
#[derive(Clone, Debug)]
pub struct FetchRequest {
    pub source: Source,
    pub candidate: Candidate,
}

/// The posting fetch hook: the page's readable text. An error that [`classify`]s as a network
/// problem fails the thread (Retry); any other error skips that posting. The default reads the page
/// through the shared safe client (`net::http`, which spaces requests to one host itself; a
/// replacement fetcher does its own pacing) and extracts the text like `pipeline::ingest`. Its
/// `NeedsBrowser` answer skips the posting, except in [`DiscoverDeps::live`], where the page is
/// rendered in the managed browser and a challenge shown there (`ChallengeFound`) pauses the source.
///
/// [`classify`]: crate::engine::health::classify
pub type PostingFetcher = Arc<dyn Fn(FetchRequest) -> BoxFuture<'static, Result<String>> + Send + Sync>;

/// A lead that passed the hard filters, for the insight step.
#[derive(Clone, Debug)]
pub struct InsightRequest {
    pub url: String,
    pub company: String,
    pub role: String,
    pub jd_text: String,
}

/// The insight hook: the lead's job insight as JSON text (stored in `leads.insight_json`), or
/// `None`. Runs next to the scoring. An error is logged and the lead is saved without insight.
/// [`DiscoverDeps::new`] returns `None`; [`DiscoverDeps::live`] wires `pipeline::insight`.
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
    pub limits: DiscoverLimits,
}

impl DiscoverDeps {
    /// The real adapters and fetch with `llm` and `limits`; the text-based challenge probe, no
    /// browser (a page that needs one is skipped), no insight hook, desktop notifications. The
    /// adapters' JSON calls, their page reads and the posting fetch go through one shared client, so
    /// they share its host check and per-host delays.
    pub fn new(llm: Arc<dyn Llm>, limits: DiscoverLimits) -> Result<Self> {
        Self::build(llm, limits, None)
    }

    fn build(llm: Arc<dyn Llm>, limits: DiscoverLimits, renderer: Option<Renderer>) -> Result<Self> {
        let client = Http::new(&HttpLimits {
            host_delay: limits.host_delay,
            max_bytes: JSON_MAX_BYTES,
            allow_private_hosts: limits.allow_private_hosts,
            ..HttpLimits::default()
        })?;
        let pages_client = client.with_max_bytes(MAX_POSTING_BYTES);
        let fetch = match &renderer {
            Some(renderer) => fetch::fetcher_with_browser(pages_client.clone(), renderer.clone()),
            None => fetch::fetcher(pages_client.clone()),
        };
        let pages: Arc<dyn Pages> = Arc::new(NetPages::new(pages_client, renderer));
        let json = Arc::new(NetHttp::new(client));
        Ok(Self {
            llm,
            sources: Arc::new(move |source| adapter_for_pages(source, json.clone(), pages.clone())),
            fetch,
            challenge: text_probe(),
            insight: Arc::new(|_| async { Ok(None) }.boxed()),
            notify: Arc::new(crate::engine::notify::desktop_notify),
            limits,
        })
    }

    /// The defaults for the running app: the default AI provider (`Err` with the "no AI provider"
    /// message, which the UI shows as the setup problem, when there is none), default limits, the
    /// managed browser as the page reader of the `yc` and `career_page` sources and the fallback of the
    /// posting fetch (it sees login, captcha and block pages, and the run waits for the user), and the
    /// job insight (`pipeline::insight`) as the insight hook.
    pub fn live(core: &Core) -> Result<Self> {
        let limits = DiscoverLimits::default();
        let renderer = Renderer::new(core, limits.allow_private_hosts);
        let mut deps = Self::build(Arc::from(core.default_llm()?), limits, Some(renderer))?;
        deps.insight = insight_hook(core, deps.llm.clone());
        Ok(deps)
    }
}

/// The insight hook backed by `pipeline::insight`: the brief of the posting (its address decides
/// which company pages are read), as the JSON stored in `leads.insight_json`; `None` when the
/// model found nothing to say. Needs a `Core` that outlives the call, so it keeps a copy of the
/// handles (store, graphs, secrets and paths are all cheap shared handles).
pub(crate) fn insight_hook(core: &Core, llm: Arc<dyn Llm>) -> InsightHook {
    let core = Arc::new(Core {
        paths: core.paths.clone(),
        store: core.store.clone(),
        secrets: core.secrets.clone(),
        graphs: core.graphs.clone(),
    });
    Arc::new(move |req| {
        let (core, llm) = (core.clone(), llm.clone());
        async move {
            let insight = crate::engine::pipeline::insight::insight(&core, llm, &req.jd_text, Some(&req.url), None).await?;
            if !insight.has_content() {
                return Ok(None);
            }
            Ok(Some(serde_json::to_string(&insight).context("serialising the insight")?))
        }
        .boxed()
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::engine::llm::fake::FakeLlm;
    use crate::engine::pipeline::insight::Insight;
    use crate::engine::{Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn request() -> InsightRequest {
        InsightRequest {
            // A job board names no company site, so the brief reads nothing but the posting.
            url: "https://boards.greenhouse.io/acme/jobs/1".into(),
            company: "Acme".into(),
            role: "Engineer".into(),
            jd_text: "Acme Robotics builds warehouse robots and needs a Rust engineer for its fleet services.".into(),
        }
    }

    #[tokio::test]
    async fn the_live_insight_hook_stores_the_brief_as_json() {
        let (_dir, core) = core();
        let llm = Arc::new(FakeLlm::with_replies([json!({
            "domain": "Warehouse robotics",
            "business_summary": "Builds robots for warehouses.",
            "company": {"size": "", "stage": "Series B", "funding": "", "hq": "", "founded": ""},
            "interview": {"stages": [], "notes": "", "found": false},
            "source_pages": []
        })]));
        let hook = insight_hook(&core, llm);
        let stored = hook(request()).await.unwrap().expect("a brief");
        let brief = Insight::from_json(&stored).expect("the stored text is the brief's JSON");
        assert_eq!((brief.domain.as_str(), brief.business_summary.as_str()), ("Warehouse robotics", "Builds robots for warehouses."));
        assert!(brief.sources.iter().any(|u| u.contains("greenhouse.io/acme/jobs/1")), "{:?}", brief.sources);
    }

    #[tokio::test]
    async fn an_empty_brief_stores_nothing_and_a_model_error_is_the_hooks_error() {
        let (_dir, core) = core();
        let empty = Arc::new(FakeLlm::with_replies([json!({
            "domain": "", "business_summary": "",
            "company": {"size": "", "stage": "", "funding": "", "hq": "", "founded": ""},
            "interview": {"stages": [], "notes": "", "found": false},
            "source_pages": []
        })]));
        assert_eq!(insight_hook(&core, empty)(request()).await.unwrap(), None);
        let down = Arc::new(FakeLlm::with_error("HTTP 503 Service Unavailable"));
        assert!(insight_hook(&core, down)(request()).await.is_err(), "the runner logs it and saves the lead without a brief");
    }
}
