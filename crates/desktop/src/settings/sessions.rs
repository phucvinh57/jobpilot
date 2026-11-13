//! Settings > Browser sessions (P4): app-owned browser profiles used to apply.
//!
//! All browser work goes through `core.sessions()` (a `SessionManager`) on the core runtime; this view only
//! starts operations and shows their result.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::browser::chrome::{self, Chrome, DownloadProgress};
use jobpilot_core::browser::SessionStatus;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::runtime;
use jobpilot_core::store::Session;

use super::section;
use crate::app::AppState;

/// Quick starts for the "New session" form: (name, login URL).
const SUGGESTIONS: [(&str, &str); 3] = [
    ("LinkedIn", "https://www.linkedin.com/login"),
    ("Indeed", "https://secure.indeed.com/account/login"),
    ("Google", "https://accounts.google.com/"),
];

/// `busy` key for operations that are not about one existing session (creating a new one).
const NEW_KEY: i64 = 0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Destructive {
    SignOut,
    Delete,
}

pub struct SessionsSection {
    state: Entity<AppState>,
    sessions: Vec<Session>,
    chrome: Option<Chrome>,
    /// Reload `sessions`/`chrome` from the store on the next render.
    dirty: bool,
    initialised: bool,
    form_open: bool,
    name_input: Option<Entity<InputState>>,
    url_input: Option<Entity<InputState>>,
    /// Running operations by session id (`NEW_KEY` for creation) with the label to show.
    busy: HashMap<i64, &'static str>,
    /// First click on Sign out / Delete asks for confirmation; the second one runs it.
    confirm: Option<(i64, Destructive)>,
    error: Option<String>,
    download: Option<Arc<DownloadProgress>>,
}

