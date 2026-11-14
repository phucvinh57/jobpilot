//! Design components: small builder functions that return gpui elements styled from the palette.
//!
//! They are pure render: no I/O, no state. The caller adds behaviour (`.on_click(..)`) and
//! children. Anything clickable carries an `id` and a `role`, so it stays reachable by assistive
//! tools (and agent-desktop). Colours come from [`palette`], so light and dark need no extra code.

// Phase 2 moves the screens onto the rest of this kit; the design canvas names each of them.
#![allow(dead_code)]

use std::rc::Rc;

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::tokens::{
    palette, Tone, BUTTON_HEIGHT, CARD_PADDING, FONT_HEADING, NAV_HEIGHT, RADIUS_BUTTON, RADIUS_CARD, RADIUS_CHIP,
    RADIUS_TAG, TEXT_BODY, TEXT_H1, TEXT_H2, TEXT_SMALL,
};
use crate::queue::logic as queue_logic;

// ------------------------------------------------------------------ text

/// Heading sizes of the type scale.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Page title, 30 px.
    H1,
    /// Card title, 18 px.
    H2,
}

/// A heading in Space Grotesk.
pub fn heading(text: impl Into<SharedString>, level: Level, cx: &App) -> Div {
    let size = match level {
        Level::H1 => TEXT_H1,
        Level::H2 => TEXT_H2,
    };
    div()
        .text_size(px(size))
        .line_height(relative(1.25))
        .font_family(FONT_HEADING)
        .font_semibold()
        .text_color(palette(cx).ink)
        .child(text.into())
}

/// Small secondary text (12 px, muted): a label, a hint, a timestamp.
pub fn caption(text: impl Into<SharedString>, cx: &App) -> Div {
    div().text_size(px(TEXT_SMALL)).text_color(palette(cx).muted).child(text.into())
}

// ------------------------------------------------------------ containers

/// A white, bordered card with rounded corners. Append content with `.child(..)`.
pub fn card(cx: &App) -> Div {
    let p = palette(cx);
    div()
        .v_flex()
        .gap_2()
        .p(px(CARD_PADDING))
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
}

/// A card with a title and a hint line (the frame of every settings section). Append the
/// section's own content with `.child(..)`.
pub fn section(title: &str, hint: &str, cx: &App) -> Div {
    section_with(title, hint, div(), cx)
}

/// [`section`] with `actions` at the right end of the title line.
pub fn section_with(title: &str, hint: &str, actions: impl IntoElement, cx: &App) -> Div {
    card(cx)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(heading(title.to_string(), Level::H2, cx))
                .child(actions),
        )
        .child(caption(hint.to_string(), cx))
}

/// A highlighted strip for something that needs attention ("Needs you", a problem with Retry):
/// a tone dot, a bold title and a body line, and `actions` on the right.
pub fn banner(tone: Tone, title: &str, body: &str, actions: impl IntoElement, cx: &App) -> Div {
    let colors = palette(cx).tone(tone);
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .p_3()
        .rounded(px(RADIUS_CARD - 2.0))
        .border_1()
        .border_color(colors.dot.opacity(0.35))
        .bg(colors.bg)
        .child(div().size_2().flex_shrink_0().rounded_full().bg(colors.dot))
        .child(
            div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(div().text_size(px(TEXT_BODY)).font_semibold().text_color(colors.fg).child(title.to_string()))
                .child(div().text_size(px(TEXT_SMALL)).text_color(colors.fg).child(body.to_string())),
        )
        .child(actions)
}

// --------------------------------------------------------------- buttons

fn sized(button: Button) -> Button {
    button.h(px(BUTTON_HEIGHT)).px_4().rounded(px(RADIUS_BUTTON)).font_medium()
}

/// The one primary action of a screen: accent fill. Add `.on_click`, `.loading`, `.disabled`.
pub fn primary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    sized(Button::new(id).primary().label(label))
}

/// A plain action: white with a border.
pub fn secondary_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    sized(Button::new(id).secondary().label(label))
}

/// A quiet action: no fill until hovered.
pub fn ghost_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    sized(Button::new(id).ghost().label(label))
}

/// A destructive action (Remove, Delete): a plain button with danger-coloured text and border,
/// never filled, so the screen keeps exactly one filled (primary) button.
pub fn danger_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    sized(Button::new(id).danger().outline().label(label))
}

// ---------------------------------------------------------- chips, tags

/// The kinds of static chip.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChipKind {
    /// A requirement the profile matches.
    Match,
    /// A requirement the profile does not cover yet (dashed outline).
    Gap,
}

