//! The user's profile: every fact a CV may state.
//!
//! The user gives JobPilot documents about their work: uploaded files (a CV as PDF, Markdown or
//! text) and notes they type ([`sources`]). The reader ([`extract`]) turns them, with the model,
//! into one profile in the CV schema ([`Cv`]), keeps only the facts it can trace back to the
//! documents ([`trace`]) and stores it. Every tailored CV is then checked against that profile by
//! the fact guard (`pipeline::guard`), as before. The user never edits the profile itself: to fix
//! it they change a document and read again.

pub mod extract;
pub mod sources;
pub mod trace;
pub mod typst;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde_yaml::{Mapping, Value};

use crate::pipeline::cv::Cv;
use crate::Core;

/// The error text when there is no profile yet; `health::classify` maps it to `Problem::NoProfile`.
pub const NO_PROFILE: &str = "no profile yet: add your CV or describe your work in Profile > About you";

/// The profile read from the user's documents.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub cv: Cv,
    /// The same profile as compact YAML (what the prompts see).
    pub yaml: String,
    pub built_at: DateTime<Utc>,
    /// The documents changed since it was read.
    pub stale: bool,
    /// Facts found in the documents that could not be traced, so left out.
    pub left_out: Vec<String>,
}

impl Core {
    /// The stored profile, if the documents were read at least once.
    pub fn profile(&self) -> Result<Option<Profile>> {
        let Some(stored) = self.store.stored_profile()? else { return Ok(None) };
        let cv = Cv::parse(&stored.yaml).context("the stored profile is not valid")?;
        let stale = stored.sources_hash != self.store.profile_sources_hash()?;
        Ok(Some(Profile { cv, yaml: stored.yaml, built_at: stored.built_at, stale, left_out: stored.left_out }))
    }

    /// The stored profile, or the [`NO_PROFILE`] error.
    pub fn require_profile(&self) -> Result<Profile> {
        self.profile()?.context(NO_PROFILE)
    }

    /// Store `cv` as the profile, as if it had been read from the current documents (demo data and
    /// tests; the reader uses the same path).
    pub fn save_profile_cv(&self, cv: &Cv, left_out: &[String]) -> Result<()> {
        let yaml = compact_yaml(cv)?;
        self.store.save_profile(&yaml, &self.store.profile_sources_hash()?, left_out)
    }

}

/// `cv` as YAML without empty values (no `url: ''`, `hidden: false`, `bullets: []`), so the
/// prompts read only what the profile says.
pub fn compact_yaml(cv: &Cv) -> Result<String> {
    let value = serde_yaml::to_value(cv).context("writing the profile")?;
    let value = prune(value).unwrap_or(Value::Mapping(Mapping::new()));
    serde_yaml::to_string(&value).context("writing the profile")
}

fn prune(value: Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(false) => None,
        Value::String(s) if s.trim().is_empty() => None,
        Value::Sequence(items) => {
            let items: Vec<Value> = items.into_iter().filter_map(prune).collect();
            (!items.is_empty()).then_some(Value::Sequence(items))
        }
        Value::Mapping(map) => {
            let map: Mapping = map.into_iter().filter_map(|(k, v)| prune(v).map(|v| (k, v))).collect();
            (!map.is_empty()).then_some(Value::Mapping(map))
        }
        other => Some(other),
    }
}

/// A test core holding `profile_yaml` as its profile (read from one note with the same text).
#[cfg(test)]
pub fn test_core_with_profile(profile_yaml: &str) -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().expect("temp dir");
    let core = Core::open_at(crate::Paths::new(dir.path().join("jp")), crate::Secrets::memory())
        .expect("open core");
    set_test_profile(&core, profile_yaml);
    (dir, core)
}

/// Store `profile_yaml` as `core`'s profile, read from one note with the same text (tests; the
/// shell's tests get it with the `test-util` feature).
#[cfg(any(test, feature = "test-util"))]
pub fn set_test_profile(core: &Core, profile_yaml: &str) {
    use crate::store::NewProfileSource;
    core.store.add_profile_source(&NewProfileSource::text("Profile", profile_yaml), usize::MAX).expect("add source");
    let cv = Cv::parse(profile_yaml).expect("valid profile");
    core.save_profile_cv(&cv, &[]).expect("save profile");
}

/// The profile the tests use (a full, real one).
#[cfg(any(test, feature = "test-util"))]
pub const TEST_PROFILE: &str = include_str!("../../tests/fixtures/profile.yaml");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_profile_until_one_is_saved_and_it_goes_stale_when_the_documents_change() {
        let dir = tempfile::tempdir().unwrap();
        let core =
            Core::open_at(crate::Paths::new(dir.path().join("jp")), crate::Secrets::memory()).unwrap();
        assert_eq!(core.profile().unwrap(), None);
        let err = core.require_profile().unwrap_err();
        assert_eq!(crate::health::classify(&format!("{err:#}")), crate::health::Problem::NoProfile);

        set_test_profile(&core, TEST_PROFILE);
        let profile = core.require_profile().unwrap();
        assert!(!profile.stale);
        assert_eq!(profile.cv.basics.name, "Vinh Nguyen Phuc");

        core.store
            .add_profile_source(&crate::store::NewProfileSource::text("More", "Also Go"), usize::MAX)
            .unwrap();
        assert!(core.require_profile().unwrap().stale);
    }

    #[test]
    fn compact_yaml_drops_empty_values_and_parses_back_the_same() {
        let cv = Cv::parse(TEST_PROFILE).unwrap();
        let yaml = compact_yaml(&cv).unwrap();
        assert!(!yaml.contains("hidden: false") && !yaml.contains("''") && !yaml.contains("[]"), "{yaml}");
        assert_eq!(Cv::parse(&yaml).unwrap(), cv);
    }
}
