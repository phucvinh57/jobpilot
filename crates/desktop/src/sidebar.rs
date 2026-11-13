//! Left column: JD input, application list, Settings.

use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use jobpilot_core::health;

use crate::app::jobs::parse_input;
use crate::app::{status_color, AppState, Screen};
use crate::problem::{self, RetryFn};

pub struct Sidebar {
    state: Entity<AppState>,
    /// Paste a JD link (or the JD text) and press Enter; a file can be dropped on the window.
    jd_input: Entity<InputState>,
}

impl Sidebar {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let jd_input = cx.new(|cx| InputState::new(window, cx).placeholder("Paste JD link or drop file"));
        cx.subscribe_in(&jd_input, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                let raw = input.read(cx).value();
                let Some(jd) = parse_input(&raw) else { return };
                input.update(cx, |input, cx| input.set_value("", window, cx));
                this.state.update(cx, |state, cx| state.submit_input(jd, cx));
            }
            InputEvent::Change => {
                // Typing again clears the last "could not start" message.
                this.state.update(cx, |state, cx| {
                    if state.input_error.take().is_some() {
                        cx.notify();
                    }
                });
            }
            _ => {}
        })
        .detach();
        Self { state, jd_input }
    }
}

impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (apps, selected, screen, input_error) = {
            let s = self.state.read(cx);
            (s.applications.clone(), s.selected, s.screen, s.input_error.clone())
        };
        let can_retry = self.state.read(cx).can_retry_input();
        let input_help = input_error.as_deref().and_then(|message| {
            let problem = health::classify(message);
            let state = self.state.clone();
            let retry: RetryFn = {
                let state = state.clone();
                std::rc::Rc::new(move |_, cx| state.update(cx, |s, cx| s.retry_input(cx)))
            };
            problem::help("input-error", problem, &state, can_retry.then_some(retry), cx)
        });
        let theme = cx.theme();

        let rows = apps.iter().map(|app| {
            let id = app.id;
            let is_selected = screen == Screen::Detail && selected == Some(id);
            let progress = self.state.read(cx).job_label(id).map(str::to_string);
            let busy = progress.is_some();
            div()
                .id(("application", id as usize))
                .v_flex()
                .gap_0p5()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                // Exposed to assistive technology (and agent-desktop) as a named, clickable row.
                .role(Role::ListItem)
                .aria_label(format!("{}: {}", app.company, app.status.label()))
                .when(is_selected, |d| d.bg(theme.sidebar_accent))
                .hover(|d| d.bg(theme.sidebar_accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.state.update(cx, |s, cx| s.select(id, cx));
                }))
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(if busy {
                            div().flex_shrink_0().child(Spinner::new().xsmall().color(theme.primary))
                        } else {
                            div().size_2().flex_shrink_0().rounded_full().bg(status_color(app.status, theme))
                        })
                        .child(div().flex_1().min_w_0().truncate().child(app.company.clone()))
                        .when(!busy, |d| {
                            d.child(div().text_xs().text_color(theme.muted_foreground).child(app.status.label()))
                        }),
                )
                .when_some(progress, |d, label| {
                    d.child(div().pl_4().text_xs().text_color(theme.muted_foreground).truncate().child(label))
                })
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
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(Input::new(&self.jd_input))
                    .when_some(input_error, |d, message| {
                        d.child(div().text_xs().text_color(theme.danger).child(message))
                    })
                    .children(input_help),
            )
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
                        d.child(
                            div()
                                .v_flex()
                                .gap_1()
                                .p_3()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(div().font_semibold().child("No applications yet"))
                                .child("Paste the link of a job description above and press Enter."),
                        )
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
