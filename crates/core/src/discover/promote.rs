//! P12 - from a lead to an application with a CV (design: `docs/plans/P12-queue.md`).
//!
//! ```text
//! let app = core.prepare_cv(lead_id).await?;       // "Prepare CV": lead -> Draft application + CV
//! core.dismiss_lead_with_reason(id, "too junior", true)?;   // "Dismiss", optionally "never this company"
//! ```
//!
//! Nothing here runs on its own: a lead becomes an application only when the user asks for it
//! ([`Core::prepare_cv`]); discovery itself never promotes.
//!
//! The `prepare_cv` graph (one thread per lead, `lead:<id>`):
//!
//! ```text
//! promote -> generate -> END
//! ```
//!
//! - `promote`: [`Core::promote`], idempotent: the application is created once, in the same
//!   transaction that marks the lead `promoted`; asking again returns it.
//! - `generate`: the existing `pipeline::generate_into` on the stored posting text, its progress
//!   forwarded to the graph's progress stream ([`Graphs::note`](crate::graph::Graphs::note)). It
//!   skips work when the CV is already there.
//!
//! The application ends as a normal `Draft` with its CV: "Approve CV" (the user's call) is the
//! next step, as for any other application. A failure ends the thread as failed; [`Core::prepare_cv`]
//! again resumes at the node that failed. Threads run one at a time (a CV takes several model
//! calls), so preparing several leads queues them.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use super::score::excluded_company;
use crate::graph::{Failure, GraphDef, Outcome, ThreadStatus, END, START};
use crate::llm::Llm;
use crate::pipeline::generate::{shared_llm, strip_jd_header};
use crate::pipeline::{JdInput, Pipeline, Progress};
use crate::store::{Application, Lead, LeadState, NewApplication, GRAPH_FAILED};
use crate::Core;

/// Graph name of "Prepare CV" (thread `lead:<lead id>`).
pub const PREPARE_GRAPH: &str = "prepare_cv";

/// One CV at a time: it is several model calls and a render, and the user reviews them in order.
static SLOT: Semaphore = Semaphore::const_new(1);

/// Writing the CV can take a while with a slow model.
const GENERATE_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// The thread of one lead.
pub fn prepare_thread(lead_id: i64) -> String {
    format!("lead:{lead_id}")
}

/// The lead of a `prepare_cv` thread.
pub fn parse_prepare_thread(thread: &str) -> Option<i64> {
    thread.strip_prefix("lead:")?.parse().ok()
}

/// The state of the `prepare_cv` graph.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PrepareState {
    pub lead_id: i64,
    /// Set by `promote`.
    pub app_id: Option<i64>,
}

/// A lead whose CV could not be written: the application exists (a Draft without CV), and
/// [`Core::prepare_cv`] resumes where it stopped.
#[derive(Clone, Debug)]
pub struct FailedPrepare {
    pub lead: Lead,
    pub failure: Failure,
}

/// What the nodes share.
struct Env {
    core: Core,
    llm: Arc<dyn Llm>,
}

fn prepare_graph(env: &Arc<Env>) -> GraphDef<PrepareState> {
    let (to_promote, to_generate) = (env.clone(), env.clone());
    GraphDef::new(PREPARE_GRAPH)
        .node_timeout("generate", GENERATE_TIMEOUT)
        .node("promote", move |s| {
            let env = to_promote.clone();
            async move { promote_node(&env, s) }
        })
        .node("generate", move |s| {
            let env = to_generate.clone();
            async move { generate_node(&env, s).await }
        })
        .edge(START, "promote")
        .edge("promote", "generate")
        .edge("generate", END)
}

fn promote_node(env: &Env, mut s: PrepareState) -> Result<PrepareState> {
    s.app_id = Some(env.core.promote(s.lead_id)?.id);
    Ok(s)
}

async fn generate_node(env: &Env, s: PrepareState) -> Result<PrepareState> {
    let core = &env.core;
    let app_id = s.app_id.context("the application was not created")?;
    let app = core.store.get_application(app_id)?.with_context(|| format!("application {app_id} no longer exists"))?;
    // A retry after the CV was written (the thread was lost): nothing to do, and never overwrite
    // a CV the user may have edited.
    if has_cv(&app) {
        return Ok(s);
    }
    let lead = core.store.get_lead(s.lead_id)?.with_context(|| format!("lead {} no longer exists", s.lead_id))?;
    let jd = read_lead_jd(&lead)?;
    let (graphs, thread) = (core.graphs.clone(), prepare_thread(s.lead_id));
    let pipeline = Pipeline::new(core, &*env.llm).with_insight(env.llm.clone());
    let mut app = pipeline
        .generate_into(app_id, JdInput::Text(jd), |p| graphs.note(&thread, PREPARE_GRAPH, "generate", &progress_label(&p)))
        .await?;
    // The analysis may not name an apply page: the posting's own address is the best there is.
    if app.apply_url.is_none() {
        app.apply_url = Some(lead.url);
        core.store.save_application(&app)?;
    }
    Ok(s)
}

