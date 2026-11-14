//! Left column: navigation (Job list, Trends), the applications grouped by stage, and Settings.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::jobs::group_by_stage;
use crate::app::{status_color, AppState, Screen, FADE};

/// Width of the icon / status-dot column, so every label starts at the same x.
const GUTTER: f32 = 16.0;

pub struct Sidebar {
    state: Entity<AppState>,
}

impl Sidebar {
    pub fn new(state: Entity<AppState>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }

    /// One full-width, left-aligned row of the sidebar: `leading` in the icon column, then the
    /// content. The selected row gets an accent background that fades in. Exposed to assistive
    /// technology (and agent-desktop) as a named button.
    #[allow(clippy::too_many_arguments)]
    fn row(
        &self,
        id: impl Into<ElementId>,
        label: String,
        role: Role,
        selected: bool,
        leading: AnyElement,
        content: impl IntoElement,
        on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &Context<Self>,
    ) -> AnimationElement<Stateful<Div>> {
        let theme = cx.theme();
        let (accent, accent_text) = (theme.sidebar_accent, theme.sidebar_accent_foreground);
        let id: ElementId = id.into();
        // Every row animates on its own state: the key carries the row's id.
        let key = SharedString::from(format!("{id}-selected"));
        div()
            .id(id)
            .role(role)
            .aria_label(label)
            .h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .text_sm()
            .hover(move |d| d.bg(accent))
            .on_click(on_click)
            .child(div().w(px(GUTTER)).flex_shrink_0().h_flex().justify_center().child(leading))
            .child(content)
            // The animation restarts whenever the row becomes (un)selected.
            .with_animation(
                ElementId::NamedInteger(key, u64::from(selected)),
                Animation::new(FADE).with_easing(ease_in_out),
                move |this, delta| {
                    if selected {
                        this.bg(accent.opacity(delta)).text_color(accent_text)
                    } else {
                        this
                    }
                },
            )
    }

    /// A navigation entry: icon, label, an optional count badge.
    #[allow(clippy::too_many_arguments)]
    fn nav(
        &self,
        id: &'static str,
        icon: IconName,
        label: &'static str,
        badge: Option<usize>,
        target: Screen,
        current: Screen,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let (muted, primary) = (theme.muted_foreground, theme.primary);
        let content = div()
            .h_flex()
            .items_center()
            .gap_2()
            .flex_1()
            .min_w_0()
            .child(div().flex_1().min_w_0().truncate().child(label))
            .when_some(badge.filter(|n| *n > 0), |d, n| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .px_1p5()
                        .rounded_full()
                        .text_xs()
                        .font_semibold()
                        .text_color(primary)
                        .bg(primary.opacity(0.14))
                        .child(n.to_string()),
                )
            });
        let aria = match badge.filter(|n| *n > 0) {
            Some(n) => format!("{label}, {n} new"),
            None => label.to_string(),
        };
        let leading = Icon::new(icon).size_4().text_color(muted).into_any_element();
        let on_click = cx.listener(move |this, _, _, cx| {
            this.state.update(cx, |s, cx| s.show(target, cx));
        });
        self.row(id, aria, Role::Button, current == target, leading, content, on_click, cx)
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (apps, selected, screen, lead_count) = {
            let s = self.state.read(cx);
            (s.applications.clone(), s.selected, s.screen, s.lead_count)
        };
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let caption = |text: &'static str| div().px_3().pb_1().text_xs().font_semibold().text_color(muted).child(text);

        let groups: Vec<AnyElement> = group_by_stage(&apps)
            .into_iter()
            .map(|(stage, rows)| {
                div()
                    .id(("stage", stage as usize))
                    .role(Role::Group)
                    .aria_label(stage.label())
                    .v_flex()
                    .gap_0p5()
                    .child(caption(stage.label()))
                    .children(rows.into_iter().map(|app| {
                        let id = app.id;
                        let is_selected = screen == Screen::Detail && selected == Some(id);
                        let progress = self.state.read(cx).job_label(id).map(str::to_string);
                        let busy = progress.is_some();
                        let leading = if busy {
                            Spinner::new().xsmall().color(theme.primary).into_any_element()
                        } else {
                            div().size_2().rounded_full().bg(status_color(app.status, theme)).into_any_element()
                        };
                        let content = div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .h_flex()
                                    .items_center()
                                    .gap_2()
                                    .child(div().flex_1().min_w_0().truncate().child(app.company.clone()))
                                    .when(!busy, |d| {
                                        d.child(div().flex_shrink_0().text_xs().text_color(muted).child(app.status.label()))
                                    }),
                            )
                            .when_some(progress, |d, label| d.child(div().text_xs().text_color(muted).truncate().child(label)));
                        let on_click = cx.listener(move |this, _, _, cx| {
                            this.state.update(cx, |s, cx| s.select(id, cx));
                        });
                        self.row(
                            ("application", id as usize),
                            format!("{}: {}", app.company, app.status.label()),
                            Role::ListItem,
                            is_selected,
                            leading,
                            content,
                            on_click,
                            cx,
                        )
                    }))
                    .into_any_element()
            })
            .collect();

        div()
            .v_flex()
            .w(px(240.))
            .h_full()
            .flex_shrink_0()
            .gap_4()
            .p_3()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(div().px_3().pt_1().text_lg().font_semibold().child("JobPilot"))
            .child(
                div()
                    .id("navigation")
                    .role(Role::Navigation)
                    .aria_label("Navigation")
                    .v_flex()
                    .gap_0p5()
                    .child(self.nav("nav-jobs", IconName::Inbox, "Job list", Some(lead_count), Screen::Jobs, screen, cx))
                    .child(self.nav("nav-trends", IconName::ChartPie, "Trends", None, Screen::Trends, screen, cx)),
            )
            .child(
                div()
                    .id("application-list")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .overflow_y_scroll()
                    .child(div().px_3().text_xs().font_semibold().text_color(muted).child("APPLICATIONS"))
                    .children(groups)
                    .when(apps.is_empty(), |d| {
                        d.child(
                            div()
                                .id("no-applications")
                                .role(Role::Paragraph)
                                .aria_label("No applications yet. Prepare a CV from the Job list.")
                                .px_3()
                                .text_sm()
                                .text_color(muted)
                                .child("No applications yet. Prepare a CV from the Job list."),
                        )
                    }),
            )
            .child(
                div()
                    .pt_2()
                    .border_t_1()
                    .border_color(theme.sidebar_border)
                    .child(self.nav("open-settings", IconName::Settings, "Settings", None, Screen::Settings, screen, cx)),
            )
    }
}
