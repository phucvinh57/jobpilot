//! The design kit: tokens (palettes, radii, type scale), the theme mapping, bundled fonts and the
//! components every screen is built from. Pure render, no I/O.
//!
//! - [`tokens`]: `Palette` (light and dark), `palette(cx)`, `Tone`, sizes.
//! - [`theme`]: `set_mode` / `sync_system` switch light/dark and put the palette onto gpui-component's `Theme` (stock widgets follow).
//! - [`fonts`]: Inter, Space Grotesk and Source Serif 4, registered at startup.
//! - [`components`]: `heading`, `card`, `section`, `banner`, buttons, `chip`, `tag`, `nav_item`,
//!   `field`, `segmented`, `score_badge`, `fit_bar`, `stepper`, `kanban_card`, ...

pub mod components;
pub mod fonts;
pub mod theme;
pub mod tokens;

#[allow(unused_imports)] // Phase 2 screens import the rest of the kit from here
pub use components::*;
#[allow(unused_imports)]
pub use tokens::{palette, Palette, Tone};
