//! Filesystem layout of the private app state.
//!
//! Everything lives under `$XDG_DATA_HOME/jobpilot` (override with `JOBPILOT_DATA_DIR`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Env var that overrides the data directory (used by tests).
pub const DATA_DIR_ENV: &str = "JOBPILOT_DATA_DIR";

/// File name of the master profile.
pub const PROFILE_FILE: &str = "profile.yaml";

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    /// SQLite database file.
    pub db: PathBuf,
    /// `applications/<slug>/{jd.md, cv.yaml, cv.pdf, page-*.png}`.
    pub applications_dir: PathBuf,
    /// `sessions/<id>/` browser profiles (mode 0700).
    pub sessions_dir: PathBuf,
    /// Screenshots.
    pub shots_dir: PathBuf,
}

impl Paths {
    /// Layout rooted at `data_dir` (no I/O).
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            db: data_dir.join("jobpilot.db"),
            applications_dir: data_dir.join("applications"),
            sessions_dir: data_dir.join("sessions"),
            shots_dir: data_dir.join("shots"),
            data_dir,
        }
    }

    /// `JOBPILOT_DATA_DIR` if set, else `<XDG data dir>/jobpilot`.
    pub fn from_env() -> Result<Self> {
        if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::new(dir));
        }
        let base = dirs::data_dir().context("cannot determine the user data directory")?;
        Ok(Self::new(base.join("jobpilot")))
    }

    /// Create all directories (`sessions/` is private to the user).
    pub fn ensure(&self) -> Result<()> {
        for dir in [&self.data_dir, &self.applications_dir, &self.sessions_dir, &self.shots_dir] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.sessions_dir, std::fs::Permissions::from_mode(0o700))
                .context("restricting sessions dir")?;
        }
        Ok(())
    }

    pub fn application_dir(&self, slug: &str) -> PathBuf {
        self.applications_dir.join(slug)
    }

    pub fn session_dir(&self, id: i64) -> PathBuf {
        self.sessions_dir.join(id.to_string())
    }

    /// Nearest `profile.yaml` walking up from the current directory,
    /// else `<data_dir>/profile.yaml`.
    pub fn default_profile_path(&self) -> PathBuf {
        std::env::current_dir()
            .ok()
            .and_then(|cwd| find_upwards(&cwd, PROFILE_FILE))
            .unwrap_or_else(|| self.data_dir.join(PROFILE_FILE))
    }
}

fn find_upwards(start: &Path, file: &str) -> Option<PathBuf> {
    start.ancestors().map(|dir| dir.join(file)).find(|p| p.is_file())
}
