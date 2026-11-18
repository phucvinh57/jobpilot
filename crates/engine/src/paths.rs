//! Filesystem layout of the private app state.
//!
//! Everything lives under the OS user data directory (override with `JOBPILOT_DATA_DIR`).

use std::path::PathBuf;

use anyhow::{Context, Result};

/// Env var that overrides the data directory (used by tests).
pub const DATA_DIR_ENV: &str = "JOBPILOT_DATA_DIR";

#[derive(Clone, Debug)]
pub struct Paths {
    pub data_dir: PathBuf,
    /// SQLite database file.
    pub db: PathBuf,
    /// `applications/<slug>/{jd.md, cv.yaml, cv.pdf, page-*.png}`.
    pub applications_dir: PathBuf,
    /// Screenshots.
    pub shots_dir: PathBuf,
    /// `logs/jobpilot.log.<date>` (no secrets, see `logging`).
    pub logs_dir: PathBuf,
}

impl Paths {
    /// Layout rooted at `data_dir` (no I/O).
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        let data_dir = data_dir.into();
        Self {
            db: data_dir.join("jobpilot.db"),
            applications_dir: data_dir.join("applications"),
            shots_dir: data_dir.join("shots"),
            logs_dir: data_dir.join("logs"),
            data_dir,
        }
    }

    /// `JOBPILOT_DATA_DIR` if set, else `<user data dir>/jobpilot` (`%APPDATA%\jobpilot` on Windows).
    pub fn from_env() -> Result<Self> {
        if let Some(dir) = std::env::var_os(DATA_DIR_ENV).filter(|v| !v.is_empty()) {
            return Ok(Self::new(dir));
        }
        let base = dirs::data_dir().context("cannot determine the user data directory")?;
        Ok(Self::new(base.join("jobpilot")))
    }

    /// Create all directories.
    pub fn ensure(&self) -> Result<()> {
        for dir in [&self.data_dir, &self.applications_dir, &self.shots_dir, &self.logs_dir] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(())
    }

    pub fn application_dir(&self, slug: &str) -> PathBuf {
        self.applications_dir.join(slug)
    }

}
