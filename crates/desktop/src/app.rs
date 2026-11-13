//! Shared app state and the root view.

use std::sync::Arc;

use anyhow::Result;
use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::store::{Application, NewApplication, Status};
use jobpilot_core::Core;

use crate::detail::Detail;
use crate::settings::SettingsView;
use crate::sidebar::Sidebar;

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
}

impl AppState {
    pub fn new(core: Arc<Core>) -> Self {
        let mut state = Self { core, applications: Vec::new(), selected: None, screen: Screen::Detail };
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
    #[allow(dead_code)]
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
        div()
            .h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
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
