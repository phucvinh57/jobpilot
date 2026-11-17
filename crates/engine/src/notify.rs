//! Desktop notifications (best effort: a missing notification daemon is not an error).
//!
//! Shared by mail (replies) and discovery (a source needs the user). Text is shown as given:
//! callers pass ids, counts and labels, never page, JD or email bodies.

/// Show a notification without blocking the caller.
pub fn desktop_notify(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        if let Err(e) = notify_rust::Notification::new().appname("JobPilot").summary(&title).body(&body).show() {
            tracing::debug!("desktop notification failed: {e}");
        }
    });
}
