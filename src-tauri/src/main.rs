//! JobPilot desktop app: the binary only starts the library (`lib.rs`, the composition root).

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> anyhow::Result<()> {
    jobpilot_lib::run()
}
