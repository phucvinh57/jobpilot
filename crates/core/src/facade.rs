//! The single entry point the UI (and other crates) use.

use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::{Paths, Secrets, Store};

/// kv key holding the effective profile path.
pub const PROFILE_PATH_KEY: &str = "profile_path";

/// Shared app state. Wrap in `Arc` to share: `Arc::new(Core::open()?)`.
/// Sub-plans add behaviour with `impl Core { .. }` in their own module.
pub struct Core {
    pub paths: Paths,
    pub store: Store,
    pub secrets: Secrets,
}

impl Core {
    /// Open the real app state (`JOBPILOT_DATA_DIR` / XDG data dir, keyring or `JOBPILOT_SECRETS=memory`).
    pub fn open() -> Result<Self> {
        Self::open_at(Paths::from_env()?, Secrets::from_env())
    }

    /// Open with explicit paths/secrets (tests). Creates directories and migrates the database.
    pub fn open_at(paths: Paths, secrets: Secrets) -> Result<Self> {
        paths.ensure()?;
        let store = Store::open(&paths.db)?;
        Ok(Self { paths, store, secrets })
    }

    /// The effective profile path: kv `profile_path`, else [`Paths::default_profile_path`].
    pub fn profile_path(&self) -> PathBuf {
        match self.store.kv_get(PROFILE_PATH_KEY) {
            Ok(Some(p)) if !p.is_empty() => PathBuf::from(p),
            _ => self.paths.default_profile_path(),
        }
    }

    pub fn set_profile_path(&self, path: &Path) -> Result<()> {
        self.store.kv_set(PROFILE_PATH_KEY, &path.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_layout_and_profile_path_setting() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        assert!(core.paths.db.is_file());
        assert!(core.paths.applications_dir.is_dir());
        assert!(core.paths.sessions_dir.is_dir());
        assert!(core.paths.shots_dir.is_dir());

        core.set_profile_path(Path::new("/x/profile.yaml")).unwrap();
        assert_eq!(core.profile_path(), PathBuf::from("/x/profile.yaml"));
    }

    #[test]
    fn runtime_spawn_delivers_result() {
        let rx = crate::runtime::spawn(async { 21 * 2 });
        assert_eq!(crate::runtime::block_on(rx).unwrap(), 42);
    }
}
