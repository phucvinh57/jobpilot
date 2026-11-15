//! The engine: everything except the UI (no Tauri, no webview). The shell's contexts call it; it
//! never names them (`graph/tests.rs` checks that).
//!
//! - `paths`, `runtime`, `store`, `secrets`, `redact`, `logging`, `health`, `facade`: foundation
//! - `llm`: AI providers; `pipeline`: JD -> tailored CV -> PDF, job insight
//! - `browser`: managed Chrome; `apply`: review -> confirm -> submit; `mail`: Gmail status
//! - `graph`: flows as resumable graphs; `notify`: desktop notifications
//! - `prefs`: job-search preferences; `profile`: the user's documents and the profile read from them
//! - `discover`: job sources -> scored leads; `trends`: market trends and the digest
//! - `net`: the one safe HTTP client for third-party data
//!
//! Extend [`Core`] from your own module with `impl Core { ... }` instead of editing `facade.rs`.

pub mod apply;
pub mod browser;
pub mod discover;
pub mod facade;
pub mod graph;
pub mod health;
pub mod llm;
pub mod logging;
pub mod mail;
pub mod net;
pub mod notify;
pub mod paths;
pub mod pipeline;
pub mod prefs;
pub mod profile;
pub mod redact;
pub mod runtime;
pub mod secrets;
pub mod store;
pub mod trends;

pub use facade::Core;
pub use paths::Paths;
pub use secrets::Secrets;
pub use store::Store;
