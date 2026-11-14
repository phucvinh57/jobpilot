//! Left column: the logo, navigation (Today, Pipeline, Trends), Settings and the health line.
//! The applications themselves live on the Pipeline screen.

use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::health::{Health, Problem};

use crate::app::{AppState, Screen};
use crate::ui::{self, NavBadge};

/// Width of the sidebar (px); the Application screen sizes its CV pages from it.
pub(crate) const WIDTH: f32 = 224.0;

/// The one-line setup state at the bottom of the sidebar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthLine {
    /// Dot colour: green when nothing is missing, amber when something needs a look.
    pub ok: bool,
    /// Still checking: a neutral dot.
    pub checking: bool,
    pub text: String,
    /// The Settings section that helps with the first problem, if there is one.
    pub focus: Option<&'static str>,
}

/// What to say about the prerequisites (AI provider, typst, Chrome): nothing is claimed before the
/// first check finished.
pub fn health_line(health: Option<&Health>) -> HealthLine {
    let Some(health) = health else {
        return HealthLine { ok: true, checking: true, text: "Checking setup".into(), focus: None };
    };
    let mut problems = Vec::new();
    if !health.provider {
        problems.push(Problem::NoProvider);
    }
    if health.typst.is_none() {
        problems.push(Problem::NoTypst);
    }
    if !health.chrome {
        problems.push(Problem::NoChrome);
    }
    let focus = problems.iter().find_map(|p| p.settings_section());
    let text = match problems.as_slice() {
        [] => "All checks passing".to_string(),
        [Problem::NoProvider] => "No AI provider".to_string(),
        [Problem::NoTypst] => "typst is missing".to_string(),
        [Problem::NoChrome] => "No Chrome found".to_string(),
        many => format!("{} setup problems", many.len()),
    };
    HealthLine { ok: problems.is_empty(), checking: false, text, focus }
}

impl HealthLine {
    /// Pressing the line opens Settings: only when something is missing and a Settings section
    /// helps with it (typst has none).
    pub fn actionable(&self) -> bool {
        !self.ok && self.focus.is_some()
    }
}

/// The badge of the Today entry: what needs the user, in amber, else the number of new leads.
pub fn today_badge(needs_you: usize, new_leads: usize) -> Option<NavBadge> {
    if needs_you > 0 {
        Some(NavBadge::NeedsYou(needs_you))
    } else if new_leads > 0 {
        Some(NavBadge::New(new_leads))
    } else {
        None
    }
}

/// The entry to highlight: an open application belongs to the screen it was opened from.
pub fn nav_screen(screen: Screen, detail_from: Screen) -> Screen {
    if screen == Screen::Detail {
        detail_from
    } else {
        screen
    }
}

pub struct Sidebar {
    state: Entity<AppState>,
}

