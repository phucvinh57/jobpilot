//! Design tokens: the light and dark palettes, radii, sizes and the type scale.
//!
//! Colours come from the approved design (`LIGHT`) and a derived dark set (`DARK`). Views read
//! them through [`palette`], and [`super::theme::apply_palette`] maps the same values onto
//! gpui-component's `Theme`, so stock widgets (inputs, checkboxes, tabs) follow the design too.

// The design's tokens are all defined here; screens adopt them one by one (Phase 2).
#![allow(dead_code)]

use std::sync::LazyLock;

use gpui_kit::component::{ActiveTheme as _, ThemeMode};
use gpui_kit::{rgb, App, Hsla};

/// Radius of cards and section cards.
pub const RADIUS_CARD: f32 = 14.0;
/// Radius of buttons and text inputs.
pub const RADIUS_BUTTON: f32 = 10.0;
/// Radius of chips (skills, filters).
pub const RADIUS_CHIP: f32 = 8.0;
/// Radius of status tags.
pub const RADIUS_TAG: f32 = 6.0;

/// Minimum height of a button (a comfortable touch target).
pub const BUTTON_HEIGHT: f32 = 40.0;
/// Minimum height of a navigation row.
pub const NAV_HEIGHT: f32 = 40.0;
/// Inner padding of a card.
pub const CARD_PADDING: f32 = 16.0;
/// Gap between blocks inside a card or a page.
pub const GAP: f32 = 12.0;

/// Type scale (px): page title, card title, body text, captions and chips.
pub const TEXT_H1: f32 = 30.0;
pub const TEXT_H2: f32 = 18.0;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_SMALL: f32 = 12.0;

/// Body font (Inter), also set as the theme's font family.
pub const FONT_BODY: &str = "Inter";
/// Headings (Space Grotesk).
pub const FONT_HEADING: &str = "Space Grotesk";
/// The CV preview (Source Serif 4).
pub const FONT_SERIF: &str = "Source Serif 4";

/// Every colour of the design for one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Page background.
    pub bg: Hsla,
    /// Cards, inputs, popovers.
    pub surface: Hsla,
    /// Navigation and table heads: slightly off the surface.
    pub surface_alt: Hsla,
    pub border: Hsla,
    /// The edge of controls (buttons, inputs): a step darker than `border`, so a white control
    /// stays visible on a white card.
    pub control_border: Hsla,
    /// Primary text.
    pub ink: Hsla,
    /// Secondary text.
    pub muted: Hsla,
    /// Tertiary text (hints, timestamps); still readable.
    pub subtle: Hsla,
    pub accent: Hsla,
    /// Hover/pressed accent, and text on `accent_soft`.
    pub accent_dark: Hsla,
    pub accent_soft: Hsla,
    /// Text and icons on a saturated fill (`accent`, `ok`, `warn`, `danger`).
    pub on_fill: Hsla,
    pub warn: Hsla,
    /// Text on `warn_bg`.
    pub warn_fg: Hsla,
    pub warn_bg: Hsla,
    pub ok: Hsla,
    pub ok_fg: Hsla,
    pub ok_bg: Hsla,
    pub danger: Hsla,
    pub danger_fg: Hsla,
    pub danger_bg: Hsla,
    /// Neutral chip and bar-track fill.
    pub track: Hsla,
    /// A bar that is not highlighted (a skill not in the profile yet).
    pub bar_grey: Hsla,
}

/// `0xRRGGBB` as a colour.
fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

fn light() -> Palette {
    Palette {
        bg: hex(0xF6F6F3),
        surface: hex(0xFFFFFF),
        surface_alt: hex(0xFBFBF9),
        border: hex(0xE4E4DF),
        control_border: hex(0xD6D6D0),
        ink: hex(0x17181C),
        muted: hex(0x5D6068),
        subtle: hex(0x64676F),
        accent: hex(0x2A4FD6),
        accent_dark: hex(0x1C3AA8),
        accent_soft: hex(0xE8EDFC),
        on_fill: hex(0xFFFFFF),
        warn: hex(0xB4540A),
        warn_fg: hex(0x8A4108),
        warn_bg: hex(0xFDF3E7),
        ok: hex(0x1F7A4D),
        ok_fg: hex(0x17603C),
        ok_bg: hex(0xE6F4EC),
        danger: hex(0xB42318),
        danger_fg: hex(0x912018),
        danger_bg: hex(0xFDECEA),
        track: hex(0xEFEFEB),
        bar_grey: hex(0xB9BAB4),
    }
}

fn dark() -> Palette {
    Palette {
        bg: hex(0x121317),
        surface: hex(0x1A1B20),
        surface_alt: hex(0x16171B),
        border: hex(0x2A2C33),
        control_border: hex(0x3A3D45),
        ink: hex(0xECECEA),
        muted: hex(0xA3A6AE),
        subtle: hex(0x9396A0),
        accent: hex(0x6F8BFF),
        accent_dark: hex(0x9AAEFF),
        accent_soft: hex(0x1E2647),
        on_fill: hex(0x0E1220),
        warn: hex(0xF0A050),
        warn_fg: hex(0xF6C08A),
        warn_bg: hex(0x3A2614),
        ok: hex(0x4CC38A),
        ok_fg: hex(0x8FDDB5),
        ok_bg: hex(0x14301F),
        danger: hex(0xF4796E),
        danger_fg: hex(0xF9A39B),
        danger_bg: hex(0x3A1816),
        track: hex(0x262830),
        bar_grey: hex(0x5A5D66),
    }
}