/// A small static label for a matched or missing requirement.
pub fn chip(label: impl Into<SharedString>, kind: ChipKind, cx: &App) -> Div {
    let p = palette(cx);
    let base = div()
        .h_flex()
        .items_center()
        .min_h(px(28.0))
        .px_2p5()
        .rounded(px(RADIUS_CHIP))
        .text_size(px(TEXT_SMALL))
        .font_medium()
        .border_1();
    match kind {
        ChipKind::Match => base.border_color(p.accent_soft).bg(p.accent_soft).text_color(p.accent_dark),
        ChipKind::Gap => base.border_dashed().border_color(p.bar_grey).text_color(p.muted),
    }
    .child(label.into())
}

/// A filter toggle. A real `Button`, so it takes keyboard focus and Enter/Space; `selected`
/// gives the tinted look. Add `.on_click(..)`.
pub fn filter_chip(id: impl Into<ElementId>, label: impl Into<SharedString>, selected: bool) -> Button {
    Button::new(id)
        .outline()
        .compact()
        .rounded(px(RADIUS_CHIP))
        .min_h(px(32.0))
        .label(label)
        .selected(selected)
}

/// A status tag: a small rounded pill in a tone's colours.
pub fn tag(label: impl Into<SharedString>, tone: Tone, cx: &App) -> Div {
    let colors = palette(cx).tone(tone);
    div()
        .h_flex()
        .flex_shrink_0()
        .items_center()
        .px_2()
        .py_0p5()
        .rounded(px(RADIUS_TAG))
        .text_size(px(TEXT_SMALL))
        .font_semibold()
        .text_color(colors.fg)
        .bg(colors.bg)
        .child(label.into())
}

/// A small citation chip (`3`) that points at a stored source. Add `.on_click(..)`.
pub fn cite_chip(id: impl Into<ElementId>, number: usize, cx: &App) -> Stateful<Div> {
    let p = palette(cx);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(format!("Source {number}"))
        .h_flex()
        .items_center()
        .justify_center()
        .min_w(px(22.0))
        .h(px(20.0))
        .px_1p5()
        .rounded(px(RADIUS_TAG))
        .cursor_pointer()
        .text_size(px(11.0))
        .font_semibold()
        .text_color(p.accent_dark)
        .bg(p.accent_soft)
        .hover(move |d| d.bg(p.accent.opacity(0.25)))
        .child(number.to_string())
}

// ------------------------------------------------------------ navigation

/// One entry of the left navigation: icon, label, an optional count badge. The current entry is
/// tinted. Add `.on_click(..)`.
pub fn nav_item(
    id: impl Into<ElementId>,
    icon: IconName,
    label: &str,
    badge: Option<usize>,
    current: bool,
    cx: &App,
) -> Stateful<Div> {
    let p = palette(cx);
    let badge = badge.filter(|n| *n > 0);
    let aria = match badge {
        Some(n) => format!("{label}, {n} new"),
        None => label.to_string(),
    };
    let (fg, bg) = if current { (p.accent_dark, p.accent_soft) } else { (p.ink, gpui_kit::transparent_black()) };
    let hover = if current { p.accent_soft } else { p.track };
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(aria)
        .h_flex()
        .items_center()
        .gap_2p5()
        .w_full()
        .min_h(px(NAV_HEIGHT))
        .px_3()
        .rounded(px(RADIUS_BUTTON))
        .cursor_pointer()
        .text_size(px(TEXT_BODY))
        .text_color(fg)
        .bg(bg)
        .when(current, |d| d.font_semibold())
        .hover(move |d| d.bg(hover))
        .child(Icon::new(icon).size_4().text_color(if current { p.accent } else { p.muted }))
        .child(div().flex_1().min_w_0().truncate().child(label.to_string()))
        .when_some(badge, |d, n| {
            d.child(
                div()
                    .flex_shrink_0()
                    .px_1p5()
                    .rounded_full()
                    .text_size(px(11.0))
                    .font_semibold()
                    .text_color(p.on_fill)
                    .bg(p.accent)
                    .child(n.to_string()),
            )
        })
}

/// A small status dot: green when `ok`, amber when something needs a look.
pub fn health_dot(ok: bool, cx: &App) -> Div {
    let p = palette(cx);
    div().size_2().flex_shrink_0().rounded_full().bg(if ok { p.ok } else { p.warn })
}

// ----------------------------------------------------------------- forms

