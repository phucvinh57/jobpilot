//! Trends screen: market sources, the latest digest and past reports (P13). Placeholder until then.

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::{AppState, Screen};

/// Market trends from the sources you add.
pub struct TrendsView {
    state: Entity<AppState>,
}

impl TrendsView {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for TrendsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("trends")
            .role(Role::Group)
            .aria_label("Trends")
            .v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Trends"))
                    .child(Button::new("trends-back").ghost().label("Back").on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.show(Screen::Detail, cx))
                    }))),
            )
            .child(
                div()
                    .id("trends-placeholder")
                    .role(Role::Paragraph)
                    .aria_label("Trends: coming soon")
                    .v_flex()
                    .gap_1()
                    .p_4()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(div().font_semibold().child("Coming soon"))
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Add sources and JobPilot will summarise where the market is going, with links.")),
            )
    }
}
