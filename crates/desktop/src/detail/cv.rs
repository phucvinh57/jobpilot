//! CV tab: YAML editor and PDF preview side by side, plus the "Ask AI" box (P3).

use gpui_kit::component::input::*;
use gpui_kit::component::*;
use gpui_kit::*;

use crate::app::AppState;

pub struct CvTab {
    ask_ai: Entity<InputState>,
}

impl CvTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let ask_ai = cx.new(|cx| InputState::new(window, cx).placeholder("Ask AI: \"shorter, stress Rust\""));
        Self { ask_ai }
    }
}

fn pane(title: &str, hint: &str, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .v_flex()
        .flex_1()
        .min_w_0()
        .h_full()
        .gap_1()
        .p_3()
        .border_1()
        .border_color(theme.border)
        .rounded_md()
        .child(div().text_sm().font_semibold().child(title.to_string()))
        .child(div().text_sm().text_color(theme.muted_foreground).child(hint.to_string()))
}

impl Render for CvTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .v_flex()
            .size_full()
            .gap_3()
            .child(
                div()
                    .h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .child(pane("YAML editor", "CV YAML editor (P3).", cx))
                    .child(pane("PDF preview", "Rendered CV preview (P3).", cx)),
            )
            .child(Input::new(&self.ask_ai))
    }
}
