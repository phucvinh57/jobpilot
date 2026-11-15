//! Shared kernel: what the bounded contexts use and none of them owns. The command error with its
//! `Problem`, the event bridge, the company brief read model, the preference-list rules and the
//! app shell's commands.
//!
//! Contexts depend on this module and on `crate::engine`, never on each other.

pub mod error;
pub mod events;
pub mod insight;
pub mod shell;
pub mod topics;

pub use error::{run, run_sync, CommandError, CommandResult, ProblemDto};
pub use insight::InsightDto;
