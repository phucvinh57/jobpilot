//! Shared app state and the root view.

pub mod jobs;

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::Result;
use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::browser::fetch::generate_with_browser;
use jobpilot_core::chrono::{Duration, Utc};
use jobpilot_core::health::{self, Health, Problem};
use jobpilot_core::discover::dedup::dedup_key;
use jobpilot_core::discover::lead_dir;
use jobpilot_core::discover::score::{Criteria, Criterion, Scored};
use jobpilot_core::discover::{RunSummary, LAST_RUN_KEY};
use jobpilot_core::pipeline::insight::{CompanyFacts, Interview, Skills};
use jobpilot_core::prefs::Weights;
use jobpilot_core::pipeline::{self, Insight, JdInput, Progress};
use jobpilot_core::store::{Application, LeadState, NewApplication, NewLead, NewSource, SourcePurpose, Status};
use jobpilot_core::{mail, runtime, Core};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

use crate::detail::Detail;
use crate::pipeline::PipelineView;
use crate::queue::QueueView;
use crate::settings::SettingsView;
use crate::sidebar::Sidebar;
use crate::trends::TrendsView;
use crate::ui;
use jobs::{generate_plan, progress_label, GeneratePlan};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The selected application.
    Detail,
    Settings,
    /// Today (the Job list): found jobs sorted by match, and the box that turns a pasted link into one (P12).
    Jobs,
    /// The kanban board of every application, and the latest activity.
    Pipeline,
    /// Market trends and reports (P13).
    Trends,
}

impl Screen {
    /// A number per screen, to key the fade-in of the main pane.
    fn index(self) -> u64 {
        match self {
            Screen::Detail => 0,
            Screen::Settings => 1,
            Screen::Jobs => 2,
            Screen::Trends => 3,
            Screen::Pipeline => 4,
        }
    }
}

/// Length of the fade-in when the main pane or the open lead changes.
pub const FADE: StdDuration = StdDuration::from_millis(150);

/// What to run again when a job failed and the user presses Retry.
#[derive(Clone, Debug)]
enum Job {
    Generate,
    Revise(String),
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
    /// What this session fed to `generate` for each Draft, so "Generate" can retry it.
    inputs: HashMap<i64, JdInput>,
    /// Bumped when a job rewrote an application's CV files, so the CV tab reloads them.
    cv_epochs: HashMap<i64, u64>,
    /// The last job started per application, for Retry.
    last_jobs: HashMap<i64, Job>,
    /// Prerequisites (AI provider, typst, Chrome); `None` until the first check finished.
    pub health: Option<Health>,
    /// Settings section to scroll to the next time Settings is shown.
    pub settings_focus: Option<&'static str>,
    /// The lead open in the Job list (P12).
    pub queue_selected: Option<i64>,
    /// Bumped whenever the Job list is shown, so it reads its leads again.
    pub queue_epoch: u64,
    /// Bumped whenever the Pipeline is shown, so it reads the latest activity again.
    pub pipeline_epoch: u64,
    /// New leads in the Job list (the badge on the sidebar entry), kept current by
    /// [`refresh_lead_count`](Self::refresh_lead_count) whichever screen is shown.
    pub lead_count: usize,
    /// What waits for the user on Today: job sources paused at a login / captcha / block page, plus
    /// failed "Prepare CV" runs (the badge on the Today entry). Read together with `lead_count`.
    pub needs_you: usize,
    /// Counts started so far: only the answer of the latest one is shown.
    lead_count_reads: u64,
    /// A file dropped on the window (or why it was refused), waiting for the Job list to turn it into
    /// a lead. The Job list reads it with `.take()`, so it is used once.
    pub dropped: Option<Result<JdInput, String>>,
    /// Job-search preferences were saved at least once (the setup banner points to Settings until then).
    pub prefs_saved: bool,
}

impl AppState {
    pub fn new(core: Arc<Core>) -> Self {
        let mut state = Self {
            core,
            applications: Vec::new(),
            selected: None,
            screen: Screen::Jobs,
            jobs: HashMap::new(),
            errors: HashMap::new(),
            inputs: HashMap::new(),
            cv_epochs: HashMap::new(),
            last_jobs: HashMap::new(),
            health: None,
            settings_focus: None,
            queue_selected: None,
            queue_epoch: 0,
            pipeline_epoch: 0,
            lead_count: 0,
            needs_you: 0,
            lead_count_reads: 0,
            dropped: None,
            prefs_saved: false,
        };
        state.load();
        state.selected = state.applications.first().map(|a| a.id);
        state
    }

