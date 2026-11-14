//! The browser download (Chrome for Testing) as a service: one at a time, with its progress pushed
//! to the web app as `integrations:browser-download` events while it runs.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jobpilot_core::browser::chrome::{self, DownloadProgress};
use jobpilot_core::{runtime, Core};
use tauri::AppHandle;

use super::dto::DownloadDto;
use crate::shared::events::emit;
use crate::shared::{run, CommandError, CommandResult};

/// The event that carries a [`DownloadDto`].
pub const EVENT: &str = "integrations:browser-download";

/// How often the percentage is pushed while the download runs.
const TICK: Duration = Duration::from_millis(400);

/// The download that is running, if any. Managed by Tauri (`mod.rs::setup`).
#[derive(Default)]
pub struct Downloads {
    current: Mutex<Option<Arc<DownloadProgress>>>,
}

impl Downloads {
    /// Claim the download slot; `None` when a download is running already.
    fn begin(&self) -> Option<Arc<DownloadProgress>> {
        let mut slot = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return None;
        }
        let progress = Arc::new(DownloadProgress::default());
        *slot = Some(progress.clone());
        Some(progress)
    }

    fn end(&self) {
        *self.current.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Where the download stands now (also for a screen that opened after it started).
    pub fn state(&self) -> DownloadDto {
        match &*self.current.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(progress) => DownloadDto {
                active: true,
                percent: progress.percent(),
            },
            None => DownloadDto::default(),
        }
    }
}

/// Download the managed browser. Resolves when it is installed (or failed); the percentage goes out
/// as events meanwhile, and a last one with `active: false` when it is over.
pub async fn download_browser(
    app: AppHandle,
    core: Arc<Core>,
    downloads: &Downloads,
) -> CommandResult<()> {
    let Some(progress) = downloads.begin() else {
        return Err(CommandError::new("A browser download is already running."));
    };
    tracing::info!("browser download started");
    emit(
        &app,
        EVENT,
        DownloadDto {
            active: true,
            percent: None,
        },
    );

    let finished = Arc::new(AtomicBool::new(false));
    let ticker = {
        let (app, progress, finished) = (app.clone(), progress.clone(), finished.clone());
        runtime::spawn(async move {
            while !finished.load(Ordering::Relaxed) {
                tokio::time::sleep(TICK).await;
                if !finished.load(Ordering::Relaxed) {
                    emit(
                        &app,
                        EVENT,
                        DownloadDto {
                            active: true,
                            percent: progress.percent(),
                        },
                    );
                }
            }
        })
    };

    let paths = core.paths.clone();
    let result = run(async move {
        chrome::download_managed(&paths, Some(progress))
            .await
            .map(|_| ())
    })
    .await;

    finished.store(true, Ordering::Relaxed);
    drop(ticker);
    downloads.end();
    emit(&app, EVENT, DownloadDto::default());
    match &result {
        Ok(()) => tracing::info!("browser download finished"),
        Err(e) => tracing::warn!("browser download failed: {}", e.message),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::Downloads;

    #[test]
    fn only_one_download_runs_at_a_time() {
        let downloads = Downloads::default();
        assert!(!downloads.state().active);
        let progress = downloads.begin();
        assert!(progress.is_some());
        assert!(downloads.begin().is_none(), "a second one is refused");
        assert!(downloads.state().active);
        assert_eq!(downloads.state().percent, None, "the size is not known yet");

        if let Some(progress) = progress {
            progress
                .total
                .store(200, std::sync::atomic::Ordering::Relaxed);
            progress
                .downloaded
                .store(50, std::sync::atomic::Ordering::Relaxed);
        }
        assert_eq!(downloads.state().percent, Some(25));

        downloads.end();
        assert!(!downloads.state().active);
        assert!(downloads.begin().is_some(), "the slot is free again");
    }
}