pub static LIGHT: LazyLock<Palette> = LazyLock::new(light);
pub static DARK: LazyLock<Palette> = LazyLock::new(dark);

/// The palette of a theme mode.
pub fn palette_for(mode: ThemeMode) -> &'static Palette {
    if mode.is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

/// The palette of the appearance showing now (light or dark; System is already settled on one).
pub fn palette(cx: &App) -> &'static Palette {
    palette_for(cx.theme().mode)
}

/// What a status or a banner means. Each tone is a foreground / background pair of the palette.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Nothing to say: closed, unknown, draft.
    Neutral,
    /// Information, in progress, selected.
    Accent,
    /// Needs the user.
    Warn,
    /// Good: matched, submitted, healthy.
    Ok,
    /// Failed, blocked, rejected.
    Danger,
}

/// The colours of a [`Tone`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ToneColors {
    /// Text and icons.
    pub fg: Hsla,
    /// Tag and banner fill.
    pub bg: Hsla,
    /// Strong colour for dots, bars and borders.
    pub dot: Hsla,
}

impl Palette {
    pub fn tone(&self, tone: Tone) -> ToneColors {
        match tone {
            Tone::Neutral => ToneColors { fg: self.muted, bg: self.track, dot: self.bar_grey },
            Tone::Accent => ToneColors { fg: self.accent_dark, bg: self.accent_soft, dot: self.accent },
            Tone::Warn => ToneColors { fg: self.warn_fg, bg: self.warn_bg, dot: self.warn },
            Tone::Ok => ToneColors { fg: self.ok_fg, bg: self.ok_bg, dot: self.ok },
            Tone::Danger => ToneColors { fg: self.danger_fg, bg: self.danger_bg, dot: self.danger },
        }
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::ThemeMode;
    use gpui_kit::{rgb, Hsla, Rgba};

    use super::{palette_for, Palette, Tone, DARK, LIGHT};

    /// WCAG relative luminance of a colour.
    fn luminance(color: Hsla) -> f32 {
        let c: Rgba = color.into();
        let lin = |v: f32| if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * lin(c.r) + 0.7152 * lin(c.g) + 0.0722 * lin(c.b)
    }

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn palette_follows_the_mode() {
        assert_eq!(palette_for(ThemeMode::Light), &*LIGHT);
        assert_eq!(palette_for(ThemeMode::Dark), &*DARK);
        assert_ne!(LIGHT.bg, DARK.bg);
    }

    #[test]
    fn light_values_are_the_design_hexes() {
        let want: Hsla = rgb(0xF6F6F3).into();
        assert_eq!(LIGHT.bg, want);
        let accent: Hsla = rgb(0x2A4FD6).into();
        assert_eq!(LIGHT.accent, accent);
        let dark_bg: Hsla = rgb(0x121317).into();
        assert_eq!(DARK.bg, dark_bg);
    }

    #[test]
    fn tone_maps_to_palette_pairs() {
        for p in [&*LIGHT, &*DARK] {
            assert_eq!(p.tone(Tone::Neutral).bg, p.track);
            assert_eq!(p.tone(Tone::Accent).fg, p.accent_dark);
            assert_eq!(p.tone(Tone::Accent).bg, p.accent_soft);
            assert_eq!(p.tone(Tone::Warn).fg, p.warn_fg);
            assert_eq!(p.tone(Tone::Warn).bg, p.warn_bg);
            assert_eq!(p.tone(Tone::Ok).dot, p.ok);
            assert_eq!(p.tone(Tone::Danger).bg, p.danger_bg);
        }
    }

    /// Every text colour reads on the surface it is drawn on, in both appearances (WCAG AA).
    #[test]
    fn text_contrast_is_at_least_4_5() {
        fn check(name: &str, p: &Palette) {
            let mut pairs = vec![
                ("ink on bg", p.ink, p.bg),
                ("ink on surface", p.ink, p.surface),
                ("muted on bg", p.muted, p.bg),
                ("muted on surface", p.muted, p.surface),
                ("subtle on bg", p.subtle, p.bg),
                ("subtle on surface", p.subtle, p.surface),
                ("muted on surface_alt", p.muted, p.surface_alt),
                ("accent on surface", p.accent, p.surface),
                ("on_fill on accent", p.on_fill, p.accent),
                ("on_fill on ok", p.on_fill, p.ok),
                ("on_fill on danger", p.on_fill, p.danger),
                ("on_fill on warn", p.on_fill, p.warn),
            ];
            for tone in [Tone::Neutral, Tone::Accent, Tone::Warn, Tone::Ok, Tone::Danger] {
                let c = p.tone(tone);
                pairs.push(("tone fg on bg", c.fg, c.bg));
            }
            for (what, fg, bg) in pairs {
                let ratio = contrast(fg, bg);
                assert!(ratio >= 4.5, "{name}: {what} has contrast {ratio:.2}");
            }
        }
        check("light", &LIGHT);
        check("dark", &DARK);
    }
}