/// A form field: a label above its input.
pub fn field(label: &str, input: impl IntoElement, cx: &App) -> Div {
    div()
        .v_flex()
        .gap_1()
        .child(div().text_size(px(TEXT_SMALL)).font_medium().text_color(palette(cx).muted).child(label.to_string()))
        .child(input)
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
    let p = palette(cx);
    let fill = ButtonCustomVariant::new(cx)
        .color(p.accent_soft)
        .hover(p.accent_soft)
        .active(p.accent_soft)
        .foreground(p.accent_dark);
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
            // The chosen one is tinted with the accent, the others are outlined.
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

// ---------------------------------------------------------- data display

/// How full a bar is: `value / max` clamped to 0..=1 (an empty scale is 0).
pub fn bar_fraction(value: f64, max: f64) -> f32 {
    if max <= 0.0 || value.is_nan() {
        return 0.0;
    }
    (value / max).clamp(0.0, 1.0) as f32
}

/// A horizontal bar: a track with a filled part of `fraction` (0..=1) in `color`.
fn bar(fraction: f32, color: Hsla, cx: &App) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .h(px(6.0))
        .rounded_full()
        .bg(palette(cx).track)
        .child(div().h_full().w(relative(fraction)).rounded_full().bg(color))
}

/// The design tone of a score from the queue logic: high is good, mid needs a look, low is bad.
pub fn score_tone_of(score: Option<f64>) -> Tone {
    match queue_logic::score_tone(score) {
        queue_logic::Tone::High => Tone::Ok,
        queue_logic::Tone::Mid => Tone::Warn,
        queue_logic::Tone::Low => Tone::Danger,
        queue_logic::Tone::Unknown => Tone::Neutral,
    }
}

/// A square badge with a lead's score in its tone colours (a dash when it was never scored).
pub fn score_badge(score: Option<f64>, cx: &App) -> Div {
    let colors = palette(cx).tone(score_tone_of(score));
    div()
        .h_flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(44.0))
        .rounded(px(RADIUS_BUTTON))
        .text_size(px(TEXT_H2))
        .font_family(FONT_HEADING)
        .font_semibold()
        .text_color(colors.fg)
        .bg(colors.bg)
        .child(queue_logic::score_label(score))
}

/// One fit criterion: a label, a bar in the tone of its score (0..=100) and the number.
pub fn fit_bar(label: &str, value: f64, cx: &App) -> Div {
    let p = palette(cx);
    let tone = p.tone(score_tone_of(Some(value)));
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .child(div().w(px(120.0)).flex_shrink_0().truncate().text_size(px(TEXT_SMALL)).text_color(p.muted).child(label.to_string()))
        .child(bar(bar_fraction(value, 100.0), tone.dot, cx))
        .child(
            div()
                .w(px(28.0))
                .flex_shrink_0()
                .text_size(px(TEXT_SMALL))
                .font_semibold()
                .text_color(p.ink)
                .child(format!("{value:.0}")),
        )
}

/// A skill row of the trends chart: name, a bar scaled to the most frequent skill (accent when
/// the skill is already in the profile, grey when it is not) and its count.
pub fn skill_bar(name: &str, count: u32, max: u32, in_profile: bool, cx: &App) -> Div {
    let p = palette(cx);
    let color = if in_profile { p.accent } else { p.bar_grey };
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .child(div().w(px(120.0)).flex_shrink_0().truncate().text_size(px(TEXT_SMALL)).text_color(p.ink).child(name.to_string()))
        .child(bar(bar_fraction(f64::from(count), f64::from(max)), color, cx))
        .child(div().w(px(28.0)).flex_shrink_0().text_size(px(TEXT_SMALL)).text_color(p.muted).child(count.to_string()))
}

/// Where a step of a [`stepper`] stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepState {
    Done,
    Current,
    Todo,
}

/// The state of step `index` when `current` is the step in progress (`current == steps.len()`:
/// every step is done).
pub fn step_state(index: usize, current: usize) -> StepState {
    match index.cmp(&current) {
        std::cmp::Ordering::Less => StepState::Done,
        std::cmp::Ordering::Equal => StepState::Current,
        std::cmp::Ordering::Greater => StepState::Todo,
    }
}

/// A row of numbered steps joined by lines: done steps are filled, the current one is outlined.
pub fn stepper(steps: &[&str], current: usize, cx: &App) -> Div {
    let p = palette(cx);
    let last = steps.len().saturating_sub(1);
    div().h_flex().items_center().gap_2().children(steps.iter().enumerate().flat_map(|(i, label)| {
        let state = step_state(i, current);
        let (fg, bg, border) = match state {
            StepState::Done => (p.on_fill, p.accent, p.accent),
            StepState::Current => (p.accent_dark, p.accent_soft, p.accent),
            StepState::Todo => (p.muted, p.surface, p.border),
        };
        let marker = div()
            .h_flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .size(px(24.0))
            .rounded_full()
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_size(px(TEXT_SMALL))
            .font_semibold()
            .child(if state == StepState::Done {
                Icon::new(IconName::Check).size_3().text_color(fg).into_any_element()
            } else {
                (i + 1).to_string().into_any_element()
            });
        let text = div()
            .text_size(px(TEXT_SMALL))
            .when(state == StepState::Current, |d| d.font_semibold())
            .text_color(if state == StepState::Todo { p.muted } else { p.ink })
            .child(label.to_string());
        let item = div()
            .id(("step", i))
            .role(Role::ListItem)
            .aria_label(format!("Step {}: {label}", i + 1))
            .h_flex()
            .items_center()
            .gap_2()
            .child(marker)
            .child(text)
            .into_any_element();
        let line = (i < last).then(|| {
            let done = i < current;
            div().flex_1().min_w(px(16.0)).h(px(2.0)).rounded_full().bg(if done { p.accent } else { p.border }).into_any_element()
        });
        std::iter::once(item).chain(line)
    }))
}

