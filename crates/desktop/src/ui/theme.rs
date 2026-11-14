//! Maps the design [`Palette`] onto gpui-component's `Theme`, so stock widgets (buttons, inputs,
//! checkboxes, tabs, scrollbars) follow the design too, and sets the body font and radii.
//!
//! `Theme::change` reloads the registered theme config and so drops these values: switch modes
//! with [`set_mode`] / [`sync_system`], never with `Theme` directly.

use gpui_kit::component::{ActiveTheme as _, Theme, ThemeColor, ThemeMode};
use gpui_kit::{px, App, Hsla, Window};

use super::tokens::{palette_for, Palette, FONT_BODY, RADIUS_BUTTON, RADIUS_CARD};

/// Switch to `mode` and put the design palette on top of the stock theme.
pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    apply_palette(cx.theme().mode, cx);
}

/// Follow the desktop's light/dark setting, with the design palette on top.
pub fn sync_system(window: Option<&mut Window>, cx: &mut App) {
    Theme::sync_system_appearance(window, cx);
    apply_palette(cx.theme().mode, cx);
}

/// Apply the palette of `mode` (the mode the theme is in now) to the global theme.
pub fn apply_palette(mode: ThemeMode, cx: &mut App) {
    // No theme yet (before `Theme::change` ran): nothing to edit.
    if !cx.has_global::<Theme>() {
        return;
    }
    let palette = palette_for(mode);
    Theme::update(cx, |theme| {
        set_colors(&mut theme.colors, palette, mode.is_dark());
        theme.font_family = FONT_BODY.into();
        theme.radius = px(RADIUS_BUTTON);
        theme.radius_lg = px(RADIUS_CARD);
    });
}

/// One step towards the "more pressed" side of `color`: darker on light, lighter on dark.
fn shade(color: Hsla, dark: bool, step: f32) -> Hsla {
    let l = if dark { color.l + step } else { color.l - step };
    Hsla { l: l.clamp(0.0, 1.0), ..color }
}

