//! JobPilot desktop app (GPUI). Views only: all logic lives in `jobpilot-core`.

mod app;
mod detail;
mod settings;
mod sidebar;

use std::sync::Arc;

use gpui_kit::*;
use jobpilot_core::Core;

fn main() -> anyhow::Result<()> {
    let core = Arc::new(Core::open()?);
    // Sample data for UI work: `JOBPILOT_DEMO=1 cargo run -p jobpilot` (never on by default).
    if std::env::var_os("JOBPILOT_DEMO").is_some() {
        app::seed_demo(&core)?;
    }

    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(720.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some("JobPilot".into()), ..Default::default() }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| app::JobPilot::new(core, window, cx)))
            .expect("failed to open the window");
        cx.activate(true);
    });
    Ok(())
}