impl SessionsSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self {
            state,
            sessions: Vec::new(),
            chrome: None,
            dirty: true,
            initialised: false,
            form_open: false,
            name_input: None,
            url_input: None,
            busy: HashMap::new(),
            confirm: None,
            error: None,
            download: None,
        }
    }

    /// One-time setup that needs a window/context (inputs, re-reading when the app state changes).
    fn init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.initialised = true;
        self.name_input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. LinkedIn")));
        self.url_input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Login URL, https://...")));
        // Switching screens notifies the app state: pick up statuses changed elsewhere (e.g. by auto-apply).
        cx.observe(&self.state, |this, _, cx| {
            this.dirty = true;
            cx.notify();
        })
        .detach();
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.state.read(cx).core.clone();
        match core.sessions().list() {
            Ok(list) => self.sessions = list,
            Err(e) => self.error = Some(format!("{e:#}")),
        }
        self.chrome = core.chrome();
        self.dirty = false;
    }

    /// Run `fut` on the core runtime; `on_ok` runs on the UI thread with its result.
    fn run<T: Send + 'static>(
        &mut self,
        key: i64,
        label: &'static str,
        fut: impl Future<Output = anyhow::Result<T>> + Send + 'static,
        on_ok: impl FnOnce(&mut Self, T) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.busy.insert(key, label);
        self.confirm = None;
        self.error = None;
        cx.notify();
        let rx = runtime::spawn(fut);
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.busy.remove(&key);
                match result {
                    Ok(Ok(value)) => on_ok(this, value),
                    Ok(Err(e)) => this.error = Some(format!("{e:#}")),
                    Err(_) => this.error = Some("the task was cancelled".into()),
                }
                this.dirty = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn open_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form_open = true;
        self.fill_form(SUGGESTIONS[0], window, cx);
    }

    fn fill_form(&mut self, (name, url): (&str, &str), window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(n), Some(u)) = (&self.name_input, &self.url_input) {
            n.update(cx, |i, cx| i.set_value(name.to_string(), window, cx));
            u.update(cx, |i, cx| i.set_value(url.to_string(), window, cx));
        }
        cx.notify();
    }

    fn create_and_login(&mut self, cx: &mut Context<Self>) {
        let (Some(n), Some(u)) = (&self.name_input, &self.url_input) else { return };
        let (name, url) = (n.read(cx).value().to_string(), u.read(cx).value().to_string());
        let mgr = self.state.read(cx).core.sessions();
        self.run(
            NEW_KEY,
            "Opening browser...",
            async move {
                let session = mgr.create(&name, &url).await?;
                mgr.open_login(session.id).await?;
                Ok(session.id)
            },
            |this, _| this.form_open = false,
            cx,
        );
    }

    fn open_login(&mut self, id: i64, cx: &mut Context<Self>) {
        let mgr = self.state.read(cx).core.sessions();
        self.run(id, "Opening browser...", async move { mgr.open_login(id).await.map(|_| ()) }, |_, _| {}, cx);
    }

    fn finish_login(&mut self, id: i64, cx: &mut Context<Self>) {
        let mgr = self.state.read(cx).core.sessions();
        self.run(id, "Checking...", async move { mgr.finish_login(id).await }, |_, _| {}, cx);
    }

    fn check(&mut self, id: i64, cx: &mut Context<Self>) {
        let mgr = self.state.read(cx).core.sessions();
        self.run(id, "Checking...", async move { mgr.check(id).await }, |_, _| {}, cx);
    }

    /// First click arms the confirmation, the second runs the action.
    fn destructive(&mut self, id: i64, action: Destructive, cx: &mut Context<Self>) {
        if self.confirm != Some((id, action)) {
            self.confirm = Some((id, action));
            cx.notify();
            return;
        }
        let mgr = self.state.read(cx).core.sessions();
        match action {
            Destructive::SignOut => {
                self.run(id, "Signing out...", async move { mgr.sign_out(id).await }, |_, _| {}, cx)
            }
            Destructive::Delete => self.run(id, "Deleting...", async move { mgr.delete(id).await }, |_, _| {}, cx),
        }
    }

    fn download_browser(&mut self, cx: &mut Context<Self>) {
        if self.download.is_some() {
            return;
        }
        self.error = None;
        let paths = self.state.read(cx).core.paths.clone();
        let progress = Arc::new(DownloadProgress::default());
        self.download = Some(progress.clone());
        let rx = runtime::spawn(async move { chrome::download_managed(&paths, Some(progress)).await });
        // Repaint while the download runs so the percentage moves.
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            match this.update(cx, |this, cx| {
                cx.notify();
                this.download.is_some()
            }) {
                Ok(true) => {}
                _ => break,
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.download = None;
                match result {
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => this.error = Some(format!("{e:#}")),
                    Err(_) => this.error = Some("the download was cancelled".into()),
                }
                this.dirty = true;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}

fn ago(t: DateTime<Utc>) -> String {
    let d = Utc::now() - t;
    if d.num_minutes() < 1 {
        "just now".into()
    } else if d.num_hours() < 1 {
        format!("{}m ago", d.num_minutes())
    } else if d.num_days() < 1 {
        format!("{}h ago", d.num_hours())
    } else {
        format!("{}d ago", d.num_days())
    }
}

/// Host shown under the session name.
fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).to_string()
}

