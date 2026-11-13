//! JobPilot core: everything except the UI.
//!
//! Module ownership (so parallel work does not collide):
//! - `paths`, `runtime`, `store`, `secrets`, `facade` — P0 (foundation)
//! - `llm` — P1, `pipeline` — P2, `browser` — P4, `apply` — P5, `mail` — P6
//!
//! Extend [`Core`] from your own module with `impl Core { ... }` instead of editing `facade.rs`.

pub mod apply;
pub mod browser;
pub mod facade;
pub mod llm;
pub mod mail;
pub mod paths;
pub mod pipeline;
pub mod runtime;
pub mod secrets;
pub mod store;

pub use chrono;
pub use facade::Core;
pub use paths::Paths;
pub use secrets::Secrets;
pub use store::Store;
