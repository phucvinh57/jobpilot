//! The engine: everything except the UI (no Tauri, no webview). The shell's contexts call it; it
//! never names them (`graph/tests.rs` checks that).
//!
//! Module ownership (so parallel work does not collide):
//! - `paths`, `runtime`, `store`, `secrets`, `facade` — P0 (foundation)
//! - `llm` — P1, `pipeline` — P2, `browser` — P4, `apply` — P5, `mail` — P6, `graph` — P8
//! - `notify`, `prefs` (and the store tables behind them) — P8, then P9+
//! - `trends` — P13 (collection; the digest follows in part B)
//! - `discover` — P10
//! - `net` — the shared safe HTTP client (moved out of `trends`)
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
pub mod redact;
pub mod runtime;
pub mod secrets;
pub mod store;
pub mod trends;

pub use facade::Core;
pub use paths::Paths;
pub use secrets::Secrets;
pub use store::Store;
