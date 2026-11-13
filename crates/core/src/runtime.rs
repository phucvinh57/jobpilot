//! The one global tokio runtime. The UI never owns an executor for I/O:
//! it calls [`spawn`] and awaits the returned receiver inside `cx.spawn`
//! (oneshot receivers are runtime-agnostic).

use std::future::Future;
use std::sync::OnceLock;

use tokio::runtime::{Handle, Runtime};
use tokio::sync::oneshot;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("jobpilot-rt")
            .build()
            .expect("failed to start the tokio runtime")
    })
}

/// Handle to the global runtime.
pub fn handle() -> Handle {
    runtime().handle().clone()
}

/// Run `fut` on the global runtime; the output arrives on the receiver.
/// If the receiver is dropped the result is discarded (the future still runs to completion).
pub fn spawn<F>(fut: F) -> oneshot::Receiver<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx
}

/// Block the current (non-runtime) thread on `fut`. For tests and CLI tools only; never call from the UI thread.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    runtime().block_on(fut)
}
