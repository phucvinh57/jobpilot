//! Settings > Browser sessions (P4): app-owned browser profiles used to apply.

use gpui_kit::*;

use super::section;
use crate::app::AppState;

pub struct SessionsSection {
    state: Entity<AppState>,
}

impl SessionsSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self { state }
    }
}

impl Render for SessionsSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.state.read(cx).core.store.list_sessions().map(|s| s.len()).unwrap_or(0);
        section("Browser sessions", "Logins kept in a managed browser profile, separate from your own browser.", cx)
            .child(format!("{count} session(s)"))
    }
}
