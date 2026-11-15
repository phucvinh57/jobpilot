//! From a lead to an application with a CV.
//!
//! ```text
//! let app = core.prepare_cv(lead_id).await?;       // "Prepare CV": lead -> Draft application + CV
//! core.dismiss_lead_with_reason(id, "too junior", true).await?;   // "Dismiss", optionally "never this company"
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
//! - `generate`: the existing `pipeline::generate_into` on the posting text (the application's
//!   `jd.md`, written by `promote`), its progress forwarded to the graph's progress stream
//!   ([`Graphs::note`](crate::engine::graph::Graphs::note)). It skips the pipeline only when its result was
//!   saved (the CV files exist and so do the gaps the pipeline records last); either way the
//!   application keeps an apply URL (the posting's own when the analysis names none).
//!
//! The application ends as a normal `Draft` with its CV: "Approve CV" (the user's call) is the
//! next step, as for any other application. A failure ends the thread as failed; [`Core::prepare_cv`]
//! again resumes at the node that failed. Threads run one at a time (a CV takes several model
//! calls), so preparing several leads queues them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

use super::score::excluded_company;
use crate::engine::graph::{Failure, GraphDef, Outcome, ThreadStatus, END, START};
use crate::engine::llm::Llm;
use crate::engine::pipeline::generate::{read_jd_text, shared_llm, strip_jd_header, write_jd};
use crate::engine::pipeline::{JdInput, Pipeline, Progress};
use crate::engine::store::{Application, Lead, LeadState, NewApplication, GRAPH_FAILED};
use crate::engine::Core;

/// Graph name of "Prepare CV" (thread `lead:<lead id>`).
pub const PREPARE_GRAPH: &str = "prepare_cv";

/// One CV at a time: it is several model calls and a render, and the user reviews them in order.
static SLOT: Semaphore = Semaphore::const_new(1);

/// Leads whose "Prepare CV" is queued or running, per database, with how many calls hold each.
/// "Dismiss, never this company" leaves them alone, and the lead itself cannot be dismissed
/// meanwhile.
static BUSY: Mutex<BTreeMap<(PathBuf, i64), usize>> = Mutex::new(BTreeMap::new());

fn busy() -> MutexGuard<'static, BTreeMap<(PathBuf, i64), usize>> {
    BUSY.lock().unwrap_or_else(|e| e.into_inner())
}

/// Marks a lead as being prepared until dropped.
struct Preparing(PathBuf, i64);

impl Preparing {
    fn new(core: &Core, lead_id: i64) -> Self {
        let key = (core.paths.db.clone(), lead_id);
        *busy().entry(key).or_insert(0) += 1;
        Self(core.paths.db.clone(), lead_id)
    }
}

impl Drop for Preparing {
    fn drop(&mut self) {
        let key = (std::mem::take(&mut self.0), self.1);
        let mut busy = busy();
        if let Some(n) = busy.get_mut(&key) {
            *n -= 1;
            if *n == 0 {
                busy.remove(&key);
            }
        }
    }
}

/// What "Dismiss" did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Dismissed {
    /// Other leads of the company dismissed with it ("never this company").
    pub also: usize,
    /// Other leads of the company left in the queue because their CV is queued or being written.
    pub kept: usize,
    /// "Never this company" was asked for but the preference could not be saved (why); the lead
    /// itself was dismissed all the same.
    pub not_excluded: Option<String>,
}

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
    let lead = core.store.get_lead(s.lead_id)?.with_context(|| format!("lead {} no longer exists", s.lead_id))?;
    // A retry after the CV was written and saved (the thread was lost): nothing to write, and
    // never overwrite a CV the user may have edited. Files alone do not count: a run that died
    // before saving its result left them behind.
    if cv_done(&app) {
        return keep_apply_url(core, app_id, &lead.url).map(|()| s);
    }
    let jd = read_jd_text(&app).or_else(|_| read_lead_jd(&lead))?;
    let (graphs, thread) = (core.graphs.clone(), prepare_thread(s.lead_id));
    let pipeline = Pipeline::new(core, &*env.llm).with_insight(env.llm.clone());
    let written = pipeline
        .generate_into(app_id, JdInput::Text(jd), |p| graphs.note(&thread, PREPARE_GRAPH, "generate", &progress_label(&p)))
        .await;
    // The analysis may not name an apply page: the posting's own address is the best there is.
    // Also after a failure: the Draft that stays must still have somewhere to apply.
    let kept = keep_apply_url(core, app_id, &lead.url);
    written?;
    kept?;
    Ok(s)
}

/// The posting's address when it is a web link: a lead the user added from pasted text or a file
/// has a `manual:` address instead, which is not somewhere to apply.
fn web_link(url: &str) -> Option<String> {
    let lower = url.trim().to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| url.to_string())
}