impl Render for SessionsSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.initialised {
            self.init(window, cx);
        }
        if self.dirty {
            self.reload(cx);
        }
        let mgr = self.state.read(cx).core.sessions();
        let theme = cx.theme();
        let muted = theme.muted_foreground;

        // Browser line.
        let browser_text = if let Some(progress) = &self.download {
            match progress.percent() {
                Some(p) => format!("Downloading browser... {p}%"),
                None => "Downloading browser...".to_string(),
            }
        } else {
            match &self.chrome {
                Some(c) => format!("Browser: {} ({})", c.path.display(), c.source.label()),
                None => "Browser: not found".to_string(),
            }
        };
        let browser_line = div()
            .h_flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().truncate().text_sm().text_color(muted).child(browser_text))
            .child(
                Button::new("download-browser")
                    .small()
                    .outline()
                    .label(if self.chrome.is_some() { "Update browser" } else { "Download browser" })
                    .loading(self.download.is_some())
                    .disabled(self.download.is_some())
                    .on_click(cx.listener(|this, _, _, cx| this.download_browser(cx))),
            );

        // Session rows.
        let rows: Vec<_> = self
            .sessions
            .iter()
            .map(|s| {
                let id = s.id;
                let busy = self.busy.get(&id).copied();
                let login = mgr.login_in_progress(id);
                let status = s.session_status();
                let pill = match status {
                    SessionStatus::Ok => Tag::success(),
                    SessionStatus::Expired => Tag::danger(),
                    SessionStatus::New => Tag::secondary(),
                }
                .child(status.as_str());
                let verified = s.last_verified_at.map(|t| format!("verified {}", ago(t))).unwrap_or_else(|| "never verified".into());
                let confirm_sign_out = self.confirm == Some((id, Destructive::SignOut));
                let confirm_delete = self.confirm == Some((id, Destructive::Delete));

                let actions = if let Some(label) = busy {
                    div().h_flex().gap_2().child(div().text_sm().text_color(muted).child(label))
                } else if login {
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_sm().text_color(muted).child("Log in in the browser window, then press Done"))
                        .child(
                            Button::new(("done", id as usize))
                                .small()
                                .primary()
                                .label("Done")
                                .on_click(cx.listener(move |this, _, _, cx| this.finish_login(id, cx))),
                        )
                } else {
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new(("check", id as usize))
                                .small()
                                .outline()
                                .label("Check")
                                .on_click(cx.listener(move |this, _, _, cx| this.check(id, cx))),
                        )
                        .child(
                            Button::new(("open", id as usize))
                                .small()
                                .outline()
                                .label("Open")
                                .on_click(cx.listener(move |this, _, _, cx| this.open_login(id, cx))),
                        )
                        .child(
                            Button::new(("sign-out", id as usize))
                                .small()
                                .outline()
                                .label(if confirm_sign_out { "Sure?" } else { "Sign out" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.destructive(id, Destructive::SignOut, cx)
                                })),
                        )
                        .child(
                            Button::new(("delete", id as usize))
                                .small()
                                .danger()
                                .label(if confirm_delete { "Sure?" } else { "Delete" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.destructive(id, Destructive::Delete, cx)
                                })),
                        )
                };

                div()
                    .h_flex()
                    .items_center()
                    .gap_3()
                    .py_1()
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().font_semibold().truncate().child(s.name.clone()))
                            .child(div().text_xs().text_color(muted).truncate().child(format!("{} - {verified}", host(&s.start_url)))),
                    )
                    .child(pill)
                    .child(actions)
            })
            .collect();

        // New-session form / button.
        let creating = self.busy.get(&NEW_KEY).copied();
        let form = if self.form_open {
            let (name_input, url_input) = (self.name_input.clone(), self.url_input.clone());
            div()
                .v_flex()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(theme.border)
                .rounded_md()
                .children(name_input.map(|i| Input::new(&i)))
                .children(url_input.map(|i| Input::new(&i)))
                .child(
                    div().h_flex().gap_2().items_center().child(div().text_xs().text_color(muted).child("Suggestions:")).children(
                        SUGGESTIONS.iter().enumerate().map(|(i, suggestion)| {
                            let suggestion = *suggestion;
                            Button::new(("suggestion", i))
                                .small()
                                .ghost()
                                .label(suggestion.0)
                                .on_click(cx.listener(move |this, _, window, cx| this.fill_form(suggestion, window, cx)))
                        }),
                    ),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("create-session")
                                .small()
                                .primary()
                                .label("Open browser to log in")
                                .loading(creating.is_some())
                                .disabled(creating.is_some())
                                .on_click(cx.listener(|this, _, _, cx| this.create_and_login(cx))),
                        )
                        .child(
                            Button::new("cancel-session")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(creating.is_some())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.form_open = false;
                                    cx.notify();
                                })),
                        )
                        .children(creating.map(|label| div().text_sm().text_color(muted).child(label))),
                )
                .into_any_element()
        } else {
            div()
                .h_flex()
                .child(
                    Button::new("new-session")
                        .small()
                        .primary()
                        .label("New session")
                        .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx))),
                )
                .into_any_element()
        };

        section("Browser sessions", "Logins kept in a managed browser profile, separate from your own browser.", cx)
            .child(browser_line)
            .child(div().v_flex().gap_1().children(rows))
            .when(self.sessions.is_empty() && !self.form_open, |d| {
                d.child(div().text_sm().text_color(muted).child("No sessions yet"))
            })
            .child(form)
            .children(self.error.clone().map(|e| div().text_sm().text_color(cx.theme().danger).child(e)))
    }
}