fn progress_label(p: &Progress) -> String {
    match p.detail.as_str() {
        "" => p.stage.label().to_string(),
        detail => format!("{}: {detail}", p.stage.label()),
    }
}

fn has_cv(app: &Application) -> bool {
    app.cv_yaml_path().is_file() && app.cv_pdf_path().is_file()
}

/// The posting text of a lead (`jd.md` without its header).
fn read_lead_jd(lead: &Lead) -> Result<String> {
    let path = lead.jd_path.as_deref().map(Path::to_path_buf).context("the posting text of this lead was not saved")?;
    let raw = std::fs::read_to_string(&path).with_context(|| format!("reading the posting text at {}", path.display()))?;
    Ok(strip_jd_header(&raw).to_string())
}

impl Core {
    /// A copy of the handles the graph nodes need (`Core` is not `Clone`: its parts are).
    fn handles(&self) -> Core {
        Core {
            paths: self.paths.clone(),
            store: self.store.clone(),
            secrets: self.secrets.clone(),
            graphs: self.graphs.clone(),
        }
    }

    /// Turn a lead into a Draft application: the application is created from the stored posting
    /// (its text is read when the CV is written), the insight is copied, and the lead is marked
    /// `promoted`. Idempotent: a lead that was promoted returns its application. Only the user's
    /// "Prepare CV" calls this (through [`prepare_cv`](Self::prepare_cv)); discovery never does.
    pub fn promote(&self, lead_id: i64) -> Result<Application> {
        let lead = self.store.get_lead(lead_id)?.with_context(|| format!("lead {lead_id} no longer exists"))?;
        match lead.state {
            LeadState::Promoted => {
                let app_id = lead.app_id.context("the application of this lead was deleted")?;
                return self.store.get_application(app_id)?.context("the application of this lead was deleted");
            }
            LeadState::Dismissed => bail!("this lead was dismissed"),
            LeadState::New => {}
        }
        // Refuse before anything is created: without the text there is no CV to write.
        read_lead_jd(&lead)?;
        let slug = self.store.unique_slug("draft")?;
        let new = NewApplication {
            dir: self.paths.application_dir(&slug),
            slug,
            company: lead.company.clone(),
            role: lead.role.clone(),
            source: Some(lead.url.clone()),
            apply_url: Some(lead.url.clone()),
        };
        let app = self.store.promote_lead_to_application(lead.id, &new, lead.insight_json.as_deref())?;
        tracing::info!(lead = lead.id, app = app.id, "lead promoted");
        Ok(app)
    }

    /// "Prepare CV": promote the lead and write its CV with the default AI provider. Returns the
    /// application (a `Draft` with `cv.yaml` and `cv.pdf`; "Approve CV" is the user's next step).
    ///
    /// Several calls queue: one CV is written at a time. A failure (no provider, network, the fact
    /// guard) is returned as the error and leaves the thread failed; calling this again for the
    /// same lead resumes at the node that failed, without a second application. Run it on the
    /// global runtime and follow it with `core.graphs.subscribe()` (thread [`prepare_thread`]).
    pub async fn prepare_cv(&self, lead_id: i64) -> Result<Application> {
        self.prepare_cv_with(shared_llm(self)?, lead_id).await
    }

