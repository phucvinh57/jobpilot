//! Settings: six cards, each in its own file (Profile & preferences, AI providers: P1,
//! Browser sessions: P4, Job sources: P10, Gmail: P6, Appearance).

pub mod appearance;
mod gmail;
pub mod profile;
mod providers;
mod sessions;
mod sources;

use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::AppState;
use appearance::AppearanceSection;
use gmail::GmailSection;
use profile::ProfileSection;
use providers::ProvidersSection;
use sessions::SessionsSection;
use sources::SourcesSection;

/// Section titles in display order; `settings_focus` names one of them. Child 0 is the header.
const SECTIONS: [&str; 6] =
    ["Profile & preferences", "AI providers", "Browser sessions", "Job sources", "Gmail", "Appearance"];

pub struct SettingsView {
    scroll: ScrollHandle,
    state: Entity<AppState>,
    profile: Entity<ProfileSection>,
    providers: Entity<ProvidersSection>,
    sessions: Entity<SessionsSection>,
    sources: Entity<SourcesSection>,
    gmail: Entity<GmailSection>,
    appearance: Entity<AppearanceSection>,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            scroll: ScrollHandle::new(),
            profile: cx.new(|cx| ProfileSection::new(state.clone(), cx)),
            providers: cx.new(|_| ProvidersSection::new(state.clone())),
            sessions: cx.new(|_| SessionsSection::new(state.clone())),
            sources: cx.new(|_| SourcesSection::new(state.clone())),
            gmail: cx.new(|cx| GmailSection::new(state.clone(), cx)),
            appearance: cx.new(|_| AppearanceSection::new(state.clone())),
            state,
        }
    }
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
            .p_6()
            .overflow_y_scroll()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_2xl().font_semibold().child("Settings")),
            )
            .child(self.profile.clone())
            .child(self.providers.clone())
            .child(self.sessions.clone())
            .child(self.sources.clone())
            .child(self.gmail.clone())
            .child(self.appearance.clone())
    }
}