impl Sidebar {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }

    /// A navigation entry that shows `target` when pressed.
    #[allow(clippy::too_many_arguments)]
    fn nav(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        badge: Option<NavBadge>,
        target: Screen,
        current: Screen,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        ui::nav_item_with(id, icon, label, badge, current == target, cx)
            .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.show(target, cx))))
    }

    /// The logo: a ring with a needle, and the name.
    fn logo(&self, cx: &Context<Self>) -> impl IntoElement {
        let p = ui::palette(cx);
        div()
            .h_flex()
            .items_center()
            .gap_2p5()
            .px_2p5()
            .pt_1()
            .pb_5()
            .child(
                div()
                    .size(px(26.0))
                    .flex_shrink_0()
                    .h_flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(p.accent)
                    .child(Icon::new(IconName::ArrowUp).size_3().text_color(p.accent).rotate(radians(std::f32::consts::FRAC_PI_4))),
            )
            .child(ui::heading("JobPilot", ui::Level::H2, cx))
    }

    /// The health line: a dot and a few words. When something is missing and Settings has a section
    /// that fixes it, pressing the line opens that section.
    fn health(&self, line: HealthLine, cx: &Context<Self>) -> impl IntoElement {
        let p = ui::palette(cx);
        let dot = if line.checking { div().size_2().flex_shrink_0().rounded_full().bg(p.bar_grey) } else { ui::health_dot(line.ok, cx) };
        let focus = line.focus;
        let actionable = line.actionable();
        let row = div()
            .id("health")
            .role(if actionable { Role::Button } else { Role::Paragraph })
            .aria_label(line.text.clone())
            .h_flex()
            .items_center()
            .gap_2()
            .px_2p5()
            .pt_3()
            .text_size(px(ui::tokens::TEXT_SMALL))
            .text_color(p.subtle)
            .child(dot)
            .child(div().flex_1().min_w_0().truncate().child(line.text));
        if !actionable {
            row
        } else {
            row.cursor_pointer()
                .hover(move |d| d.text_color(p.ink))
                .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.open_settings(focus, cx))))
        }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (screen, today, applications, health) = {
            let s = self.state.read(cx);
            (
                nav_screen(s.screen, s.detail_from),
                today_badge(s.needs_you, s.lead_count),
                s.applications.len(),
                health_line(s.health.as_ref()),
            )
        };
        let p = ui::palette(cx);
        div()
            .v_flex()
            .w(px(WIDTH))
            .h_full()
            .flex_shrink_0()
            .gap_1()
            .px_3()
            .py_5()
            .bg(p.surface_alt)
            .text_color(p.ink)
            .border_r_1()
            .border_color(p.border)
            .child(self.logo(cx))
            .child(
                div()
                    .id("navigation")
                    .role(Role::Navigation)
                    .aria_label("Navigation")
                    .v_flex()
                    .gap_1()
                    .child(self.nav("nav-today", IconName::Inbox, "Today", today, Screen::Jobs, screen, cx))
                    .child(self.nav(
                        "nav-pipeline",
                        IconName::LayoutDashboard,
                        "Pipeline",
                        Some(NavBadge::Total(applications)),
                        Screen::Pipeline,
                        screen,
                        cx,
                    ))
                    .child(self.nav("nav-trends", IconName::ChartPie, "Trends", None, Screen::Trends, screen, cx)),
            )
            // Pushes Settings and the health line to the bottom.
            .child(div().flex_1())
            .child(self.nav("open-settings", IconName::Settings, "Settings", None, Screen::Settings, screen, cx))
            .child(self.health(health, cx))
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::health::Health;

    use super::{health_line, nav_screen, today_badge};
    use crate::app::Screen;
    use crate::ui::NavBadge;

    fn health(typst: bool, chrome: bool, provider: bool) -> Health {
        Health { typst: typst.then(|| "typst 0.13".to_string()), chrome, provider }
    }

    #[test]
    fn nothing_is_claimed_before_the_first_check() {
        let line = health_line(None);
        assert!(line.checking);
        assert_eq!(line.text, "Checking setup");
        assert_eq!(line.focus, None);
    }

    #[test]
    fn all_good_is_one_calm_line() {
        let line = health_line(Some(&health(true, true, true)));
        assert!(line.ok && !line.checking);
        assert_eq!(line.text, "All checks passing");
        assert_eq!(line.focus, None);
    }

    #[test]
    fn one_problem_is_named_and_points_at_its_settings() {
        let line = health_line(Some(&health(true, true, false)));
        assert!(!line.ok);
        assert_eq!(line.text, "No AI provider");
        assert_eq!(line.focus, Some("AI providers"));
        let line = health_line(Some(&health(true, false, true)));
        assert_eq!(line.text, "No Chrome found");
        assert_eq!(line.focus, Some("Browser sessions"));
        // typst has no settings section: the line is still shown, with nothing to open.
        let line = health_line(Some(&health(false, true, true)));
        assert_eq!(line.text, "typst is missing");
        assert_eq!(line.focus, None);
    }

    #[test]
    fn only_a_line_with_somewhere_to_go_is_pressable() {
        assert!(!health_line(None).actionable(), "still checking");
        assert!(!health_line(Some(&health(true, true, true))).actionable(), "nothing to fix");
        assert!(health_line(Some(&health(true, true, false))).actionable());
        assert!(!health_line(Some(&health(false, true, true))).actionable(), "typst has no Settings section");
        assert!(health_line(Some(&health(false, false, true))).actionable(), "Chrome has one");
    }

    #[test]
    fn today_shows_what_needs_you_before_new_leads() {
        assert_eq!(today_badge(2, 5), Some(NavBadge::NeedsYou(2)));
        assert_eq!(today_badge(0, 5), Some(NavBadge::New(5)));
        assert_eq!(today_badge(1, 0), Some(NavBadge::NeedsYou(1)));
        assert_eq!(today_badge(0, 0), None);
    }

    #[test]
    fn an_open_application_highlights_where_it_was_opened_from() {
        assert_eq!(nav_screen(Screen::Detail, Screen::Jobs), Screen::Jobs);
        assert_eq!(nav_screen(Screen::Detail, Screen::Pipeline), Screen::Pipeline);
        assert_eq!(nav_screen(Screen::Trends, Screen::Jobs), Screen::Trends);
        assert_eq!(nav_screen(Screen::Settings, Screen::Pipeline), Screen::Settings);
    }

    #[test]
    fn several_problems_are_counted() {
        let line = health_line(Some(&health(false, false, false)));
        assert_eq!(line.text, "3 setup problems");
        assert_eq!(line.focus, Some("AI providers"), "the first problem with a fix wins");
    }
}
