//! The orchestrator: JD -> analysis -> tailored CV -> fact guard -> PDF, with progress events.
//!
//! Every operation works on an [`Application`] that exists from the first stage (so the UI can show
//! it) and always ends in status `Draft` with whatever was produced on disk; moving to `CvReady` is
//! the user's "Approve CV".

use std::fmt;
use std::future::Future;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::params;

use super::analyze::{analyze, valid_http_url, Analysis};
use super::cv::Cv;
use super::guard::{format_violations, guard, Violation};
use super::insight;
use super::ingest::{ingest, precheck, JdInput, JdText};
use super::render::{render, RenderResult, TypstError};
use super::tailor::{self, JdContext, TailorOutput};
use crate::engine::llm::Llm;
use crate::engine::store::{Application, NewApplication};
use crate::engine::Core;

/// kv key: pages the CV may take (default 1).
pub const MAX_PAGES_KEY: &str = "cv_max_pages";
const DEFAULT_MAX_PAGES: usize = 1;
/// The role of a placeholder row until the analysis names the job.
const ANALYSING_ROLE: &str = "Analysing the job description";

/// Coarse pipeline stage, for a progress line in the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Fetching,
    Analyzing,
    Tailoring,
    Checking,
    Rendering,
    Done,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Fetching => "Fetching the job description",
            Stage::Analyzing => "Analysing the job description",
            Stage::Tailoring => "Tailoring the CV",
            Stage::Checking => "Checking facts against the profile",
            Stage::Rendering => "Rendering the PDF",
            Stage::Done => "Done",
        }
    }
}

/// One progress notification. The first one of [`generate`] carries the new application's id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    pub app_id: i64,
    pub stage: Stage,
    /// Extra context such as "repairing 2 fact violations"; usually empty.
    pub detail: String,
}

/// The CV still breaks the fact rules after the repair round.
#[derive(Clone, Debug)]
pub struct GuardFailed {
    pub violations: Vec<Violation>,
}

impl fmt::Display for GuardFailed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the CV still breaks {} fact rule(s) after one repair round:\n{}",
            self.violations.len(),
            format_violations(&self.violations)
        )
    }
}

impl std::error::Error for GuardFailed {}

/// Result of [`rerender`] / [`Pipeline::rerender`].
#[derive(Clone, Debug)]
pub struct RerenderOutcome {
    pub render: RenderResult,
    /// Fact-guard findings. Warnings only: the user edited the CV by hand.
    pub violations: Vec<Violation>,
}

type Notify<'a> = &'a (dyn Fn(Progress) + Send + Sync);

/// The pipeline bound to a core and an LLM. The free functions use the default provider; tests and
/// callers that already hold a client use this directly.
pub struct Pipeline<'a> {
    pub core: &'a Core,
    pub llm: &'a dyn Llm,
    /// An owned handle on the same provider for the company brief (P11), which runs as a graph and
    /// so cannot borrow `llm`. `None` = no brief ([`Pipeline::new`]); see [`Pipeline::with_insight`].
    pub insight: Option<Arc<dyn Llm>>,
}

/// The default AI provider as a shared handle: the pipeline borrows it, the brief graph owns a clone.
pub(crate) fn shared_llm(core: &Core) -> Result<Arc<dyn Llm>> {
    Ok(Arc::from(core.default_llm()?))
}

/// Create a Draft application for `input` and run the whole pipeline with the default AI provider.
/// The first progress event carries the application id. On failure the application stays (Draft)
/// with an `error` event; downcast the error to [`super::NeedsBrowser`] or [`GuardFailed`].
pub async fn generate(core: &Core, input: JdInput, progress: impl Fn(Progress) + Send + Sync) -> Result<Application> {
    let llm = shared_llm(core)?;
    Pipeline::new(core, &*llm).with_insight(llm.clone()).generate(input, progress).await
}

