//! Settings > Appearance: System, Light or Dark.
//!
//! The choice is saved by the core (`Core::set_appearance`, off the UI thread) and applied through
//! gpui-component's theme. "System" follows the window's appearance and keeps following it when
//! the desktop switches. The running choice lives in a gpui global ([`Current`]), so neither the
//! view nor the appearance observer ever reads the database.

use gpui_kit::component::*;
use gpui_kit::*;
use jobpilot_core::prefs::Appearance;
use jobpilot_core::runtime;

use crate::app::AppState;
use crate::ui::theme::{set_mode, sync_system};
use crate::ui::{palette, section_with, segmented};

/// The appearance applied now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Current(pub Appearance);

impl Global for Current {}

/// The choice applied now (System until [`apply`] has run).
pub fn current(cx: &App) -> Appearance {
    cx.try_global::<Current>().map_or(Appearance::System, |c| c.0)
}

/// Apply `appearance` to every window. System reads the window's own appearance when there is one
/// (the desktop's setting may not reach the application-level query on Linux).
pub fn apply(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    cx.set_global(Current(appearance));
    match appearance {
        Appearance::Light => set_mode(ThemeMode::Light, window, cx),
        Appearance::Dark => set_mode(ThemeMode::Dark, window, cx),
        Appearance::System => sync_system(window, cx),
    }
}

/// Follow the desktop's light/dark switch while the choice is System. Call once per window.
pub fn watch_system(window: &mut Window) {
    window
        .observe_window_appearance(|window, cx| {
            if current(cx) == Appearance::System {
                sync_system(Some(window), cx);
            }
        })
        .detach();
}

pub struct AppearanceSection {
    state: Entity<AppState>,
    /// Why the choice could not be saved (it still applies until the app closes).
    error: Option<String>,
}

impl AppearanceSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state, error: None }
    }

    fn pick(&mut self, appearance: Appearance, window: &mut Window, cx: &mut Context<Self>) {
        apply(appearance, Some(window), cx);
        self.error = None;
        cx.notify();
        let core = self.state.read(cx).core.clone();
        let rx = runtime::spawn(async move { core.set_appearance(appearance) });
        cx.spawn(async move |this, cx| {
            let message = match rx.await {
                Ok(Ok(())) => return,
                Ok(Err(e)) => format!("{e:#}"),
                Err(_) => "the task was cancelled".to_string(),
            };
            this.update(cx, |this, cx| {
                this.error = Some(format!("Could not save the choice: {message}"));
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

impl Render for AppearanceSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = *palette(cx);
        let this = cx.entity();
        let current = current(cx);
        let choices = Appearance::ALL.map(|a| (a, a.label()));
        let control = segmented("appearance", "Appearance", &choices, current, move |appearance, window, cx| {
            this.update(cx, |this, cx| this.pick(appearance, window, cx))
        }, cx);
        section_with("Appearance", "Light or dark. System follows your desktop.", control, cx)
            .id("settings-appearance")
            .role(Role::Group)
            .aria_label("Appearance")
            .child(div().text_xs().text_color(p.muted).child(format!("Now: {}", cx.theme().mode.name())))
            .children(self.error.clone().map(|e| div().text_sm().text_color(p.danger).child(e)))
    }
}
