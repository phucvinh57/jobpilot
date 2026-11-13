//! Settings > Gmail (P6): read-only access used to track replies.

use gpui_kit::*;

use super::section;
use crate::app::AppState;

pub struct GmailSection {
    #[allow(dead_code)]
    state: Entity<AppState>,
}

impl GmailSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for GmailSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        section("Gmail", "Connect with your own Google Cloud OAuth client (read-only scope).", cx).child("Not connected")
    }
}