/// Re-run the pipeline on an existing application with new input, e.g. the rendered HTML of a page
/// that needed a browser (`JdInput::Html`).
pub async fn generate_into(
    core: &Core,
    app_id: i64,
    input: JdInput,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<Application> {
    let llm = shared_llm(core)?;
    Pipeline::new(core, &*llm).with_insight(llm.clone()).generate_into(app_id, input, progress).await
}

/// New CV from the stored JD and analysis (no fetching).
pub async fn regenerate(core: &Core, app_id: i64, progress: impl Fn(Progress) + Send + Sync) -> Result<Application> {
    let llm = shared_llm(core)?;
    Pipeline::new(core, &*llm).with_insight(llm.clone()).regenerate(app_id, progress).await
}

/// Targeted change to the current CV ("shorter, stress Rust"), guarded and re-rendered.
pub async fn revise(
    core: &Core,
    app_id: i64,
    instruction: &str,
    progress: impl Fn(Progress) + Send + Sync,
) -> Result<Application> {
    let llm = core.default_llm()?;
    Pipeline::new(core, &*llm).revise(app_id, instruction, progress).await
}

/// After a manual edit of `cv.yaml`: parse, guard (warnings), render. No LLM involved.
pub async fn rerender(core: &Core, app_id: i64) -> Result<RerenderOutcome> {
    Pipeline::offline(core).rerender(app_id).await
}

struct NoLlm;

impl Llm for NoLlm {
    fn complete_json<'a>(
        &'a self,
        _: &'a str,
        _: &'a str,
        _: &'a serde_json::Value,
    ) -> crate::engine::llm::LlmFuture<'a, serde_json::Value> {
        Box::pin(std::future::ready(Err(anyhow!("no AI provider is needed here"))))
    }

    fn list_models(&self) -> crate::engine::llm::LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Ok(Vec::new())))
    }
}

static NO_LLM: NoLlm = NoLlm;

impl<'a> Pipeline<'a> {
    pub fn new(core: &'a Core, llm: &'a dyn Llm) -> Self {
        Self { core, llm, insight: None }
    }

    /// Also write the job brief (P11) of an application that has none, with `llm` (normally a clone
    /// of the handle `llm` was borrowed from). The brief runs detached: it never delays or fails the
    /// pipeline, and lands in `insight_json` when done (the UI shows it on its next refresh).
    pub fn with_insight(mut self, llm: Arc<dyn Llm>) -> Self {
        self.insight = Some(llm);
        self
    }

    /// A pipeline for operations that never call the model (`rerender`).
    fn offline(core: &'a Core) -> Self {
        Self { core, llm: &NO_LLM, insight: None }
    }

    pub async fn generate(&self, input: JdInput, progress: impl Fn(Progress) + Send + Sync) -> Result<Application> {
        precheck(&input)?;
        let app = self.create_placeholder(&input)?;
        let notify: Notify = &progress;
        let id = app.id;
        self.logged(id, self.run_generate(app, input, notify)).await
    }

