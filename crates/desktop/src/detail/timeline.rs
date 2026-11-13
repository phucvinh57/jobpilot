//! Timeline tab: the application's events, oldest first.

use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::chrono::Local;

use crate::app::AppState;

pub struct TimelineTab {
    state: Entity<AppState>,
}

impl TimelineTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }
}

impl Render for TimelineTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let events = {
            let state = self.state.read(cx);
            let Some(app) = state.selected_app() else { return div().id("timeline") };
            state.core.store.list_events(app.id).unwrap_or_default()
        };
        let theme = cx.theme();
        div().id("timeline").v_flex().size_full().gap_2().overflow_y_scroll().children(events.into_iter().map(|e| {
            div()
                .h_flex()
                .gap_3()
                .text_sm()
                .child(
                    div()
                        .w(px(130.))
                        .flex_shrink_0()
                        .text_color(theme.muted_foreground)
                        .child(e.at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()),
                )
                .child(div().flex_1().min_w_0().child(e.message))
        }))
    }
}
