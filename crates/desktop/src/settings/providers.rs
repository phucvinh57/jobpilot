//! Settings > AI providers (P1): add/edit provider, test, choose the default.

use gpui_kit::*;

use super::section;
use crate::app::AppState;

pub struct ProvidersSection {
    state: Entity<AppState>,
}

impl ProvidersSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for ProvidersSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.state.read(cx).core.store.list_providers().map(|p| p.len()).unwrap_or(0);
        section("AI providers", "OpenAI- or Anthropic-compatible endpoints; API keys are kept in the OS keyring.", cx)
            .child(format!("{count} provider(s) configured"))
    }
}
