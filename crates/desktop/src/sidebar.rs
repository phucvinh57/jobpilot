//! Left column: JD input, application list, Settings.

use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::{status_color, AppState, Screen};

pub struct Sidebar {
    state: Entity<AppState>,
    /// Paste a JD link here (wired to the generate pipeline in P3).
    jd_input: Entity<InputState>,
}

impl Sidebar {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let jd_input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste JD link or drop file"));
        Self { state, jd_input }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (apps, selected, screen) = {
            let s = self.state.read(cx);
            (s.applications.clone(), s.selected, s.screen)
        };
        let theme = cx.theme();

        let rows = apps.iter().map(|app| {
            let id = app.id;
            let is_selected = screen == Screen::Detail && selected == Some(id);
            div()
                .id(("application", id as usize))
                .h_flex()
                .items_center()
                .gap_2()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .when(is_selected, |d| d.bg(theme.sidebar_accent))
                .hover(|d| d.bg(theme.sidebar_accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |s, cx| s.select(id, cx));
                }))
                .child(div().size_2().flex_shrink_0().rounded_full().bg(status_color(app.status, theme)))
                .child(div().flex_1().min_w_0().truncate().child(app.company.clone()))
                .child(div().text_xs().text_color(theme.muted_foreground).child(app.status.label()))
        });

        div()
            .v_flex()
            .w(px(260.))
            .h_full()
            .flex_shrink_0()
            .gap_3()
            .p_3()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .child(Input::new(&self.jd_input))
            .child(
                div()
                    .id("application-list")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(apps.is_empty(), |d| {
                        d.child(div().p_3().text_sm().text_color(theme.muted_foreground).child("No applications yet"))
                    }),
            )
            .child(
                Button::new("open-settings")
                    .ghost()
                    .icon(IconName::Settings)
                    .label("Settings")
                    .selected(screen == Screen::Settings)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.show(Screen::Settings, cx));
                    })),
            )
    }
}
