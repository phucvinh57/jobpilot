//! Shared app state and the root view.

pub mod jobs;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;

use anyhow::Result;
use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::browser::fetch::generate_with_browser;
use jobpilot_core::pipeline::{self, JdInput, Progress};
use jobpilot_core::store::{Application, NewApplication, Status};
use jobpilot_core::{mail, runtime, Core};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::detail::Detail;
use crate::settings::SettingsView;
use crate::sidebar::Sidebar;
use jobs::{generate_plan, progress_label, GeneratePlan};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Detail,
    Settings,
}

/// State shared by every view: wrap in `Entity<AppState>` and `cx.observe` it to re-render.
/// Views call `core` for data; slow work goes through `jobpilot_core::runtime::spawn`.
pub struct AppState {
    pub core: Arc<Core>,
    pub applications: Vec<Application>,
    pub selected: Option<i64>,
    pub screen: Screen,
    /// Running pipeline work (generate / revise): application id -> current progress line.
    jobs: HashMap<i64, String>,
    /// Last failure per application, shown inline until its next job starts or it is dismissed.
    errors: HashMap<i64, String>,
    /// Failure of an input that never became a Draft (bad link, no AI provider...).
    pub input_error: Option<String>,
    /// What this session fed to `generate` for each Draft, so "Generate" can retry it.
    inputs: HashMap<i64, JdInput>,
    /// Bumped when a job rewrote an application's CV files, so the CV tab reloads them.
    cv_epochs: HashMap<i64, u64>,
}

impl AppState {
    pub fn new(core: Arc<Core>) -> Self {
        let mut state = Self {
            core,
            applications: Vec::new(),
            selected: None,
            screen: Screen::Detail,
            jobs: HashMap::new(),
            errors: HashMap::new(),
            input_error: None,
            inputs: HashMap::new(),
            cv_epochs: HashMap::new(),
        };
        state.load();
        state.selected = state.applications.first().map(|a| a.id);
        state
    }

    fn load(&mut self) {
        match self.core.store.list_applications() {
            Ok(apps) => self.applications = apps,
            Err(e) => eprintln!("failed to load applications: {e:#}"),
        }
    }

