//! Apply tab: review the filled application form, then submit (P5).

use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::AppState;

use super::PrimaryAction;

pub struct ApplyTab;

impl ApplyTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self
    }

    /// Called by the detail pane's primary button for `ReviewAndSubmit` and `Submit`
    /// (the pane has already switched to this tab). Implemented in P5.
    pub fn on_primary(&mut self, _action: PrimaryAction, _window: &mut Window, _cx: &mut Context<Self>) {}
}

impl Render for ApplyTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p_3()
            .border_1()
            .border_color(cx.theme().border)
            .rounded_md()
            .text_color(cx.theme().muted_foreground)
            .child("Form answers, attached PDF and filled-form screenshot appear here (P5).")
    }
}