/// Every `ThemeColor` field the app's widgets read, from the palette. Pure, so it is tested
/// without a window.
pub fn set_colors(c: &mut ThemeColor, p: &Palette, dark: bool) {
    let clear = Hsla::transparent_black();
    let hover = |color: Hsla| shade(color, dark, 0.05);
    let active = |color: Hsla| shade(color, dark, 0.09);

    // Page, text, borders.
    c.background = p.bg;
    c.foreground = p.ink;
    c.muted = p.track;
    c.muted_foreground = p.muted;
    c.border = p.border;
    c.input = p.border;
    c.ring = p.accent;
    c.caret = p.ink;
    c.selection = p.accent.opacity(0.28);
    c.popover = p.surface;
    c.popover_foreground = p.ink;
    c.group_box = p.surface;
    c.group_box_foreground = p.ink;
    c.accordion = p.surface;
    c.description_list_label = p.surface_alt;
    c.description_list_label_foreground = p.muted;
    c.drag_border = p.accent;
    c.drop_target = p.accent_soft;
    c.window_border = p.border;
    // `accent` is the hover/selected background of menu and list items.
    c.accent = p.accent_soft;
    c.accent_foreground = p.accent_dark;
    c.link = p.accent;
    c.link_hover = p.accent_dark;
    c.link_active = p.accent_dark;

    // Buttons: primary is the accent; the default and secondary buttons are white with a border.
    c.primary = p.accent;
    c.primary_hover = p.accent_dark;
    c.primary_active = shade(p.accent_dark, dark, 0.06);
    c.primary_foreground = p.on_fill;
    c.button_primary = c.primary;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button_primary_foreground = c.primary_foreground;
    c.secondary = p.surface;
    c.secondary_hover = p.track;
    c.secondary_active = p.border;
    c.secondary_foreground = p.ink;
    c.button = p.surface;
    c.button_hover = p.track;
    c.button_active = p.border;
    c.button_foreground = p.ink;
    c.button_secondary = c.secondary;
    c.button_secondary_hover = c.secondary_hover;
    c.button_secondary_active = c.secondary_active;
    c.button_secondary_foreground = c.secondary_foreground;

    // Status colours.
    c.info = p.accent;
    c.info_hover = hover(p.accent);
    c.info_active = active(p.accent);
    c.info_foreground = p.on_fill;
    c.warning = p.warn;
    c.warning_hover = hover(p.warn);
    c.warning_active = active(p.warn);
    c.warning_foreground = p.on_fill;
    c.success = p.ok;
    c.success_hover = hover(p.ok);
    c.success_active = active(p.ok);
    c.success_foreground = p.on_fill;
    c.danger = p.danger;
    c.danger_hover = hover(p.danger);
    c.danger_active = active(p.danger);
    c.danger_foreground = p.on_fill;
    c.button_info = c.info;
    c.button_info_hover = c.info_hover;
    c.button_info_active = c.info_active;
    c.button_info_foreground = c.info_foreground;
    c.button_warning = c.warning;
    c.button_warning_hover = c.warning_hover;
    c.button_warning_active = c.warning_active;
    c.button_warning_foreground = c.warning_foreground;
    c.button_success = c.success;
    c.button_success_hover = c.success_hover;
    c.button_success_active = c.success_active;
    c.button_success_foreground = c.success_foreground;
    c.button_danger = c.danger;
    c.button_danger_hover = c.danger_hover;
    c.button_danger_active = c.danger_active;
    c.button_danger_foreground = c.danger_foreground;

    // Navigation.
    c.sidebar = p.surface_alt;
    c.sidebar_border = p.border;
    c.sidebar_foreground = p.ink;
    c.sidebar_accent = p.accent_soft;
    c.sidebar_accent_foreground = p.accent_dark;
    c.sidebar_primary = p.accent;
    c.sidebar_primary_foreground = p.on_fill;
    c.title_bar = p.surface_alt;
    c.title_bar_border = p.border;
    c.status_bar = p.surface_alt;
    c.status_bar_border = p.border;

    // Lists, tables, tabs.
    c.list = p.surface;
    c.list_even = p.surface;
    c.list_head = p.surface_alt;
    c.list_hover = p.track;
    c.list_active = p.accent_soft;
    c.list_active_border = p.accent;
    c.table = p.surface;
    c.table_even = p.surface;
    c.table_head = p.surface_alt;
    c.table_head_foreground = p.muted;
    c.table_foot = p.surface_alt;
    c.table_foot_foreground = p.muted;
    c.table_hover = p.track;
    c.table_active = p.accent_soft;
    c.table_active_border = p.accent;
    c.table_row_border = p.border;
    c.tab_bar = clear;
    c.tab = clear;
    c.tab_active = p.surface;
    c.tab_foreground = p.muted;
    c.tab_active_foreground = p.ink;
    c.tab_bar_segmented = p.track;

    // Controls and feedback.
    c.switch = p.bar_grey;
    c.switch_thumb = if dark { p.ink } else { p.surface };
    c.slider_bar = p.accent;
    c.slider_thumb = p.surface;
    c.progress_bar = p.accent;
    c.skeleton = p.track;
    c.scrollbar = clear;
    c.scrollbar_thumb = p.bar_grey.opacity(0.6);
    c.scrollbar_thumb_hover = p.bar_grey;

    // Charts and base hues.
    c.chart_1 = p.accent;
    c.chart_2 = p.ok;
    c.chart_3 = p.warn;
    c.chart_4 = p.danger;
    c.chart_5 = p.bar_grey;
    c.chart_bullish = p.ok;
    c.chart_bearish = p.danger;
    c.chart_grid = p.border;
    c.red = p.danger;
    c.red_light = p.danger_bg;
    c.green = p.ok;
    c.green_light = p.ok_bg;
    c.blue = p.accent;
    c.blue_light = p.accent_soft;
    c.yellow = p.warn;
    c.yellow_light = p.warn_bg;
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::ThemeColor;

    use super::{set_colors, shade};
    use crate::ui::tokens::{DARK, LIGHT};

    #[test]
    fn colors_come_from_the_palette() {
        for (p, dark) in [(&*LIGHT, false), (&*DARK, true)] {
            let mut c = ThemeColor::default();
            set_colors(&mut c, p, dark);
            assert_eq!(c.background, p.bg);
            assert_eq!(c.foreground, p.ink);
            assert_eq!(c.border, p.border);
            assert_eq!(c.muted_foreground, p.muted);
            assert_eq!(c.primary, p.accent);
            assert_eq!(c.primary_foreground, p.on_fill);
            assert_eq!(c.sidebar, p.surface_alt);
            assert_eq!(c.sidebar_accent, p.accent_soft);
            assert_eq!(c.warning, p.warn);
            assert_eq!(c.success, p.ok);
            assert_eq!(c.danger, p.danger);
            assert_eq!(c.link, p.accent);
            assert_eq!(c.ring, p.accent);
            // A button reads its own colour, not the base one: they agree.
            assert_eq!(c.button_primary, c.primary);
            assert_eq!(c.button_danger, c.danger);
            assert_eq!(c.button_secondary, c.secondary);
        }
    }

    #[test]
    fn pressed_states_move_away_from_the_page() {
        let c = LIGHT.warn;
        assert!(shade(c, false, 0.05).l < c.l, "darker on light");
        let d = DARK.warn;
        assert!(shade(d, true, 0.05).l > d.l, "lighter on dark");
        assert!((0.0..=1.0).contains(&shade(LIGHT.ink, false, 0.5).l));
    }
}