/// Give the application the posting's address as apply URL when it has none.
fn keep_apply_url(core: &Core, app_id: i64, posting_url: &str) -> Result<()> {
    let Some(mut app) = core.store.get_application(app_id)? else { return Ok(()) };
    if app.apply_url.is_none() {
        let Some(url) = web_link(posting_url) else { return Ok(()) };
        app.apply_url = Some(url);
        core.store.save_application(&app)?;
    }
    Ok(())
}

fn progress_label(p: &Progress) -> String {
    match p.detail.as_str() {
        "" => p.stage.label().to_string(),
        detail => format!("{}: {detail}", p.stage.label()),
    }
}

/// The CV was written and its result saved: the files exist, and `gaps_json`, which the pipeline
/// stores last (after the render, before it reports), is set.
fn cv_done(app: &Application) -> bool {
    app.cv_yaml_path().is_file() && app.cv_pdf_path().is_file() && app.gaps_json.is_some()
}

/// The posting text of a lead (`jd.md` without its header).
fn read_lead_jd(lead: &Lead) -> Result<String> {
    let path = lead.jd_path.as_deref().map(Path::to_path_buf).context("the posting text of this lead was not saved")?;
    let raw = std::fs::read_to_string(&path).with_context(|| format!("reading the posting text at {}", path.display()))?;
    Ok(strip_jd_header(&raw).to_string())
}

/// `jd.md` of a new Draft: the posting text under the usual header.
fn write_posting(app: &Application, text: &str) -> Result<()> {
    std::fs::create_dir_all(&app.dir).with_context(|| format!("creating {}", app.dir.display()))?;
    write_jd(app, text)
}

impl Core {
    /// Is "Prepare CV" queued or running for this lead?
    pub fn is_preparing(&self, lead_id: i64) -> bool {
        busy().contains_key(&(self.paths.db.clone(), lead_id))
    }

    /// A copy of the handles the graph nodes need (`Core` is not `Clone`: its parts are).
    fn handles(&self) -> Core {
        Core {
            paths: self.paths.clone(),
            store: self.store.clone(),
            secrets: self.secrets.clone(),
            graphs: self.graphs.clone(),
        }
    }

    /// Turn a lead into a Draft application: the application is created with the posting text as
    /// its `jd.md` (same layout as any application, so Generate and Regenerate work on the Draft),
    /// the insight is copied, and the lead is marked `promoted`. Idempotent: a lead that was
    /// promoted returns its application. If that application was deleted since, the lead can be
    /// promoted again (a new application). Only the user's "Prepare CV" calls this (through
    /// [`prepare_cv`](Self::prepare_cv)); discovery never does.
    pub fn promote(&self, lead_id: i64) -> Result<Application> {
        let lead = self.store.get_lead(lead_id)?.with_context(|| format!("lead {lead_id} no longer exists"))?;
        match lead.state {
            LeadState::Dismissed => bail!("this lead was dismissed"),
            LeadState::Promoted => {
                if let Some(app) = lead.app_id.map(|id| self.store.get_application(id)).transpose()?.flatten() {
                    // A crash between the transaction and the file leaves a Draft without text.
                    if !cv_done(&app) && !app.jd_path().is_file() {
                        write_posting(&app, &read_lead_jd(&lead)?)?;
                    }
                    return Ok(app);
                }
            }
            LeadState::New => {}
        }
        // Refuse before anything is created: without the text there is no CV to write.
        let text = read_lead_jd(&lead)?;
        let slug = self.store.unique_slug("draft")?;
        let new = NewApplication {
            dir: self.paths.application_dir(&slug),
            slug,
            company: lead.company.clone(),
            role: lead.role.clone(),
            source: web_link(&lead.url),
            apply_url: web_link(&lead.url),
        };
        let app = self.store.promote_lead_to_application(lead.id, &new, lead.insight_json.as_deref())?;
        write_posting(&app, &text)?;
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
        let held = Preparing::new(self, lead_id);
        self.prepare_held(llm, lead_id, held).await
    }

