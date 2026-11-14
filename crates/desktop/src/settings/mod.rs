//! Settings: six cards, each in its own file (Profile & preferences, AI providers: P1,
//! Browser sessions: P4, Job sources: P10, Gmail: P6, Appearance).

pub mod appearance;
mod gmail;
pub mod profile;
mod providers;
mod sessions;
mod sources;

use std::rc::Rc;

use gpui_kit::component::button::*;
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

/// Shared frame of a settings section: bordered card with a title and a hint line.
/// Append the section's own content with `.child(..)`.
pub fn section(title: &str, hint: &str, cx: &App) -> Div {
    section_with(title, hint, div(), cx)
}

/// [`section`] with `actions` at the right end of the title line.
pub fn section_with(title: &str, hint: &str, actions: impl IntoElement, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .v_flex()
        .gap_2()
        .p_4()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(div().text_lg().font_semibold().child(title.to_string()))
                .child(actions),
        )
        .child(div().text_sm().text_color(theme.muted_foreground).child(hint.to_string()))
}

/// A row of exclusive choices: one compact button per choice, the current one selected. Each
/// button keeps its own id (`<id>-<label>`), so it stays reachable by assistive tools.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    label: &'static str,
    choices: &[(T, &'static str)],
    current: T,
    on_pick: impl Fn(T, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    let on_pick = Rc::new(on_pick);
    let primary = cx.theme().primary;
    let fill = ButtonCustomVariant::new(cx)
        .color(primary.opacity(0.22))
        .hover(primary.opacity(0.30))
        .active(primary.opacity(0.30))
        .foreground(cx.theme().foreground);
    div()
        .id(id)
        .role(Role::Group)
        .aria_label(label)
        .h_flex()
        .flex_wrap()
        .gap_1()
        .children(choices.iter().map(|&(value, text)| {
            let on_pick = on_pick.clone();
            let chosen = value == current;
            // The chosen one is tinted with the theme's primary colour, the others are outlined.
            // "Job type: Remote", not "Remote": other buttons of the page may share the visible word.
            let button = Button::new(SharedString::from(format!("{id}-{text}")))
                .compact()
                .label(text)
                .accessibility_label(format!("{label}: {text}"))
                .selected(chosen);
            let button = if chosen { button.custom(fill) } else { button.outline() };
            button.on_click(move |_, window, cx| on_pick(value, window, cx))
        }))
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
