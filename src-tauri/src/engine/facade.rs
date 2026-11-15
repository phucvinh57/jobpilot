//! The single entry point the UI (the shell's contexts) uses.

use anyhow::Result;

use crate::engine::graph::Graphs;
use crate::engine::{Paths, Secrets, Store};

/// Shared app state. Wrap in `Arc` to share: `Arc::new(Core::open()?)`.
/// Sub-plans add behaviour with `impl Core { .. }` in their own module.
pub struct Core {
    pub paths: Paths,
    pub store: Store,
    pub secrets: Secrets,
    /// The graph runtime: runs, resumes and reports the flows (see [`crate::engine::graph`]).
    pub graphs: Graphs,
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
        let graphs = Graphs::new(store.clone());
        Ok(Self { paths, store, secrets, graphs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_the_layout() {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        assert!(core.paths.db.is_file());
        assert!(core.paths.applications_dir.is_dir());
        assert!(core.paths.sessions_dir.is_dir());
        assert!(core.paths.shots_dir.is_dir());
    }

    #[test]
    fn runtime_spawn_delivers_result() {
        let rx = crate::engine::runtime::spawn(async { 21 * 2 });
        assert_eq!(crate::engine::runtime::block_on(rx).unwrap(), 42);
    }
}