/// A card on the pipeline board: title, subtitle and a tag with the next action. Add
/// `.on_click(..)` to open it.
pub fn kanban_card(
    id: impl Into<ElementId>,
    title: &str,
    subtitle: &str,
    next: Option<(&str, Tone)>,
    cx: &App,
) -> Stateful<Div> {
    let p = palette(cx);
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(format!("{title}, {subtitle}"))
        .v_flex()
        .gap_1p5()
        .p_3()
        .rounded(px(RADIUS_CARD - 2.0))
        .border_1()
        .border_color(p.border)
        .bg(p.surface)
        .cursor_pointer()
        .hover(move |d| d.border_color(p.accent.opacity(0.5)))
        .child(div().text_size(px(TEXT_BODY)).font_semibold().text_color(p.ink).child(title.to_string()))
        .child(div().text_size(px(TEXT_SMALL)).text_color(p.muted).child(subtitle.to_string()))
        .when_some(next, |d, (label, tone)| d.child(div().h_flex().pt_1().child(tag(label.to_string(), tone, cx))))
}

/// One line of the activity feed: a tone dot, what happened, and when.
pub fn activity_row(tone: Tone, text: &str, when: &str, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .h_flex()
        .items_center()
        .gap_3()
        .py_1p5()
        .child(div().size_2().flex_shrink_0().rounded_full().bg(p.tone(tone).dot))
        .child(div().flex_1().min_w_0().truncate().text_size(px(TEXT_BODY)).text_color(p.ink).child(text.to_string()))
        .child(div().flex_shrink_0().text_size(px(TEXT_SMALL)).text_color(p.subtle).child(when.to_string()))
}

/// What a list shows when it has nothing: an icon, a title, a hint and an optional action.
pub fn empty_state(icon: IconName, title: &str, body: &str, action: Option<AnyElement>, cx: &App) -> Div {
    let p = palette(cx);
    div()
        .v_flex()
        .items_center()
        .gap_2()
        .p_6()
        .rounded(px(RADIUS_CARD))
        .border_1()
        .border_dashed()
        .border_color(p.border)
        .child(
            div()
                .h_flex()
                .items_center()
                .justify_center()
                .size(px(44.0))
                .rounded_full()
                .bg(p.track)
                .child(Icon::new(icon).size_5().text_color(p.muted)),
        )
        .child(div().text_size(px(TEXT_BODY)).font_semibold().text_color(p.ink).child(title.to_string()))
        .child(div().text_size(px(TEXT_SMALL)).text_color(p.muted).child(body.to_string()))
        .when_some(action, |d, action| d.child(div().pt_1().child(action)))
}

#[cfg(test)]
mod tests {
    use super::{bar_fraction, score_tone_of, step_state, StepState};
    use crate::ui::tokens::Tone;

    #[test]
    fn steps_before_the_current_are_done() {
        assert_eq!(step_state(0, 2), StepState::Done);
        assert_eq!(step_state(1, 2), StepState::Done);
        assert_eq!(step_state(2, 2), StepState::Current);
        assert_eq!(step_state(3, 2), StepState::Todo);
        assert_eq!(step_state(4, 2), StepState::Todo);
    }

    #[test]
    fn a_current_past_the_end_means_all_done() {
        assert!((0..5).all(|i| step_state(i, 5) == StepState::Done));
        assert_eq!(step_state(0, 0), StepState::Current);
    }

    #[test]
    fn bars_are_clamped() {
        assert_eq!(bar_fraction(50.0, 100.0), 0.5);
        assert_eq!(bar_fraction(150.0, 100.0), 1.0);
        assert_eq!(bar_fraction(-5.0, 100.0), 0.0);
        assert_eq!(bar_fraction(3.0, 0.0), 0.0);
        assert_eq!(bar_fraction(f64::NAN, 10.0), 0.0);
    }

    #[test]
    fn scores_map_to_tones() {
        assert_eq!(score_tone_of(Some(90.0)), Tone::Ok);
        assert_eq!(score_tone_of(Some(60.0)), Tone::Warn);
        assert_eq!(score_tone_of(Some(10.0)), Tone::Danger);
        assert_eq!(score_tone_of(None), Tone::Neutral);
    }
}
