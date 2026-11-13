//! Desktop notifications (best effort: a missing notification daemon is not an error).

/// Show a notification without blocking the caller.
pub(crate) fn desktop_notify(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("JobPilot").summary(&title).body(&body).show() {
            tracing::debug!("desktop notification failed: {e}");
        }
    });
}
