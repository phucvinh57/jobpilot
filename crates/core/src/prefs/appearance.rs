//! The app's colour scheme choice, kept in the kv store so it survives a restart.

use anyhow::{Context, Result};

use crate::Core;

/// kv key of the saved choice.
pub const APPEARANCE_KEY: &str = "ui.appearance";

/// Light, dark, or whatever the operating system uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    /// Every choice, in the order a picker shows them.
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "System",
            Appearance::Light => "Light",
            Appearance::Dark => "Dark",
        }
    }

    /// `system` / `light` / `dark` (as stored).
    pub fn as_str(self) -> &'static str {
        match self {
            Appearance::System => "system",
            Appearance::Light => "light",
            Appearance::Dark => "dark",
        }
    }

    /// Inverse of [`Appearance::as_str`] (ignores case and surrounding spaces); `None` for anything else.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "system" => Some(Appearance::System),
            "light" => Some(Appearance::Light),
            "dark" => Some(Appearance::Dark),
            _ => None,
        }
    }
}

impl Core {
    /// The saved choice; [`Appearance::System`] when none was saved or the saved value is unknown.
    pub fn appearance(&self) -> Result<Appearance> {
        let saved = self.store.kv_get(APPEARANCE_KEY).context("reading the appearance setting")?;
        Ok(saved.as_deref().and_then(Appearance::parse).unwrap_or_default())
    }

    /// Save the choice.
    pub fn set_appearance(&self, appearance: Appearance) -> Result<()> {
        self.store.kv_set(APPEARANCE_KEY, appearance.as_str()).context("saving the appearance setting")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Paths, Secrets};

    #[test]
    fn the_choice_is_saved_and_unknown_values_mean_system() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        assert_eq!(core.appearance().unwrap(), Appearance::System, "nothing saved yet");

        for a in Appearance::ALL {
            core.set_appearance(a).unwrap();
            assert_eq!(core.appearance().unwrap(), a);
            assert_eq!(core.store.kv_get(APPEARANCE_KEY).unwrap().as_deref(), Some(a.as_str()));
        }

        core.store.kv_set(APPEARANCE_KEY, "solarized").unwrap();
        assert_eq!(core.appearance().unwrap(), Appearance::System, "an unknown value falls back");
        core.store.kv_set(APPEARANCE_KEY, "").unwrap();
        assert_eq!(core.appearance().unwrap(), Appearance::System);
    }

    #[test]
    fn labels_and_names_round_trip() {
        for a in Appearance::ALL {
            assert_eq!(Appearance::parse(a.as_str()), Some(a));
            assert!(!a.label().is_empty());
        }
        assert_eq!(Appearance::parse(" Dark "), Some(Appearance::Dark));
        assert_eq!(Appearance::parse("auto"), None);
        assert_eq!(Appearance::default(), Appearance::System);
    }
}