    pub async fn generate_into(
        &self,
        app_id: i64,
        input: JdInput,
        progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<Application> {
        let app = self.load(app_id)?;
        let notify: Notify = &progress;
        self.logged(app_id, self.run_generate(app, input, notify)).await
    }

    pub async fn regenerate(&self, app_id: i64, progress: impl Fn(Progress) + Send + Sync) -> Result<Application> {
        let notify: Notify = &progress;
        self.logged(app_id, async {
            let app = self.load(app_id)?;
            let jd_text = read_jd_text(&app)?;
            let analysis = match stored_analysis(&app) {
                Some(a) => a,
                None => {
                    emit(notify, app.id, Stage::Analyzing, "");
                    let a = analyze(self.llm, &jd_text, app.source.as_deref()).await?;
                    self.save_analysis(&app, &a)?;
                    a
                }
            };
            self.start_brief(&app);
            self.produce(app, &analysis, &jd_text, Task::Fresh, notify).await
        })
        .await
    }

    pub async fn revise(
        &self,
        app_id: i64,
        instruction: &str,
        progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<Application> {
        let instruction = instruction.trim();
        if instruction.is_empty() {
            bail!("write what to change, e.g. \"shorter, stress Rust\"");
        }
        let notify: Notify = &progress;
        self.logged(app_id, async {
            let app = self.load(app_id)?;
            let current = std::fs::read_to_string(app.cv_yaml_path()).with_context(|| {
                format!("there is no CV yet at {}: generate one first", app.cv_yaml_path().display())
            })?;
            let jd_text = read_jd_text(&app).unwrap_or_default();
            let analysis = stored_analysis(&app).unwrap_or_else(|| Analysis {
                company: app.company.clone(),
                role: app.role.clone(),
                ..Default::default()
            });
            self.produce(app, &analysis, &jd_text, Task::Revise { instruction, current: &current }, notify).await
        })
        .await
    }

    pub async fn rerender(&self, app_id: i64) -> Result<RerenderOutcome> {
        let app = self.load(app_id)?;
        let yaml = std::fs::read_to_string(app.cv_yaml_path())
            .with_context(|| format!("reading {}", app.cv_yaml_path().display()))?;
        let cv = Cv::parse(&yaml)?;
        let (profile, _) = self.load_profile()?;
        let violations = guard(&profile, &cv);
        let render = render(&app.cv_yaml_path(), &app.dir).await?;
        Ok(RerenderOutcome { render, violations })
    }

    // ------------------------------------------------------------ steps

    async fn run_generate(&self, mut app: Application, input: JdInput, notify: Notify<'_>) -> Result<Application> {
        emit(notify, app.id, Stage::Fetching, "");
        let jd = ingest(input).await?;
        emit(notify, app.id, Stage::Analyzing, "");
        let analysis = analyze(self.llm, &jd.text, jd.source.as_deref()).await?;
        app = self.adopt_analysis(app, &analysis, &jd)?;
        self.start_brief(&app);
        self.produce(app, &analysis, &jd.text, Task::Fresh, notify).await
    }

    /// Start the job brief of `app` in the background when an insight provider is set and the
    /// application has none. Needs `jd.md` and the analysis, which both exist by now.
    fn start_brief(&self, app: &Application) {
        if let (Some(llm), None) = (&self.insight, &app.insight_json) {
            insight::spawn_compute(self.core, app.id, llm.clone());
        }
    }

    /// Tailor/revise, check, repair once, render, trim once, persist and report.
    async fn produce(
        &self,
        app: Application,
        analysis: &Analysis,
        jd_text: &str,
        task: Task<'_>,
        notify: Notify<'_>,
    ) -> Result<Application> {
        let (profile, profile_text) = self.load_profile()?;
        let system = tailor::system_prompt(&profile_text);
        let ctx = JdContext { analysis, jd_text };
        let cv_path = app.cv_yaml_path();

        emit(notify, app.id, Stage::Tailoring, "");
        let first = match &task {
            Task::Fresh => tailor::tailor(self.llm, &system, &ctx).await?,
            Task::Revise { instruction, current } => {
                tailor::revise(self.llm, &system, &ctx, current, instruction).await?
            }
        };

        // Check -> (repair once) -> render. One repair budget covers both fact violations and typst errors.
        let mut attempt = first;
        let mut repaired = false;
        let mut rendered = loop {
            emit(notify, app.id, Stage::Checking, "");
            let mut problems = validate(&profile, &attempt.yaml);
            if problems.is_empty() {
                std::fs::write(&cv_path, &attempt.yaml).with_context(|| format!("writing {}", cv_path.display()))?;
                emit(notify, app.id, Stage::Rendering, "");
                match render(&cv_path, &app.dir).await {
                    Ok(r) => break r,
                    Err(e) if !repaired && e.downcast_ref::<TypstError>().is_some() => {
                        problems = vec![Violation { path: "typst".into(), message: e.to_string() }];
                    }
                    Err(e) => return Err(e),
                }
            } else if repaired {
                // Keep the last attempt on disk so the user can fix it by hand and re-render.
                let _ = std::fs::write(&cv_path, &attempt.yaml);
                return Err(GuardFailed { violations: problems }.into());
            }
            repaired = true;
            emit(notify, app.id, Stage::Tailoring, &format!("repairing {} problem(s)", problems.len()));
            let fixed = tailor::repair(self.llm, &system, &ctx, &attempt.yaml, &format_violations(&problems)).await?;
            attempt = keep_lists(fixed, &attempt);
        };

        // Page budget: one trim round.
        let max_pages = self.max_pages();
        let mut warning = None;
        if rendered.page_count > max_pages {
            emit(
                notify,
                app.id,
                Stage::Tailoring,
                &format!("trimming from {} to {max_pages} page(s)", rendered.page_count),
            );
            match self.try_trim(&profile, &system, &ctx, &attempt, &rendered, max_pages, &app, notify).await {
                Ok(Some((trimmed, r))) => (attempt, rendered) = (trimmed, r),
                Ok(None) => {}
                Err(e) => warning = Some(format!("could not trim the CV: {e:#}")),
            }
        }
        if rendered.page_count > max_pages && warning.is_none() {
            warning = Some(format!("the CV takes {} pages (target {max_pages})", rendered.page_count));
        }

        // Persist and report.
        let mut app = self.load(app.id)?;
        match &task {
            Task::Fresh => app.gaps_json = Some(serde_json::to_string(&attempt.gaps)?),
            Task::Revise { .. } if !attempt.gaps.is_empty() => {
                app.gaps_json = Some(serde_json::to_string(&attempt.gaps)?)
            }
            Task::Revise { .. } => {}
        }
        self.core.store.save_application(&app)?;
        self.report(&app, &task, &attempt, warning.as_deref());
        emit(notify, app.id, Stage::Done, "");
        self.load(app.id)
    }

    /// Ask for a trimmed CV; `Some` when it passes the guard and renders on fewer pages.
    /// `cv.yaml` / the preview are left as they were whenever this returns `None` or an error.
    #[allow(clippy::too_many_arguments)]
    async fn try_trim(
        &self,
        profile: &Cv,
        system: &str,
        ctx: &JdContext<'_>,
        current: &TailorOutput,
        rendered: &RenderResult,
        max_pages: usize,
        app: &Application,
        notify: Notify<'_>,
    ) -> Result<Option<(TailorOutput, RenderResult)>> {
        let trimmed = tailor::trim(self.llm, system, ctx, &current.yaml, rendered.page_count, max_pages).await?;
        let problems = validate(profile, &trimmed.yaml);
        if !problems.is_empty() {
            bail!("the trimmed CV broke the fact rules:\n{}", format_violations(&problems));
        }
        emit(notify, app.id, Stage::Rendering, "");
        let cv_path = app.cv_yaml_path();
        std::fs::write(&cv_path, &trimmed.yaml)?;
        let outcome = render(&cv_path, &app.dir).await;
        if let Ok(r) = &outcome {
            if r.page_count < rendered.page_count {
                return Ok(Some((keep_lists(trimmed, current), r.clone())));
            }
        }
        // No improvement (or a render error): put the full CV and its preview back.
        std::fs::write(&cv_path, &current.yaml)?;
        let _ = render(&cv_path, &app.dir).await;
        outcome.map(|_| None)
    }

    fn report(&self, app: &Application, task: &Task<'_>, out: &TailorOutput, warning: Option<&str>) {
        let note = |kind: &str, msg: String| {
            let _ = self.core.store.add_event(app.id, kind, &msg);
        };
        if let Task::Revise { instruction, .. } = task {
            note("cv", format!("CV revised: {instruction}"));
        } else {
            note("cv", "CV generated".into());
        }
        for (label, items) in [("Emphasised", &out.emphasised), ("Omitted", &out.omitted), ("Gaps", &out.gaps)] {
            if !items.is_empty() {
                note("cv", format!("{label}: {}", items.join("; ")));
            }
        }
        if let Some(w) = warning {
            note("warning", w.to_string());
        }
    }

    // ------------------------------------------------------------ application bookkeeping

    fn load(&self, app_id: i64) -> Result<Application> {
        self.core.store.get_application(app_id)?.with_context(|| format!("application {app_id} no longer exists"))
    }

    /// The stored profile as typed data and as compact YAML (the `NoProfile` error when there is none).
    fn load_profile(&self) -> Result<(Cv, String)> {
        let p = self.core.require_profile()?;
        Ok((p.cv, p.yaml))
    }

    fn max_pages(&self) -> usize {
        self.core
            .store
            .kv_get(MAX_PAGES_KEY)
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n >= 1)
            .unwrap_or(DEFAULT_MAX_PAGES)
    }

    /// A Draft row the UI can show while the JD is still being fetched.
    fn create_placeholder(&self, input: &JdInput) -> Result<Application> {
        let (source, hint) = match input {
            JdInput::Url(u) => (Some(u.trim().to_string()), host_of(u).unwrap_or_else(|| u.trim().to_string())),
            JdInput::Html { url, .. } => (Some(url.clone()), host_of(url).unwrap_or_else(|| "Job page".into())),
            JdInput::File(p) => (
                Some(p.to_string_lossy().into_owned()),
                p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "JD file".into()),
            ),
            JdInput::Text(_) => (None, "Pasted JD".to_string()),
        };
        let slug = self.core.store.unique_slug("draft")?;
        let dir = self.core.paths.application_dir(&slug);
        self.core.store.create_application(&NewApplication {
            slug,
            company: hint,
            role: ANALYSING_ROLE.into(),
            source,
            apply_url: None,
            dir,
        })
    }

