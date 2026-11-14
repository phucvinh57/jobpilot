//! Shared kernel: what every bounded context uses and none of them owns. The command error with
//! its `Problem`, the event bridge, the company brief read model and the app shell's commands.
//!
//! Contexts depend on this module and on `jobpilot_core`, never on each other.

// Scaffold: the contexts are being ported and do not use every helper yet (removed at integration).
#![allow(dead_code, unused_imports)]

pub mod error;
pub mod events;
pub mod insight;
pub mod shell;

pub use error::{run, run_sync, CommandError, CommandResult, ProblemDto};
pub use events::Scope;
pub use insight::InsightDto;
