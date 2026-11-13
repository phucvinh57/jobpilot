//! JobPilot core: everything except the UI.
//!
//! Module ownership (so parallel work does not collide):
//! - `paths`, `runtime`, `store`, `secrets`, `facade` — P0 (foundation)
//! - `llm` — P1, `pipeline` — P2, `browser` — P4, `apply` — P5, `mail` — P6
//! - `notify`, `prefs` (and the store tables behind them) — P8, then P9+
//!
//! Extend [`Core`] from your own module with `impl Core { ... }` instead of editing `facade.rs`.

pub mod apply;
pub mod browser;
pub mod facade;
pub mod health;
pub mod llm;
pub mod logging;
pub mod mail;
pub mod notify;
pub mod paths;
pub mod pipeline;
pub mod prefs;
pub mod redact;
pub mod runtime;
pub mod secrets;
pub mod store;

pub use chrono;
pub use facade::Core;
pub use paths::Paths;
pub use secrets::Secrets;
pub use store::Store;
