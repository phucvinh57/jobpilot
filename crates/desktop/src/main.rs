//! JobPilot desktop app (GPUI). Views only: all logic lives in `jobpilot-core`.

mod app;
mod detail;
mod pipeline;
mod problem;
mod queue;
mod settings;
mod sidebar;
mod trends;
mod ui;

use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::*;
use jobpilot_core::prefs::Appearance;
use jobpilot_core::{logging, mail, Core, Paths, Secrets};

/// Window / desktop-entry id (`jobpilot.desktop`, StartupWMClass).
const APP_ID: &str = "jobpilot";

fn main() -> anyhow::Result<()> {
    // Logs go to stderr and `<data dir>/logs/`, with secrets redacted.
    let paths = Paths::from_env()?;
    let _log = logging::init(&paths);
    tracing::info!("JobPilot {} starting, data in {}", env!("CARGO_PKG_VERSION"), paths.data_dir.display());

    let core = Arc::new(Core::open_at(paths, Secrets::from_env())?);
    // Sample data for UI work: `JOBPILOT_DEMO=1 cargo run -p jobpilot` (never on by default).
    if std::env::var_os("JOBPILOT_DEMO").is_some() {
        app::seed_demo(&core)?;
    }

    // The saved light/dark choice, applied before the first frame (a bad read only means System).
    let appearance = core.appearance().unwrap_or_else(|e| {
        tracing::warn!("reading the appearance setting failed: {e:#}");
        Appearance::System
    });

    // Gmail tracking: poll for replies while the app runs (no-op unless Gmail is connected).
    mail::autostart(&core);

    // A window that fails to open must not panic: report it and exit with an error.
    let opened = std::rc::Rc::new(std::cell::Cell::new(true));
    let opened_in_run = opened.clone();
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        // Fonts before the first frame: the theme asks for Inter by name.
        ui::fonts::register(cx);
        settings::appearance::apply(appearance, None, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some("JobPilot".into()), ..Default::default() }),
            app_id: Some(APP_ID.into()),
            ..Default::default()
        };
        let window = gpui_kit::open_window(options, cx, |window, cx| {
            // The window knows the desktop's appearance (the app-level query can be wrong on
            // Linux): settle System on it, and follow the desktop when it switches.
            settings::appearance::apply(appearance, Some(window), cx);
            settings::appearance::watch_system(window);
            cx.new(|cx| app::JobPilot::new(core, window, cx))
        });
        if let Err(e) = window {
            tracing::error!("failed to open the window: {e:#}");
            opened_in_run.set(false);
            cx.quit();
            return;
        }
        cx.activate(true);
    });
    // The window is gone: do not leave an auto-apply Chrome running behind.
    jobpilot_core::runtime::block_on(jobpilot_core::apply::close_all());
    tracing::info!("JobPilot closed");
    if !opened.get() {
        return Err(anyhow::anyhow!("the window could not be opened (see the log)")).context("JobPilot cannot start");
    }
    Ok(())
}
