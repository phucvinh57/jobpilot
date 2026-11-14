//! Application services of this context: thin `#[tauri::command]`s named `profile_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.

use tauri::ipc::Invoke;

/// This context's commands; `main.rs` routes every `profile_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![]
}