    /// The body of [`prepare_cv_with`](Self::prepare_cv_with): `held` marks the lead as being
    /// prepared (a batch holds all its leads from the start) and is released when this ends.
    async fn prepare_held(&self, llm: Arc<dyn Llm>, lead_id: i64, held: Preparing) -> Result<Application> {
        let _held = held;
        let _turn = SLOT.acquire().await.context("the CV queue is closed")?;
        let thread = prepare_thread(lead_id);
        let env = Arc::new(Env { core: self.handles(), llm });
        let graph = prepare_graph(&env);
        let mut status = self.graphs.status(&thread)?.map(|t| t.status);
        // The application was deleted since the thread ran: its saved state points at nothing.
        // Start again, with a new application.
        if matches!(status, Some(ThreadStatus::Done | ThreadStatus::Failed(_) | ThreadStatus::Waiting(_)))
            && self.store.get_lead(lead_id)?.is_some_and(|l| l.state == LeadState::Promoted && l.app_id.is_none())
        {
            self.graphs.cancel(&thread)?;
            status = None;
        }
        let outcome = match status {
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
        self.prepare_cvs_each(lead_ids, |id, result| out.push((id, result))).await;
        out
    }

    /// [`prepare_cvs`](Self::prepare_cvs), handing each result to `done` as soon as it is known
    /// (for a screen that shows them one by one). All the leads count as being prepared from the
    /// start: dismissing a company does not take the ones still waiting their turn.
    pub async fn prepare_cvs_each(&self, lead_ids: &[i64], mut done: impl FnMut(i64, Result<Application>)) {
        match shared_llm(self) {
            Ok(llm) => self.prepare_cvs_each_with(llm, lead_ids, done).await,
            // No provider: the same answer for every lead.
            Err(e) => lead_ids.iter().for_each(|id| done(*id, Err(anyhow!("{e:#}")))),
        }
    }

    /// [`prepare_cvs_each`](Self::prepare_cvs_each) with an explicit model.
    pub async fn prepare_cvs_each_with(
        &self,
        llm: Arc<dyn Llm>,
        lead_ids: &[i64],
        mut done: impl FnMut(i64, Result<Application>),
    ) {
        // In reverse, so that `pop` hands them out in order.
        let mut held: Vec<Preparing> = lead_ids.iter().rev().map(|id| Preparing::new(self, *id)).collect();
        for id in lead_ids {
            let Some(mine) = held.pop() else { break };
            done(*id, self.prepare_held(llm.clone(), *id, mine).await);
        }
    }

    /// The application of a promoted lead whose CV is written, if there is one.
    fn prepared(&self, lead_id: i64) -> Result<Option<Application>> {
        let Some(lead) = self.store.get_lead(lead_id)? else { return Ok(None) };
        let Some(app_id) = lead.app_id.filter(|_| lead.state == LeadState::Promoted) else { return Ok(None) };
        Ok(self.store.get_application(app_id)?.filter(cv_done))
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
    /// company's other leads in the queue are dismissed with it, except those whose CV is queued
    /// or being written ([`Dismissed::kept`]). Never promotes anything.
    ///
    /// The lead goes first: if the exclusion cannot be saved after that, the lead stays dismissed
    /// and [`Dismissed::not_excluded`] says why. A lead whose CV is queued or being written cannot
    /// be dismissed (an error).
    pub async fn dismiss_lead_with_reason(&self, lead_id: i64, reason: &str, exclude_company: bool) -> Result<Dismissed> {
        let lead = self.store.get_lead(lead_id)?.with_context(|| format!("lead {lead_id} no longer exists"))?;
        if !lead.in_queue() {
            bail!("this lead is already {}", lead.state.as_str());
        }
        let reason = reason.trim();
        {
            // Held while the lead is dismissed: a "Prepare CV" cannot slip in between the check
            // and the change.
            let busy = busy();
            if busy.contains_key(&(self.paths.db.clone(), lead_id)) {
                bail!("its CV is being prepared: wait for it to finish first");
            }
            self.store.dismiss_lead(lead_id, reason)?;
        }
        let company = lead.company.trim();
        let exclusion = exclude_company && !company.is_empty();
        tracing::info!(lead = lead_id, exclusion, "lead dismissed");
        if !exclusion {
            return Ok(Dismissed::default());
        }
        // Through the preferences edit lock ([`Core::edit_preferences`]): a chip click or a notes
        // read that is in the middle of its own read-change-write must not write the old list back
        // over the exclusion.
        let saved = self
            .edit_preferences(|prefs| {
                if excluded_company(prefs, company).is_some() {
                    return false;
                }
                prefs.exclude_companies.push(company.to_string());
                true
            })
            .await;
        let prefs = match saved {
            Ok(prefs) => prefs,
            Err(e) => {
                tracing::warn!(lead = lead_id, "excluding a company failed: {e:#}");
                return Ok(Dismissed { not_excluded: Some(format!("{e:#}")), ..Dismissed::default() });
            }
        };
        let mut out = Dismissed::default();
        let busy = busy();
        for other in self.store.list_queue_leads()? {
            let Some(why) = excluded_company(&prefs, &other.company) else { continue };
            if busy.contains_key(&(self.paths.db.clone(), other.id)) {
                out.kept += 1;
            } else if self.store.dismiss_lead(other.id, &why).is_ok() {
                out.also += 1;
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests;
