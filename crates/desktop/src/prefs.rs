//! Preferences screen: the job-search chat and the saved preferences (P9). Placeholder until then.

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::{AppState, Screen};

/// The preference chat and the saved preferences.
pub struct PreferencesView {
    state: Entity<AppState>,
}

impl PreferencesView {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for PreferencesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("preferences")
            .role(Role::Group)
            .aria_label("Preferences")
            .v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Preferences"))
                    .child(Button::new("preferences-back").ghost().label("Back").on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.show(Screen::Detail, cx))
                    }))),
            )
            .child(
                div()
                    .id("preferences-placeholder")
                    .role(Role::Paragraph)
                    .aria_label("Preferences: coming soon")
                    .v_flex()
                    .gap_1()
                    .p_4()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(div().font_semibold().child("Coming soon"))
                    .child(div().text_sm().text_color(theme.muted_foreground).child("Tell JobPilot what job you are looking for and it will remember it here.")),
            )
    }
}