    /// Give the placeholder its real name: slug + folder, company, role, apply URL, `jd.md`.
    fn adopt_analysis(&self, mut app: Application, a: &Analysis, jd: &JdText) -> Result<Application> {
        let first_analysis = app.analysis_json.is_none();
        if first_analysis {
            let slug = self.core.store.unique_slug(&a.base_slug())?;
            let dir = self.core.paths.application_dir(&slug);
            if app.dir != dir {
                if app.dir.is_dir() && !dir.exists() {
                    std::fs::rename(&app.dir, &dir)
                        .with_context(|| format!("moving {} to {}", app.dir.display(), dir.display()))?;
                }
                self.core.store.with_conn(|c| {
                    c.execute(
                        "UPDATE applications SET slug = ?1, dir = ?2 WHERE id = ?3",
                        params![slug, dir.to_string_lossy(), app.id],
                    )?;
                    Ok(())
                })?;
                app.slug = slug;
                app.dir = dir;
            }
        }
        std::fs::create_dir_all(&app.dir).with_context(|| format!("creating {}", app.dir.display()))?;

        if !a.company.is_empty() {
            app.company = a.company.clone();
        }
        app.role = if a.role.is_empty() { "Role not stated".into() } else { a.role.clone() };
        if jd.source.is_some() {
            app.source = jd.source.clone();
        }
        app.apply_url = valid_http_url(&a.apply_url).or_else(|| jd.url.clone());
        app.analysis_json = Some(serde_json::to_string_pretty(a)?);
        write_jd(&app, &jd.text)?;
        self.core.store.save_application(&app)?;
        self.load(app.id)
    }

