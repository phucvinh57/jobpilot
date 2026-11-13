//! Secrets (API keys, OAuth refresh tokens) live ONLY in the OS keyring, service `jobpilot`.
//! Never put them in the database, logs or files.
//!
//! Set `JOBPILOT_SECRETS=memory` (or use [`Secrets::memory`]) for an in-memory store in tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};

pub const SERVICE: &str = "jobpilot";

/// Env var selecting the backend: `memory` for tests, anything else = keyring.
pub const BACKEND_ENV: &str = "JOBPILOT_SECRETS";

#[derive(Clone)]
pub struct Secrets {
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    Keyring,
    Memory(Arc<Mutex<HashMap<String, String>>>),
}

impl Secrets {
    /// OS keyring backend.
    pub fn keyring() -> Self {
        Self { backend: Backend::Keyring }
    }

    /// In-memory backend (not shared with other instances).
    pub fn memory() -> Self {
        Self { backend: Backend::Memory(Arc::default()) }
    }

    /// Memory backend if `JOBPILOT_SECRETS=memory`, else the keyring.
    pub fn from_env() -> Self {
        match std::env::var(BACKEND_ENV).as_deref() {
            Ok("memory") => Self::memory(),
            _ => Self::keyring(),
        }
    }

    /// `Ok(None)` when the key is not set.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        let value = match &self.backend {
            Backend::Memory(m) => Ok(lock(m).get(key).cloned()),
            Backend::Keyring => match entry(key)?.get_password() {
                Ok(v) => Ok(Some(v)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(e).with_context(|| format!("reading secret `{key}` from the keyring")),
            },
        };
        // Whatever leaves the keyring is scrubbed from logs from now on.
        if let Ok(Some(v)) = &value {
            crate::redact::register(v);
        }
        value
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        crate::redact::register(value);
        match &self.backend {
            Backend::Memory(m) => {
                lock(m).insert(key.to_string(), value.to_string());
                Ok(())
            }
            Backend::Keyring => entry(key)?
                .set_password(value)
                .with_context(|| format!("writing secret `{key}` to the keyring")),
        }
    }

    /// Deleting a missing key is not an error.
    pub fn delete(&self, key: &str) -> Result<()> {
        match &self.backend {
            Backend::Memory(m) => {
                lock(m).remove(key);
                Ok(())
            }
            Backend::Keyring => match entry(key)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e).with_context(|| format!("deleting secret `{key}` from the keyring")),
            },
        }
    }
}

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).with_context(|| format!("opening keyring entry `{key}`"))
}

fn lock(m: &Mutex<HashMap<String, String>>) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_backend_round_trip() {
        let s = Secrets::memory();
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        s.delete("k").unwrap();
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
    }
}
