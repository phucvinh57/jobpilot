//! Shared "something is missing or failed" pieces: what to do about it, and the buttons that do it.
//!
//! The classification (no AI provider, no Chrome, no typst, network, a site that needs the user) lives in
//! `jobpilot_core::health`; this file turns it into call-to-action rows so every screen points the user
//! to the same fixes.

use std::rc::Rc;

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::health::Problem;

use crate::app::{AppState, Screen};

/// What the Retry button does.
pub type RetryFn = Rc<dyn Fn(&mut Window, &mut App)>;

/// Advice and buttons for `problem`: Settings shortcut, "Check again" for a missing typst, Retry for
/// network errors. `None` when there is nothing useful to offer (a plain error).
pub fn help(id: &str, problem: Problem, state: &Entity<AppState>, retry: Option<RetryFn>, cx: &App) -> Option<Div> {
    help_with(id, problem, state, retry, None, cx)
}

/// [`help`] plus the handler of "Open browser", shown for `Problem::NeedsHuman` (a login, captcha or
/// block page the user must deal with in the visible browser). The paused run that knows the page
/// supplies `open_browser`; without one the button opens Settings > Browser sessions.
pub fn help_with(
    id: &str,
    problem: Problem,
    state: &Entity<AppState>,
    retry: Option<RetryFn>,
    open_browser: Option<RetryFn>,
    cx: &App,
) -> Option<Div> {
    let advice = problem.advice()?;
    let muted = cx.theme().muted_foreground;
    let buttons = action_buttons(id, problem, state, retry, open_browser);
    Some(
        div()
            .v_flex()
            .gap_1()
            .child(div().text_sm().text_color(muted).child(advice))
            .when(!buttons.is_empty(), |d| d.child(div().h_flex().flex_wrap().gap_2().children(buttons))),
    )
}

/// The buttons of [`help_with`] alone: Settings shortcut, "Check again", "Open browser", Retry.
fn action_buttons(
    id: &str,
    problem: Problem,
    state: &Entity<AppState>,
    retry: Option<RetryFn>,
    open_browser: Option<RetryFn>,
) -> Vec<AnyElement> {
    let mut buttons: Vec<AnyElement> = Vec::new();

    if let Some(section) = problem.settings_section() {
        let state = state.clone();
        buttons.push(
            Button::new(SharedString::from(format!("{id}-settings")))
                .outline()
                .compact()
                .icon(IconName::Settings)
                .label(format!("Open Settings > {section}"))
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.open_settings(Some(section), cx)))
                .into_any_element(),
        );
    }
    if problem == Problem::NoTypst {
        let state = state.clone();
        buttons.push(
            Button::new(SharedString::from(format!("{id}-recheck")))
                .outline()
                .compact()
                .label("Check again")
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.refresh_health(cx)))
                .into_any_element(),
        );
    }
    if matches!(problem, Problem::NeedsHuman(_)) {
        let state = state.clone();
        buttons.push(
            Button::new(SharedString::from(format!("{id}-open-browser")))
                .outline()
                .compact()
                .icon(IconName::ExternalLink)
                .label("Open browser")
                .on_click(move |_, window, cx| match &open_browser {
                    Some(open) => open(window, cx),
                    None => state.update(cx, |s, cx| s.open_settings(Some("Browser sessions"), cx)),
                })
                .into_any_element(),
        );
    }
    if let Some(retry) = retry.filter(|_| problem.retryable()) {
        buttons.push(
            Button::new(SharedString::from(format!("{id}-retry")))
                .outline()
                .compact()
                .label("Retry")
                .on_click(move |_, window, cx| retry(window, cx))
                .into_any_element(),
        );
    }
    buttons
}

/// Compact bar above the main pane while something the main flow needs is missing (no AI provider,
/// no typst), and until the job preferences are saved (it points to Settings > Profile & preferences).
/// One line per missing thing, each with its action button. `None` when everything is in place (or
/// the first check has not finished).
pub fn setup_banner(state: &Entity<AppState>, cx: &App) -> Option<Div> {
    let (blockers, screen, prefs_saved) = {
        let s = state.read(cx);
        (s.blockers(), s.screen, s.prefs_saved)
    };
    // Settings is where the user fixes these; no need to nag there.
    if screen == Screen::Settings {
        return None;
    }
    let theme = cx.theme();
    // One line: warning icon, what is missing, then the buttons that fix it.
    let line = |id: &str, title: &'static str, buttons: Vec<AnyElement>| {
        div()
            .id(SharedString::from(format!("{id}-banner")))
            .role(Role::Group)
            .aria_label(title)
            .h_flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_1p5()
            .rounded_md()
            .border_1()
            .border_color(theme.warning.opacity(0.45))
            .bg(theme.warning.opacity(0.08))
            .child(Icon::new(IconName::TriangleAlert).size_4().flex_shrink_0().text_color(theme.warning))
            .child(div().flex_1().min_w_0().truncate().text_sm().child(title))
            .child(div().h_flex().flex_shrink_0().gap_2().children(buttons))
    };
    let mut rows: Vec<Stateful<Div>> = blockers
        .into_iter()
        .filter_map(|problem| {
            let (id, title) = match problem {
                Problem::NoProvider => ("setup-provider", "No AI provider yet: JobPilot cannot write CVs without one."),
                Problem::NoTypst => ("setup-typst", "Typst is not installed: CVs cannot be rendered."),
                _ => return None,
            };
            problem.advice()?;
            Some(line(id, title, action_buttons(id, problem, state, None, None)))
        })
        .collect();
    if !prefs_saved {
        let state = state.clone();
        let open = Button::new("setup-preferences")
            .outline()
            .compact()
            .icon(IconName::Settings2)
            .label("Set your preferences")
            .on_click(move |_, _, cx| state.update(cx, |s, cx| s.open_settings(Some("Profile & preferences"), cx)))
            .into_any_element();
        rows.push(line("setup-preferences", "Tell JobPilot what job you are looking for.", vec![open]));
    }
    if rows.is_empty() {
        return None;
    }
    Some(div().v_flex().gap_1().mx_6().mt_4().children(rows))
}
