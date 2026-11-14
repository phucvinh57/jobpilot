//! Settings: one scrolling page of cards, in this order: Profile & preferences, Connections (AI
//! providers: P1, Gmail: P6, Browser sessions: P4), Job sources (P10), Appearance. Each card is
//! its own file.

pub mod appearance;
mod gmail;
pub mod profile;
mod providers;
mod sessions;
mod sources;

use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::AppState;
use crate::ui::{caption, heading, Level};
use appearance::AppearanceSection;
use gmail::GmailSection;
use profile::ProfileSection;
use providers::ProvidersSection;
use sessions::SessionsSection;
use sources::SourcesSection;

/// The page's blocks in display order, one child each after the title: `settings_focus` names one
/// of them. "Connections" is the heading over the three cards that follow it.
const SECTIONS: [&str; 7] =
    ["Profile & preferences", "Connections", "AI providers", "Gmail", "Browser sessions", "Job sources", "Appearance"];

/// The child of the page that shows `section` (child 0 is the title), if it is one of [`SECTIONS`].
fn child_of(section: &str) -> Option<usize> {
    SECTIONS.iter().position(|s| *s == section).map(|ix| ix + 1)
}

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
        if let Some(ix) = target.and_then(child_of) {
            self.scroll.scroll_to_item(ix);
        }
        div()
            .id("settings")
            .track_scroll(&self.scroll)
            .v_flex()
            .size_full()
            .gap_5()
            .px_8()
            .py_7()
            .overflow_y_scroll()
            .child(heading("Settings", Level::H1, cx))
            .child(self.profile.clone())
            .child(
                div()
                    .id("settings-connections")
                    .role(Role::Heading)
                    .aria_label("Connections")
                    .v_flex()
                    .gap_0p5()
                    .child(heading("Connections", Level::H2, cx))
                    .child(caption("Keys and tokens are kept in your system keyring, never in files.", cx)),
            )
            .child(self.providers.clone())
            .child(self.gmail.clone())
            .child(self.sessions.clone())
            .child(self.sources.clone())
            .child(self.appearance.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{child_of, SECTIONS};

    #[test]
    fn a_call_to_action_finds_its_card() {
        // Child 0 is the title; each block of the page follows in order.
        assert_eq!(child_of("Profile & preferences"), Some(1));
        assert_eq!(child_of("AI providers"), Some(3));
        assert_eq!(child_of("Browser sessions"), Some(5));
        assert_eq!(child_of("Nothing"), None);
        assert!(SECTIONS.contains(&"Job sources") && SECTIONS.contains(&"Appearance"));
    }
}