    /// [`prepare_cv`](Self::prepare_cv) with an explicit model.
    pub async fn prepare_cv_with(&self, llm: Arc<dyn Llm>, lead_id: i64) -> Result<Application> {
        let _turn = SLOT.acquire().await.context("the CV queue is closed")?;
        let thread = prepare_thread(lead_id);
        let env = Arc::new(Env { core: self.handles(), llm });
        let graph = prepare_graph(&env);
        let outcome = match self.graphs.status(&thread)?.map(|t| t.status) {
            Some(ThreadStatus::Failed(_) | ThreadStatus::Waiting(_)) => self.graphs.resume(&graph, &thread, |_| {}).await?,
            Some(ThreadStatus::Running) => bail!("a CV for this lead is being written right now"),
            Some(ThreadStatus::Done) | None => {
                if let Some(app) = self.prepared(lead_id)? {
                    return Ok(app);
                }
                self.graphs.run(&graph, &thread, PrepareState { lead_id, app_id: None }).await?
            }
        };
        match outcome {
            Outcome::Done(state) => {
                let app_id = state.app_id.context("the CV was written but the application is unknown")?;
                self.store.get_application(app_id)?.context("the application of this lead was deleted")
            }
            Outcome::Failed(failure) => Err(anyhow!("{}", failure.message)),
            Outcome::Cancelled => bail!("preparing the CV was cancelled"),
            Outcome::Waiting(_) => bail!("preparing the CV paused unexpectedly"),
        }
    }

    /// "Prepare CV" on several leads: one after the other, in the order given. Each lead has its
    /// own thread and its own result; one failing does not stop the next.
    pub async fn prepare_cvs(&self, lead_ids: &[i64]) -> Vec<(i64, Result<Application>)> {
        let mut out = Vec::with_capacity(lead_ids.len());
        for id in lead_ids {
            out.push((*id, self.prepare_cv(*id).await));
        }
        out
    }

    /// The application of a promoted lead whose CV is written, if there is one.
    fn prepared(&self, lead_id: i64) -> Result<Option<Application>> {
        let Some(lead) = self.store.get_lead(lead_id)? else { return Ok(None) };
        let Some(app_id) = lead.app_id.filter(|_| lead.state == LeadState::Promoted) else { return Ok(None) };
        Ok(self.store.get_application(app_id)?.filter(has_cv))
    }

    /// Leads whose "Prepare CV" failed and waits for a retry (survives a restart), oldest first.
    pub fn prepare_failures(&self) -> Result<Vec<FailedPrepare>> {
        let mut out = Vec::new();
        for row in self.store.graph_threads_with_status(GRAPH_FAILED)?.into_iter().filter(|t| t.graph == PREPARE_GRAPH) {
            let Some(lead_id) = parse_prepare_thread(&row.thread) else { continue };
            let (Some(lead), Some(info)) = (self.store.get_lead(lead_id)?, self.graphs.status(&row.thread)?) else {
                continue;
            };
            if let ThreadStatus::Failed(failure) = info.status {
                out.push(FailedPrepare { lead, failure });
            }
        }
        Ok(out)
    }

    /// Forget a failed "Prepare CV": the thread is deleted. The application stays as a Draft
    /// (the sidebar's Generate still works on it).
    pub fn prepare_forget(&self, lead_id: i64) -> Result<()> {
        self.graphs.cancel(&prepare_thread(lead_id))
    }

    /// "Dismiss": the lead leaves the queue, with an optional `reason` (kept on the lead).
    /// With `exclude_company` the company is also added to the preferences' `exclude_companies`
    /// ("don't show this company again": later runs filter it out before any model call), and the
    /// company's other `new` leads are dismissed with it. Returns how many other leads went.
    /// Never promotes anything.
    pub fn dismiss_lead_with_reason(&self, lead_id: i64, reason: &str, exclude_company: bool) -> Result<usize> {
        let lead = self.store.get_lead(lead_id)?.with_context(|| format!("lead {lead_id} no longer exists"))?;
        if lead.state != LeadState::New {
            bail!("this lead is already {}", lead.state.as_str());
        }
        let reason = reason.trim();
        let company = lead.company.trim();
        let exclusion = exclude_company && !company.is_empty();
        // The preference first: if it cannot be saved nothing has changed.
        let prefs = if exclusion {
            let mut prefs = self.preferences()?;
            prefs.exclude_companies.push(company.to_string());
            Some(self.save_preferences(prefs)?)
        } else {
            None
        };
        self.store.dismiss_lead(lead_id, reason)?;
        tracing::info!(lead = lead_id, exclusion, "lead dismissed");
        let Some(prefs) = prefs else { return Ok(0) };
        let mut also = 0;
        for other in self.store.list_leads(LeadState::New)? {
            if let Some(why) = excluded_company(&prefs, &other.company) {
                if self.store.dismiss_lead(other.id, &why).is_ok() {
                    also += 1;
                }
            }
        }
        Ok(also)
    }
}

#[cfg(test)]
mod tests;
