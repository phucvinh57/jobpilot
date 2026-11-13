//! Queue screen: found jobs sorted by match (P12). Placeholder until then.

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::{AppState, Screen};

/// The queue of found jobs.
pub struct QueueView {
    state: Entity<AppState>,
}

impl QueueView {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for QueueView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("queue")
            .role(Role::Group)
            .aria_label("Queue")
            .v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Queue"))
                    .child(Button::new("queue-back").ghost().label("Back").on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.show(Screen::Detail, cx))
                    }))),
            )
            .child(
                div()
                    .id("queue-placeholder")
                    .role(Role::Paragraph)
                    .aria_label("Queue: coming soon")
                    .v_flex()
                    .gap_1()
                    .p_4()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(div().font_semibold().child("Coming soon"))
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Jobs found from your sources will appear here, best match first.")),
            )
    }
}
