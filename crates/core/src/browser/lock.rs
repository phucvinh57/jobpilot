//! One live browser per profile directory.
//!
//! The lock is an exclusive advisory lock (`flock`) on `<profile>/.jobpilot.lock`, held by the open file
//! descriptor. The kernel drops it when the owner exits or crashes, so a lock file left behind by a dead
//! process is never "held": it is simply re-acquired (and the PID inside rewritten). The PID is only
//! diagnostic, used to tell the user who holds the profile.

use std::fmt;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Seek, Write};
use std::path::{Path, PathBuf};

/// Lock file name inside a session profile directory.
pub const LOCK_FILE: &str = ".jobpilot.lock";

#[derive(Debug)]
pub enum LockError {
    /// Another live browser (or process) owns the profile. Holds its PID when readable.
    Held(Option<u32>),
    Io(io::Error),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held(Some(pid)) => {
                write!(f, "this browser profile is already open (process {pid}); close it first")
            }
            LockError::Held(None) => write!(f, "this browser profile is already open; close it first"),
            LockError::Io(e) => write!(f, "cannot lock the browser profile: {e}"),
        }
    }
}

impl std::error::Error for LockError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LockError::Io(e) => Some(e),
            LockError::Held(_) => None,
        }
    }
}

/// Exclusive ownership of a profile directory; released on drop.
#[derive(Debug)]
pub struct ProfileLock {
    file: File,
    path: PathBuf,
}

impl ProfileLock {
    /// Take the lock for `dir` (which must exist) or fail with [`LockError::Held`].
    pub fn acquire(dir: &Path) -> Result<Self, LockError> {
        let path = dir.join(LOCK_FILE);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(LockError::Io)?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(LockError::Held(read_pid(&path))),
            Err(TryLockError::Error(e)) => return Err(LockError::Io(e)),
        }
        // We own it now: overwrite whatever PID a previous (dead) owner left.
        file.set_len(0).map_err(LockError::Io)?;
        file.rewind().map_err(LockError::Io)?;
        write!(file, "{}", std::process::id()).map_err(LockError::Io)?;
        file.flush().map_err(LockError::Io)?;
        Ok(Self { file, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ProfileLock {
    fn drop(&mut self) {
        // Closing the descriptor releases the lock too; unlock explicitly so it is immediate.
        let _ = self.file.unlock();
    }
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acquire_then_double_acquire_fails() {
        let dir = tempfile::tempdir().unwrap();
        let first = ProfileLock::acquire(dir.path()).unwrap();
        assert!(first.path().is_file());
        match ProfileLock::acquire(dir.path()) {
            Err(LockError::Held(pid)) => assert_eq!(pid, Some(std::process::id())),
            other => panic!("expected Held, got {other:?}"),
        }
    }

    #[test]
    fn released_on_drop_can_be_reacquired() {
        let dir = tempfile::tempdir().unwrap();
        drop(ProfileLock::acquire(dir.path()).unwrap());
        ProfileLock::acquire(dir.path()).unwrap();
    }

    #[test]
    fn stale_lock_file_of_dead_process_is_cleared() {
        let dir = tempfile::tempdir().unwrap();
        // A lock file left by a crashed owner: PID that cannot exist, nobody holds the flock.
        std::fs::write(dir.path().join(LOCK_FILE), "4294967294").unwrap();
        let lock = ProfileLock::acquire(dir.path()).unwrap();
        let pid = std::fs::read_to_string(lock.path()).unwrap();
        assert_eq!(pid, std::process::id().to_string());
    }

    #[test]
    fn garbage_lock_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(LOCK_FILE), "not a pid\n\n\n").unwrap();
        ProfileLock::acquire(dir.path()).unwrap();
    }
}
