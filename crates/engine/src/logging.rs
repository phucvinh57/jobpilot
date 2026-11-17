//! Logging: stderr plus a daily rolling file `<data_dir>/logs/jobpilot.log.<date>`.
//!
//! Every line goes through [`crate::redact::redact`] before it is written, so API keys and OAuth
//! tokens never reach either sink, even when a library puts them in an error message.
//! Level: `RUST_LOG` if set, else `info` for JobPilot and `warn` for everything else.
//! `log` records from libraries are bridged into the same pipeline.

use std::io::{self, Write};
use std::path::Path;

use tracing_appender::non_blocking::{NonBlocking, WorkerGuard};
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

use crate::redact::redact;
use crate::Paths;

/// Log file name prefix (`jobpilot.log.2026-11-13`).
pub const LOG_FILE: &str = "jobpilot.log";
/// Daily files kept on disk.
const KEEP_FILES: usize = 14;
/// Default filter: the engine at info, everything else at warn (the caller adds its own crate).
const DEFAULT_FILTER: &str = "warn,jobpilot_engine=info";

/// Keep this alive for the life of the program: dropping it flushes the file writer.
pub struct LogGuard {
    _file: Option<WorkerGuard>,
}

/// Install the global subscriber. Safe to call more than once (later calls do nothing).
/// A log directory that cannot be created only disables the file sink. `app_target` is the
/// calling crate's log target (`jobpilot_lib`), logged at info like the engine; `RUST_LOG`
/// replaces the whole default filter.
pub fn init(paths: &Paths, app_target: &str) -> LogGuard {
    let filter = || {
        EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(format!("{DEFAULT_FILTER},{app_target}=info")))
    };

    let (file_writer, guard) = match file_writer(&paths.logs_dir) {
        Ok((writer, guard)) => (Some(writer), Some(guard)),
        Err(e) => {
            eprintln!("jobpilot: file logging is off ({}): {e}", paths.logs_dir.display());
            (None, None)
        }
    };
    let stderr_layer = fmt::layer().with_writer(Redacting(io::stderr)).with_target(true);
    let file_layer = file_writer.map(|w| fmt::layer().with_writer(Redacting(w)).with_ansi(false).with_target(true));

    // `try_init` also installs the `log` -> `tracing` bridge (feature `tracing-log`).
    let _ = tracing_subscriber::registry().with(filter()).with(stderr_layer).with(file_layer).try_init();
    install_panic_hook();
    LogGuard { _file: guard }
}

fn file_writer(dir: &Path) -> io::Result<(NonBlocking, WorkerGuard)> {
    std::fs::create_dir_all(dir)?;
    let appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(LOG_FILE)
        .max_log_files(KEEP_FILES)
        .build(dir)
        .map_err(|e| io::Error::other(e.to_string()))?;
    Ok(tracing_appender::non_blocking(appender))
}

/// A panic is logged (message and location) before the default hook prints it.
fn install_panic_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let default = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!("panic: {info}");
            default(info);
        }));
    });
}

/// Wraps a writer factory so everything written is redacted.
#[derive(Clone)]
struct Redacting<M>(M);

impl<'a, M> MakeWriter<'a> for Redacting<M>
where
    M: MakeWriter<'a>,
{
    type Writer = RedactingWriter<M::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter { inner: self.0.make_writer(), buf: Vec::new() }
    }
}

/// Collects one event, then writes it redacted. (The fmt layer writes an event and drops the writer.)
pub struct RedactingWriter<W: Write> {
    inner: W,
    buf: Vec<u8>,
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flush_buffer()?;
        self.inner.flush()
    }
}

impl<W: Write> RedactingWriter<W> {
    fn flush_buffer(&mut self) -> io::Result<()> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.buf).into_owned();
        self.buf.clear();
        self.inner.write_all(redact(&text).as_bytes())
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_buffer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl Write for Sink {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }

    #[test]
    fn events_are_redacted_before_they_reach_the_sink() {
        let sink = Sink::default();
        let subscriber = tracing_subscriber::registry()
            .with(fmt::layer().with_writer(Redacting(sink.clone())).with_ansi(false));
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!("provider test failed: HTTP 401: Authorization: Bearer abcdef123456789");
            tracing::warn!("token refresh: refresh_token=1//0gabcdefghijklmnopqrstuvwxyz failed");
            tracing::info!("applied to Acme");
        });
        let out = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        assert!(!out.contains("abcdef123456789") && !out.contains("1//0g"), "{out}");
        assert!(out.contains("[REDACTED]") && out.contains("applied to Acme"), "{out}");
    }

    #[test]
    fn init_writes_a_log_file_in_the_data_dir() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().join("jp"));
        let (writer, guard) = file_writer(&paths.logs_dir).unwrap();
        let subscriber = tracing_subscriber::registry().with(fmt::layer().with_writer(Redacting(writer)).with_ansi(false));
        tracing::subscriber::with_default(subscriber, || tracing::info!("hello log file, api_key=sk-abcdefghijklmnopqrstuvwxyz"));
        drop(guard);
        let file = std::fs::read_dir(&paths.logs_dir).unwrap().next().unwrap().unwrap().path();
        assert!(file.file_name().unwrap().to_string_lossy().starts_with(LOG_FILE));
        let text = std::fs::read_to_string(file).unwrap();
        assert!(text.contains("hello log file") && !text.contains("sk-abc"), "{text}");
    }
}