    fn load(&mut self) {
        match self.core.store.list_applications() {
            Ok(apps) => self.applications = apps,
            Err(e) => tracing::error!("failed to load applications: {e:#}"),
        }
        match self.core.store.has_preferences() {
            Ok(saved) => self.prefs_saved = saved,
            Err(e) => tracing::error!("failed to read the preferences: {e:#}"),
        }
    }

    /// Re-read the applications from the store (called after generate/submit/sync), and the
    /// sidebar badge: a deleted application, a mail change or an approval can move leads in or out
    /// of the Job list.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load();
        self.refresh_lead_count(cx);
        cx.notify();
    }

    pub fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.screen = Screen::Detail;
        cx.notify();
    }

    pub fn show(&mut self, screen: Screen, cx: &mut Context<Self>) {
        let leaving_settings = self.screen == Screen::Settings && screen != Screen::Settings;
        match screen {
            Screen::Jobs => self.queue_epoch += 1,
            Screen::Pipeline => self.pipeline_epoch += 1,
            _ => {}
        }
        self.screen = screen;
        if leaving_settings {
            // A provider, Chrome or typst may have been set up meanwhile.
            self.refresh_health(cx);
        }
        cx.notify();
    }

    /// Show Settings, scrolled to `section` (`"AI providers"`, `"Browser sessions"`, `"Gmail"`,
    /// `"Profile & preferences"`).
    pub fn open_settings(&mut self, section: Option<&'static str>, cx: &mut Context<Self>) {
        self.settings_focus = section;
        self.show(Screen::Settings, cx);
    }

    /// A file was dropped on the window: show the Job list, which turns it into a lead (see
    /// [`dropped`](Self::dropped)).
    pub fn drop_input(&mut self, input: Result<JdInput, String>, cx: &mut Context<Self>) {
        self.dropped = Some(input);
        self.show(Screen::Jobs, cx);
    }

    /// Re-check the prerequisites in the background (also at startup).
    pub fn refresh_health(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let rx = runtime::spawn(async move { core.health().await });
        cx.spawn(async move |this, cx| {
            if let Ok(health) = rx.await {
                this.update(cx, |state, cx| {
                    if state.health.as_ref() != Some(&health) {
                        state.health = Some(health);
                        cx.notify();
                    }
                })
                .ok();
            }
        })
        .detach();
    }

    /// Read the Today badge again: the number of leads in the Job list and what needs the user
    /// (see [`needs_you`](Self::needs_you)), off the UI thread and without the Job list screen:
    /// call it once at startup and after anything that adds, prepares or dismisses leads, or pauses
    /// or resumes a source (a search, the daily run, an add by hand). An older answer that arrives
    /// after a newer one is dropped.
    pub fn refresh_lead_count(&mut self, cx: &mut Context<Self>) {
        self.lead_count_reads += 1;
        let read = self.lead_count_reads;
        let core = self.core.clone();
        let rx = runtime::spawn(async move { (core.new_lead_count(), needs_you_count(&core)) });
        cx.spawn(async move |this, cx| {
            let Ok((leads, needs_you)) = rx.await else { return };
            this.update(cx, |state, cx| {
                if !is_latest_read(read, state.lead_count_reads) {
                    return;
                }
                match leads {
                    Ok(count) => state.lead_count = count,
                    Err(e) => tracing::warn!("counting the new leads failed: {e:#}"),
                }
                match needs_you {
                    Ok(count) => state.needs_you = count,
                    Err(e) => tracing::warn!("counting what needs the user failed: {e:#}"),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Prerequisites that block the main flow (empty until the first check finished).
    pub fn blockers(&self) -> Vec<Problem> {
        self.health.as_ref().map(Health::blockers).unwrap_or_default()
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

    /// Retry: run the failed job again (Generate, or the same "Ask AI" instruction).
    pub fn retry(&mut self, id: i64, cx: &mut Context<Self>) {
        match self.last_jobs.get(&id).cloned() {
            Some(Job::Revise(instruction)) => self.revise(id, instruction, cx),
            _ => self.run_generate(id, cx),
        }
    }

    /// Counter that changes whenever a job rewrote this application's `cv.yaml` / previews.
    pub fn cv_epoch(&self, id: i64) -> u64 {
        self.cv_epochs.get(&id).copied().unwrap_or(0)
    }

    /// Primary button on a Draft without a CV: re-run generation for this application.
    pub fn run_generate(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.is_busy(id) {
            return;
        }
        self.last_jobs.insert(id, Job::Generate);
        let Some(app) = self.applications.iter().find(|a| a.id == id).cloned() else { return };
        match generate_plan(&app, self.inputs.get(&id)) {
            GeneratePlan::Regenerate => self.start_job(
                id,
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
                    id,
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
        self.last_jobs.insert(id, Job::Revise(instruction.clone()));
        self.start_job(
            id,
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

    /// Run `run` on the global runtime for application `id`; its progress events arrive over a channel
    /// that the UI task drains. `retry_input`: what the job was fed, kept so "Generate" can run again.
    fn start_job<F, Fut>(&mut self, id: i64, retry_input: Option<JdInput>, run: F, cx: &mut Context<Self>)
    where
        F: FnOnce(Arc<Core>, UnboundedSender<Progress>) -> Fut,
        Fut: Future<Output = Result<Application>> + Send + 'static,
    {
        self.errors.remove(&id);
        self.jobs.insert(id, "Starting".into());
        if let Some(input) = retry_input {
            self.inputs.insert(id, input);
        }
        cx.notify();

        let (tx, mut rx) = unbounded_channel::<Progress>();
        let done = runtime::spawn(run(self.core.clone(), tx));
        cx.spawn(async move |this, cx| {
            while let Some(progress) = rx.recv().await {
                if this.update(cx, |state, cx| state.on_progress(&progress, cx)).is_err() {
                    return;
                }
            }
            let result = match done.await {
                Ok(result) => result,
                Err(_) => Err(anyhow::anyhow!("the task was cancelled")),
            };
            this.update(cx, |state, cx| state.finish_job(id, result, cx)).ok();
        })
        .detach();
    }

    fn on_progress(&mut self, p: &Progress, cx: &mut Context<Self>) {
        self.errors.remove(&p.app_id);
        self.jobs.insert(p.app_id, progress_label(p));
        // The analysis renames the row (company, role).
        self.load();
        cx.notify();
    }

    fn finish_job(&mut self, id: i64, result: Result<Application>, cx: &mut Context<Self>) {
        self.jobs.remove(&id);
        *self.cv_epochs.entry(id).or_insert(0) += 1;
        match result {
            Ok(_) => {
                self.errors.remove(&id);
                self.inputs.remove(&id);
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.last_jobs.entry(id).or_insert(Job::Generate);
                self.needs_recheck(&message, cx);
                self.errors.insert(id, message);
            }
        }
        self.load();
        cx.notify();
    }

    /// A failure that says a prerequisite is missing: re-run the health check so the banner is right.
    pub fn needs_recheck(&mut self, message: &str, cx: &mut Context<Self>) {
        if matches!(health::classify(message), Problem::NoProvider | Problem::NoTypst | Problem::NoChrome) {
            self.refresh_health(cx);
        }
    }

    /// "Next job": show the Job list with its best lead open (after a submit or a dismissal).
    pub fn next_job(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        let rx = runtime::spawn(async move { core.queue_next() });
        cx.spawn(async move |this, cx| {
            let next = match rx.await {
                Ok(Ok(lead)) => lead.map(|l| l.id),
                Ok(Err(e)) => {
                    tracing::warn!("reading the next lead failed: {e:#}");
                    None
                }
                Err(_) => None,
            };
            this.update(cx, |state, cx| {
                state.queue_selected = next;
                state.show(Screen::Jobs, cx);
            })
            .ok();
        })
        .detach();
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

/// Sources paused for the user plus failed "Prepare CV" runs: the "needs you" count of Today. A
/// database read: run it off the UI thread.
fn needs_you_count(core: &Core) -> Result<usize> {
    Ok(core.discover_waiting()?.len() + core.prepare_failures()?.len())
}

/// The answer of read number `read` is the one to show only while it is the latest started.
pub(crate) fn is_latest_read(read: u64, latest: u64) -> bool {
    read == latest
}

/// Root view: sidebar on the left, the current [`Screen`] on the right (fading in when it changes).
pub struct JobPilot {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    detail: Entity<Detail>,
    settings: Entity<SettingsView>,
    queue: Entity<QueueView>,
    pipeline: Entity<PipelineView>,
    trends: Entity<TrendsView>,
}

impl JobPilot {
    pub fn new(core: Arc<Core>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|_| AppState::new(core));
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        state.update(cx, |state, cx| {
            state.watch_mail(cx);
            state.refresh_health(cx);
            state.refresh_lead_count(cx);
        });
        Self {
            sidebar: cx.new(|cx| Sidebar::new(state.clone(), window, cx)),
            detail: cx.new(|cx| Detail::new(state.clone(), window, cx)),
            settings: cx.new(|cx| SettingsView::new(state.clone(), cx)),
            queue: cx.new(|cx| QueueView::new(state.clone(), window, cx)),
            pipeline: cx.new(|cx| PipelineView::new(state.clone(), cx)),
            trends: cx.new(|cx| TrendsView::new(state.clone(), window, cx)),
            state,
        }
    }
}

impl Render for JobPilot {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.state.read(cx).screen;
        let main = match screen {
            Screen::Detail => self.detail.clone().into_any_element(),
            Screen::Settings => self.settings.clone().into_any_element(),
            Screen::Jobs => self.queue.clone().into_any_element(),
            Screen::Pipeline => self.pipeline.clone().into_any_element(),
            Screen::Trends => self.trends.clone().into_any_element(),
        };
        let p = ui::palette(cx);
        let highlight = p.accent_soft;
        div()
            .h_flex()
            .size_full()
            .bg(p.bg)
            .text_color(p.ink)
            // Drop a .md/.txt/.html/.pdf file anywhere on the window to add it to the Job list as a lead.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(highlight))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let result = jobs::file_input(paths.paths());
                this.state.update(cx, |state, cx| state.drop_input(result, cx));
            }))
            .child(self.sidebar.clone())
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .children(crate::problem::setup_banner(&self.state, cx))
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .child(main)
                            // A short fade-in whenever another screen is shown.
                            .with_animation(
                                ElementId::NamedInteger("screen".into(), screen.index()),
                                Animation::new(FADE).with_easing(ease_in_out),
                                |this, delta| this.opacity(delta),
                            ),
                    ),
            )
    }
}

/// Insert a few sample applications when the store is empty (`JOBPILOT_DEMO=1`).
pub fn seed_demo(core: &Core) -> Result<()> {
    jobpilot_core::prefs::seed_demo(core)?;
    crate::trends::seed_demo(core)?;
    seed_demo_applications(core)?;
    seed_demo_queue(core)
}

fn seed_demo_applications(core: &Core) -> Result<()> {
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
        if slug == "acme-backend-engineer" {
            let json = serde_json::to_string(&demo_insight(slug))?;
            core.store.set_insight_json(app.id, Some(&json))?;
        }
    }
    Ok(())
}

/// Job sources and scored leads for the queue (skipped when the queue has something already).
fn seed_demo_queue(core: &Core) -> Result<()> {
    if !core.store.list_sources(SourcePurpose::Jobs)?.is_empty() || !core.store.list_leads(LeadState::New)?.is_empty() {
        return Ok(());
    }
    let source = |kind: &str, name: &str, url: &str| {
        core.store.add_source(&NewSource {
            kind: kind.into(),
            name: name.into(),
            url: url.into(),
            purpose: SourcePurpose::Jobs,
            session_id: None,
        })
    };
    let greenhouse = source("greenhouse", "Acme careers", "https://boards.greenhouse.io/acme")?;
    let lever = source("lever", "Foo jobs", "https://jobs.lever.co/foo")?;
    let hn = source("hn_hiring", "HN: Who is hiring", "https://news.ycombinator.com/item?id=1")?;
    core.store.set_source_run(greenhouse.id, "ok")?;
    core.store.set_source_run(lever.id, "ok")?;
    core.store.set_source_status(hn.id, "needs_human")?;

    // (source, company, role, location, [skills, domain, location, salary, seniority])
    type Row = (u32, bool, &'static str);
    let jobs: [(i64, &str, &str, &str, [Row; 5]); 6] = [
        (greenhouse.id, "Acme", "Senior Backend Engineer", "Singapore", [
            (92, true, "Rust, PostgreSQL and distributed systems are all in your profile."),
            (80, true, "Payments infrastructure is close to your fintech interest."),
            (70, true, "Hybrid in Singapore, one of your cities."),
            (0, false, "No salary stated."),
            (90, true, "Senior, as you look for."),
        ]),
        (lever.id, "Foo", "Platform Engineer", "Remote (APAC)", [
            (78, true, "Kubernetes and Linux match; Terraform is not in your profile."),
            (55, true, "Developer tools, not a listed domain."),
            (100, true, "Fully remote."),
            (60, true, "Pay range is near your minimum."),
            (80, true, "Mid to senior."),
        ]),
        (greenhouse.id, "Acme", "Site Reliability Engineer", "Ho Chi Minh City, Vietnam", [
            (70, true, "Linux, Nginx and observability match."),
            (80, true, "Same company as the payments role."),
            (100, true, "On-site in Ho Chi Minh City, where you live."),
            (0, false, "No salary stated."),
            (60, true, "Mid-level title."),
        ]),
        (hn.id, "Quark Labs", "Staff Systems Engineer", "Berlin, Germany", [
            (60, true, "C++ and Rust asked; embedded work is not in your profile."),
            (30, true, "Robotics is an avoided domain."),
            (20, true, "On-site in Berlin."),
            (0, false, "No salary stated."),
            (40, true, "Staff level is above your experience."),
        ]),
        (lever.id, "Bar", "DevOps Engineer", "Remote", [
            (66, true, "Docker, CI/CD and AWS match."),
            (50, true, "E-commerce is neutral."),
            (100, true, "Fully remote."),
            (0, false, "No salary stated."),
            (65, true, "Mid-level."),
        ]),
        (hn.id, "Nimbus", "Backend Developer", "", [
            (0, false, "The posting is too short to judge."),
            (0, false, "No domain stated."),
            (0, false, "No location stated."),
            (0, false, "No salary stated."),
            (0, false, "No level stated."),
        ]),
    ];
    let weights = Weights::default();
    for (i, (source_id, company, role, location, rows)) in jobs.into_iter().enumerate() {
        let criterion = |(score, known, reason): Row| Criterion { score, known, reason: reason.into() };
        let [skills, domain, loc, salary, seniority] = rows.map(criterion);
        let criteria = Criteria { skills, domain, location: loc, salary, seniority };
        let scored = Scored { total: criteria.weighted(&weights), criteria, weights };
        let slug = format!("{}-{i}", company.to_lowercase().replace(' ', "-"));
        let url = format!("https://jobs.example.com/{slug}");
        let lead = core.store.add_lead(&NewLead {
            source_id: Some(source_id),
            dedup_key: dedup_key(company, role),
            url: url.clone(),
            company: company.into(),
            role: role.into(),
            location: location.into(),
            score: scored.total,
            score_json: scored.total.map(|_| scored.to_json()).transpose()?,
            insight_json: (i % 3 == 0).then(|| serde_json::to_string(&demo_insight(&slug))).transpose()?,
            ..NewLead::default()
        })?;
        let dir = lead_dir(&core.paths, lead.id);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("jd.md");
        let text = format!(
            "# {company} \u{2014} {role}\nSource: {url}\n\nWe are hiring a {role} at {company}. You will design, build and run backend services, \
             own their reliability, and work with a small team. Requirements: Linux, networking, a systems language and clear English.\n"
        );
        std::fs::write(&path, text)?;
        core.store.set_lead_jd_path(lead.id, &path)?;
    }
    let summary = RunSummary {
        run: "demo".into(),
        started_at: (Utc::now() - Duration::minutes(12)).to_rfc3339(),
        finished_at: (Utc::now() - Duration::minutes(10)).to_rfc3339(),
        saved: 6,
        waiting: 1,
        ..RunSummary::default()
    };
    core.store.kv_set(LAST_RUN_KEY, &serde_json::to_string(&summary)?)?;
    Ok(())
}

/// A company brief for the demo data: some facts found, some not.
fn demo_insight(slug: &str) -> Insight {
    let strings = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    Insight {
        domain: "B2B payments infrastructure".into(),
        business_summary: "Sells a payments API to online marketplaces, which use it to pay out sellers in 40 countries.".into(),
        company: CompanyFacts {
            size: "200-500 employees".into(),
            stage: "Series C".into(),
            hq: "Singapore".into(),
            founded: "2016".into(),
            ..CompanyFacts::default()
        },
        skills: Skills {
            required: strings(&["Rust", "PostgreSQL", "Distributed systems", "Linux"]),
            preferred: strings(&["Kafka", "Kubernetes"]),
        },
        interview: Interview {
            stages: strings(&["Recruiter call", "Technical screen", "System design", "Team interview", "Offer"]),
            notes: "The whole process takes about three weeks.".into(),
            found: true,
        },
        sources: vec![
            format!("https://{slug}.example.com/jobs/1"),
            format!("https://{slug}.example.com/about"),
            format!("https://{slug}.example.com/careers"),
        ],
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would pull in gpui's `test` attribute and shadow the standard one.
    use super::is_latest_read;

    #[test]
    fn only_the_latest_lead_count_is_shown() {
        assert!(is_latest_read(3, 3));
        assert!(!is_latest_read(2, 3), "an older answer arriving late is dropped");
    }
}