    /// Re-read the applications from the store (called after generate/submit/sync).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load();
        cx.notify();
    }

    pub fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.screen = Screen::Detail;
        cx.notify();
    }

    pub fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        self.screen = screen;
        cx.notify();
    }

    pub fn selected_app(&self) -> Option<&Application> {
        let id = self.selected?;
        self.applications.iter().find(|a| a.id == id)
    }

    // ------------------------------------------------------------ pipeline jobs

    /// Is generate/revise running for this application?
    pub fn is_busy(&self, id: i64) -> bool {
        self.jobs.contains_key(&id)
    }

    /// The progress line of the running job, if any.
    pub fn job_label(&self, id: i64) -> Option<&str> {
        self.jobs.get(&id).map(String::as_str)
    }

    /// The last failure of this application's jobs.
    pub fn error(&self, id: i64) -> Option<&str> {
        self.errors.get(&id).map(String::as_str)
    }

    pub fn dismiss_error(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.errors.remove(&id).is_some() {
            cx.notify();
        }
    }

    /// Counter that changes whenever a job rewrote this application's `cv.yaml` / previews.
    pub fn cv_epoch(&self, id: i64) -> u64 {
        self.cv_epochs.get(&id).copied().unwrap_or(0)
    }

    /// Sidebar: create a Draft from `input` and run the whole pipeline. The Draft appears (and gets
    /// selected) with the first progress event.
    pub fn submit_input(&mut self, input: JdInput, cx: &mut Context<Self>) {
        self.input_error = None;
        let run_input = input.clone();
        self.start_job(
            None,
            Some(input),
            move |core, tx| async move {
                generate_with_browser(&core, None, run_input, move |p| {
                    let _ = tx.send(p);
                })
                .await
            },
            cx,
        );
    }

    /// Primary button on a Draft without a CV: re-run generation for this application.
    pub fn run_generate(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.is_busy(id) {
            return;
        }
        let Some(app) = self.applications.iter().find(|a| a.id == id).cloned() else { return };
        match generate_plan(&app, self.inputs.get(&id)) {
            GeneratePlan::Regenerate => self.start_job(
                Some(id),
                None,
                move |core, tx| async move {
                    pipeline::regenerate(&core, id, move |p| {
                        let _ = tx.send(p);
                    })
                    .await
                },
                cx,
            ),
            GeneratePlan::Reingest(input) => {
                let run_input = input.clone();
                self.start_job(
                    Some(id),
                    Some(input),
                    move |core, tx| async move {
                        generate_with_browser(&core, Some(id), run_input, move |p| {
                            let _ = tx.send(p);
                        })
                        .await
                    },
                    cx,
                )
            }
            GeneratePlan::Unavailable(message) => {
                self.errors.insert(id, message);
                cx.notify();
            }
        }
    }

    /// "Ask AI": targeted revision of the current CV.
    pub fn revise(&mut self, id: i64, instruction: String, cx: &mut Context<Self>) {
        if self.is_busy(id) || instruction.trim().is_empty() {
            return;
        }
        self.start_job(
            Some(id),
            None,
            move |core, tx| async move {
                pipeline::revise(&core, id, &instruction, move |p| {
                    let _ = tx.send(p);
                })
                .await
            },
            cx,
        );
    }

    /// Run `run` on the global runtime; its progress events arrive over a channel that the UI task
    /// drains. `existing`: the application this job works on; `None` = a new Draft that gets
    /// selected when its first event arrives.
    fn start_job<F, Fut>(
        &mut self,
        existing: Option<i64>,
        retry_input: Option<JdInput>,
        run: F,
        cx: &mut Context<Self>,
    ) where
        F: FnOnce(Arc<Core>, UnboundedSender<Progress>) -> Fut,
        Fut: Future<Output = Result<Application>> + Send + 'static,
    {
        if let Some(id) = existing {
            self.errors.remove(&id);
            self.jobs.insert(id, "Starting".into());
            if let Some(input) = &retry_input {
                self.inputs.insert(id, input.clone());
            }
            cx.notify();
        }

        let (tx, mut rx) = unbounded_channel::<Progress>();
        let done = runtime::spawn(run(self.core.clone(), tx));
        let select_new = existing.is_none();
        cx.spawn(async move |this, cx| {
            let mut app_id = existing;
            let mut retry_input = retry_input;
            let mut first = true;
            while let Some(progress) = rx.recv().await {
                app_id = Some(progress.app_id);
                let input = retry_input.take();
                let announce = std::mem::take(&mut first);
                let alive = this.update(cx, |state, cx| state.on_progress(&progress, input, select_new && announce, cx));
                if alive.is_err() {
                    return;
                }
            }
            let result = match done.await {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("the task was cancelled")),
            };
            this.update(cx, |state, cx| state.finish_job(app_id, result, cx)).ok();
        })
        .detach();
    }

    fn on_progress(&mut self, p: &Progress, input: Option<JdInput>, select: bool, cx: &mut Context<Self>) {
        let new_row = !self.applications.iter().any(|a| a.id == p.app_id);
        self.errors.remove(&p.app_id);
        self.jobs.insert(p.app_id, progress_label(p));
        if let Some(input) = input {
            self.inputs.insert(p.app_id, input);
        }
        // The Draft row appears with the first event; the analysis renames it (company, role).
        self.load();
        if select && new_row {
            self.selected = Some(p.app_id);
            self.screen = Screen::Detail;
        }
        cx.notify();
    }

    fn finish_job(&mut self, app_id: Option<i64>, result: Result<Application>, cx: &mut Context<Self>) {
        match app_id {
            Some(id) => {
                self.jobs.remove(&id);
                *self.cv_epochs.entry(id).or_insert(0) += 1;
                match result {
                    Ok(_) => {
                        self.errors.remove(&id);
                        self.inputs.remove(&id);
                    }
                    Err(e) => {
                        self.errors.insert(id, format!("{e:#}"));
                    }
                }
            }
            // Refused before a Draft existed (bad link, no AI provider).
            None => {
                if let Err(e) = result {
                    self.input_error = Some(format!("{e:#}"));
                }
            }
        }
        self.load();
        cx.notify();
    }

    /// "Approve CV": Draft -> CvReady.
    pub fn approve_cv(&mut self, id: i64, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let rx = runtime::spawn(async move { core.store.set_status(id, Status::CvReady, "CV approved") });
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |state, cx| {
                match result {
                    Ok(Ok(())) => {
                        state.errors.remove(&id);
                    }
                    Ok(Err(e)) => {
                        state.errors.insert(id, format!("{e:#}"));
                    }
                    Err(_) => {
                        state.errors.insert(id, "the task was cancelled".into());
                    }
                }
                state.reload(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Reload the list whenever the Gmail sync touched the data (new mail, a status change).
    pub fn watch_mail(&mut self, cx: &mut Context<Self>) {
        let mut changes = mail::changes();
        cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                if this.update(cx, |state, cx| state.reload(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }
}

/// Colour of the status dot.
pub fn status_color(status: Status, theme: &Theme) -> Hsla {
    match status {
        Status::Draft => theme.muted_foreground,
        Status::CvReady => theme.info,
        Status::Submitting => theme.warning,
        Status::Applied => theme.primary,
        Status::Interview => theme.success,
        Status::Offer => theme.success,
        Status::Rejected | Status::Failed => theme.danger,
    }
}

/// Root view: sidebar on the left, detail or settings on the right.
pub struct JobPilot {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    detail: Entity<Detail>,
    settings: Entity<SettingsView>,
}

impl JobPilot {
    pub fn new(core: Arc<Core>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| AppState::new(core));
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        state.update(cx, |state, cx| state.watch_mail(cx));
        Self {
            sidebar: cx.new(|cx| Sidebar::new(state.clone(), window, cx)),
            detail: cx.new(|cx| Detail::new(state.clone(), window, cx)),
            settings: cx.new(|cx| SettingsView::new(state.clone(), cx)),
            state,
        }
    }
}

impl Render for JobPilot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let main = match self.state.read(cx).screen {
            Screen::Detail => self.detail.clone().into_any_element(),
            Screen::Settings => self.settings.clone().into_any_element(),
        };
        let theme = cx.theme();
        let highlight = theme.accent;
        div()
            .h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            // Drop a .md/.txt/.html/.pdf file anywhere on the window to start a Draft from it.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(highlight))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let result = jobs::file_input(paths.paths());
                this.state.update(cx, |state, cx| match result {
                    Ok(input) => state.submit_input(input, cx),
                    Err(message) => {
                        state.input_error = Some(message);
                        cx.notify();
                    }
                });
            }))
            .child(self.sidebar.clone())
            .child(div().flex_1().min_w_0().h_full().child(main))
    }
}

/// Insert a few sample applications when the store is empty (`JOBPILOT_DEMO=1`).
pub fn seed_demo(core: &Core) -> Result<()> {
    if !core.store.list_applications()?.is_empty() {
        return Ok(());
    }
    let samples = [
        ("acme-backend-engineer", "Acme", "Backend Engineer", Status::CvReady),
        ("foo-platform-engineer", "Foo", "Platform Engineer", Status::Applied),
        ("bar-devops-engineer", "Bar", "DevOps Engineer", Status::Interview),
        ("baz-sre", "Baz", "Site Reliability Engineer", Status::Draft),
    ];
    for (slug, company, role, status) in samples {
        let app = core.store.create_application(&NewApplication {
            slug: slug.into(),
            company: company.into(),
            role: role.into(),
            source: Some(format!("https://{slug}.example.com/jobs/1")),
            dir: core.paths.application_dir(slug),
            ..Default::default()
        })?;
        core.store.force_status(app.id, status, "demo data")?;
    }
    Ok(())
}
