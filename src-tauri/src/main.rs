//! JobPilot desktop app (Tauri). The composition root: it opens `Core`, wires the bounded contexts'
//! commands and the event bridge, and opens the one window. All logic lives in `jobpilot-core`;
//! the views are the web app in `../src`.

// No console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod applications;
mod demo;
#[cfg(feature = "dev-tools")]
mod devtools;
mod discovery;
mod integrations;
mod profile;
mod shared;
mod trends;

use std::sync::Arc;

use anyhow::Context as _;
use jobpilot_core::{logging, mail, Core, Paths, Secrets};
use tauri::ipc::Invoke;
use tauri::webview::PageLoadEvent;
use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

/// The one window's label (named in `capabilities/default.json`).
const WINDOW: &str = "main";

fn main() -> anyhow::Result<()> {
    // Logs go to stderr and `<data dir>/logs/`, with secrets redacted.
    let paths = Paths::from_env()?;
    let _log = logging::init(&paths);
    tracing::info!(
        "JobPilot {} starting, data in {}",
        env!("CARGO_PKG_VERSION"),
        paths.data_dir.display()
    );

    let core = Arc::new(Core::open_at(paths, Secrets::from_env())?);
    // Sample data for UI work: `JOBPILOT_DEMO=1 cargo run -p jobpilot` (never on by default).
    if std::env::var_os("JOBPILOT_DEMO").is_some() {
        demo::seed(&core)?;
    }

    // Gmail tracking: poll for replies while the app runs (no-op unless Gmail is connected).
    mail::autostart(&core);

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(core.clone())
        .invoke_handler(route())
        .setup(move |app| {
            shared::events::bridge(app.handle(), &core);
            discovery::setup(app.handle(), &core);
            applications::setup(app.handle(), &core);
            profile::setup(app.handle(), &core);
            integrations::setup(app.handle(), &core);
            trends::setup(app.handle(), &core);
            WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::default())
                .title("JobPilot")
                .inner_size(1100.0, 720.0)
                .min_inner_size(820.0, 560.0)
                .center()
                // The webview shows the app only: links open in the system browser (`shared_open_url`).
                .on_navigation(|url| {
                    let allowed = may_show(url.scheme(), url.host_str());
                    if !allowed {
                        // Scheme and host only: a URL may carry a token.
                        tracing::warn!(
                            "blocked navigation to {}://{}",
                            url.scheme(),
                            url.host_str().unwrap_or("")
                        );
                    }
                    allowed
                })
                .on_page_load(|_window, payload| {
                    if payload.event() == PageLoadEvent::Finished {
                        tracing::info!("the window is ready");
                        #[cfg(feature = "dev-tools")]
                        devtools::run_script(&_window);
                    }
                })
                .build()?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .context("JobPilot cannot start")?;

    app.run(|_, event| {
        if let RunEvent::Exit = event {
            // The window is gone: do not leave an auto-apply Chrome running behind.
            jobpilot_core::runtime::block_on(jobpilot_core::apply::close_all());
            tracing::info!("JobPilot closed");
        }
    });
    Ok(())
}

/// What the webview may show: the bundled app (`tauri://localhost`), a blank page, and the dev
/// server in `tauri dev`. Everything else is refused.
fn may_show(scheme: &str, host: Option<&str>) -> bool {
    match scheme {
        "about" => true,
        "tauri" | "http" | "https" => {
            matches!(host, Some("tauri.localhost" | "localhost" | "127.0.0.1"))
        }
        _ => false,
    }
}

/// Every command is `<context>_<name>`; the prefix picks the context that owns it.
fn route() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    let shared = shared::shell::handler();
    let discovery = discovery::commands::handler();
    let applications = applications::commands::handler();
    let profile = profile::commands::handler();
    let integrations = integrations::commands::handler();
    let trends = trends::commands::handler();
    #[cfg(feature = "dev-tools")]
    let dev = devtools::handler();
    move |invoke| match context_of(invoke.message.command()) {
        Some("discovery") => discovery(invoke),
        Some("applications") => applications(invoke),
        Some("profile") => profile(invoke),
        Some("integrations") => integrations(invoke),
        Some("trends") => trends(invoke),
        #[cfg(feature = "dev-tools")]
        Some("dev") => dev(invoke),
        _ => shared(invoke),
    }
}

/// The context a command belongs to: the part of its name before the first `_`.
fn context_of(command: &str) -> Option<&str> {
    command.split_once('_').map(|(context, _)| context)
}

#[cfg(test)]
mod tests {
    use super::{context_of, may_show};

    #[test]
    fn a_command_is_routed_by_its_prefix() {
        assert_eq!(context_of("discovery_queue"), Some("discovery"));
        assert_eq!(context_of("applications_set_status"), Some("applications"));
        assert_eq!(context_of("shared_status"), Some("shared"));
        assert_eq!(context_of("nonsense"), None);
    }

    #[test]
    fn the_webview_only_shows_the_app() {
        assert!(may_show("tauri", Some("localhost")));
        assert!(may_show("http", Some("tauri.localhost")));
        assert!(may_show("http", Some("localhost")), "the dev server");
        assert!(may_show("about", None));
        assert!(!may_show("https", Some("example.com")));
        assert!(!may_show("tauri", Some("example.com")));
        assert!(!may_show("file", None));
    }
}
