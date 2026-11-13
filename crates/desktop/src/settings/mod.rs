//! Settings: exactly three sections, each in its own file
//! (AI providers: P1, Browser sessions: P4, Gmail: P6).

mod gmail;
mod providers;
mod sessions;

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::{AppState, Screen};
use gmail::GmailSection;
use providers::ProvidersSection;
use sessions::SessionsSection;

/// Section titles in display order; `settings_focus` names one of them. Child 0 is the header.
const SECTIONS: [&str; 3] = ["AI providers", "Browser sessions", "Gmail"];

pub struct SettingsView {
    scroll: ScrollHandle,
    state: Entity<AppState>,
    providers: Entity<ProvidersSection>,
    sessions: Entity<SessionsSection>,
    gmail: Entity<GmailSection>,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            scroll: ScrollHandle::new(),
            providers: cx.new(|_| ProvidersSection::new(state.clone())),
            sessions: cx.new(|_| SessionsSection::new(state.clone())),
            gmail: cx.new(|cx| GmailSection::new(state.clone(), cx)),
            state,
        }
    }
}

/// Shared frame of a settings section: bordered card with a title and a hint line.
/// Append the section's own content with `.child(..)`.
pub fn section(title: &str, hint: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_2()
        .p_4()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .child(div().text_lg().font_semibold().child(title.to_string()))
        .child(div().text_sm().text_color(theme.muted_foreground).child(hint.to_string()))
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Opened from a call to action ("Open Settings > Browser sessions"): scroll to that section.
        let target = self.state.update(cx, |state, _| state.settings_focus.take());
        if let Some(ix) = target.and_then(|t| SECTIONS.iter().position(|s| *s == t)) {
            self.scroll.scroll_to_item(ix + 1);
        }
        div()
            .id("settings")
            .track_scroll(&self.scroll)
            .v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .overflow_y_scroll()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Settings"))
                    .child(Button::new("close-settings").ghost().label("Back").on_click(cx.listener(
                        |this, _, _, cx| this.state.update(cx, |s, cx| s.show(Screen::Detail, cx)),
                    ))),
            )
            .child(self.providers.clone())
            .child(self.sessions.clone())
            .child(self.gmail.clone())
    }
}
