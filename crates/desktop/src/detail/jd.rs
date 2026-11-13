//! JD tab: the job description text.

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::app::AppState;

pub struct JdTab {
    state: Entity<AppState>,
}

impl JdTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }
}

impl Render for JdTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (text, source) = match self.state.read(cx).selected_app() {
            Some(app) => (std::fs::read_to_string(app.jd_path()).ok(), app.source.clone()),
            None => (None, None),
        };
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        div()
            .id("jd-text")
            .v_flex()
            .size_full()
            .gap_2()
            .overflow_y_scroll()
            .when_some(source, |d, src| d.child(div().text_sm().text_color(muted).child(src)))
            .child(match text {
                Some(t) => div().child(t),
                None => div().text_color(muted).child("No JD text yet."),
            })
    }
}