    fn save_analysis(&self, app: &Application, a: &Analysis) -> Result<()> {
        let mut app = app.clone();
        app.analysis_json = Some(serde_json::to_string_pretty(a)?);
        self.core.store.save_application(&app)
    }

    /// Record a failure on the application's timeline, then pass it on.
    async fn logged<T>(&self, app_id: i64, work: impl Future<Output = Result<T>>) -> Result<T> {
        match work.await {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::warn!(app = app_id, "pipeline: failed: {e:#}");
                let _ = self.core.store.add_event(app_id, "error", &format!("{e:#}"));
                // A row that never got analysed must not keep saying "Analysing...". A row that
                // was named from the start (a promoted lead) keeps its role.
                if let Ok(Some(mut app)) = self.core.store.get_application(app_id) {
                    if app.analysis_json.is_none() && app.role == ANALYSING_ROLE {
                        app.role = "Job description not loaded".into();
                        let _ = self.core.store.save_application(&app);
                    }
                }
                Err(e)
            }
        }
    }
}

enum Task<'a> {
    Fresh,
    Revise { instruction: &'a str, current: &'a str },
}

fn emit(notify: Notify<'_>, app_id: i64, stage: Stage, detail: &str) {
    tracing::info!(app = app_id, "pipeline: {}{}{detail}", stage.label(), if detail.is_empty() { "" } else { ": " });
    notify(Progress { app_id, stage, detail: detail.to_string() });
}

/// Parse errors and fact violations of a model-written CV, as one list.
fn validate(profile: &Cv, yaml: &str) -> Vec<Violation> {
    match Cv::parse(yaml) {
        Ok(cv) => guard(profile, &cv),
        Err(e) => vec![Violation { path: "yaml".into(), message: format!("{e:#}") }],
    }
}

/// A repair/trim answer replaces the CV; empty report lists fall back to the earlier ones.
fn keep_lists(mut new: TailorOutput, old: &TailorOutput) -> TailorOutput {
    if new.emphasised.is_empty() {
        new.emphasised = old.emphasised.clone();
    }
    if new.omitted.is_empty() {
        new.omitted = old.omitted.clone();
    }
    if new.gaps.is_empty() {
        new.gaps = old.gaps.clone();
    }
    new
}

fn stored_analysis(app: &Application) -> Option<Analysis> {
    serde_json::from_str(app.analysis_json.as_deref()?).ok()
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url.trim()).ok()?.host_str().map(|h| h.trim_start_matches("www.").to_string())
}

/// `jd.md`: `# company — role`, the source, a blank line, then the JD text.
pub(crate) fn write_jd(app: &Application, text: &str) -> Result<()> {
    let mut body = format!("# {} — {}\n", app.company, app.role);
    if let Some(source) = &app.source {
        body.push_str(&format!("Source: {source}\n"));
    }
    body.push('\n');
    body.push_str(text.trim());
    body.push('\n');
    let path = app.jd_path();
    std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

/// The JD text from `jd.md` without the header written by [`write_jd`].
pub fn read_jd_text(app: &Application) -> Result<String> {
    let path = app.jd_path();
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!("no job description stored at {}: generate from a link or file first", path.display())
    })?;
    Ok(strip_jd_header(&raw).to_string())
}

pub(crate) fn strip_jd_header(raw: &str) -> &str {
    if !raw.starts_with("# ") {
        return raw.trim();
    }
    let mut rest = raw.split_once('\n').map_or("", |(_, r)| r);
    if rest.starts_with("Source:") {
        rest = rest.split_once('\n').map_or("", |(_, r)| r);
    }
    rest.trim()
}

#[cfg(test)]
mod tests;
