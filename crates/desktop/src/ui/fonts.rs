//! The bundled fonts (`assets/fonts`, SIL OFL, see `assets/fonts/OFL.txt`): Inter for text, Space
//! Grotesk for headings and Source Serif 4 for the CV preview.
//!
//! They are static instances (one file per weight): the text renderer picks a weight by font
//! face and does not apply variable-font axes. Registered once at startup, before the first frame.

use std::borrow::Cow;

use gpui_kit::App;

/// Inter 400 / 500 / 600.
const INTER: [&[u8]; 3] = [
    include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf"),
];
/// Space Grotesk 500 / 600.
const SPACE_GROTESK: [&[u8]; 2] = [
    include_bytes!("../../../../assets/fonts/SpaceGrotesk-Medium.ttf"),
    include_bytes!("../../../../assets/fonts/SpaceGrotesk-SemiBold.ttf"),
];
/// Source Serif 4 400 / 600.
const SOURCE_SERIF: [&[u8]; 2] = [
    include_bytes!("../../../../assets/fonts/SourceSerif4-Regular.ttf"),
    include_bytes!("../../../../assets/fonts/SourceSerif4-SemiBold.ttf"),
];

/// Every embedded font file.
fn all() -> Vec<Cow<'static, [u8]>> {
    INTER.iter().chain(&SPACE_GROTESK).chain(&SOURCE_SERIF).map(|bytes| Cow::Borrowed(*bytes)).collect()
}

/// Register the fonts with the text system. A failure is not fatal: text falls back to the system
/// font.
pub fn register(cx: &App) {
    let fonts = all();
    let count = fonts.len();
    match cx.text_system().add_fonts(fonts) {
        Ok(()) => tracing::info!("registered {count} bundled fonts"),
        Err(e) => tracing::warn!("registering the bundled fonts failed: {e:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::all;

    #[test]
    fn every_font_is_a_truetype_file() {
        let fonts = all();
        assert_eq!(fonts.len(), 7);
        for bytes in fonts {
            // TrueType outlines start with sfnt version 0x00010000.
            assert_eq!(bytes.get(..4), Some(&[0, 1, 0, 0][..]));
        }
    }
}
