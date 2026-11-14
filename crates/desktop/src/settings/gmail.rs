//! Settings > Gmail (P6): read-only access used to track replies.
//!
//! Views only: OAuth, keyring and HTTP live in `jobpilot_core::mail`; everything that can block
//! runs through `runtime::spawn`.

use std::future::Future;

use anyhow::anyhow;
use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::Local;
use jobpilot_core::mail::{self, GmailState};
use jobpilot_core::runtime;

use crate::ui::{field, section};
use crate::app::AppState;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    Idle,
    Connecting,
    Syncing,
    Disconnecting,
}

/// Input widgets; created on the first render because they need a `Window`.
struct Form {
    client_id: Entity<InputState>,
    client_secret: Entity<InputState>,
}

struct Notice {
    ok: bool,
    text: String,
}

pub struct GmailSection {
    state: Entity<AppState>,
    gmail: GmailState,
    form: Option<Form>,
    busy: Busy,
    notice: Option<Notice>,
    confirm_disconnect: bool,
}

impl GmailSection {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let gmail = state.read(cx).core.gmail_state();

        // Background polls and "Sync now" announce new mail here: reload the sidebar's
        // applications and this section's "last sync" line.
        let mut changes = mail::changes();
        let app_state = state.clone();
        cx.spawn(async move |this, cx| {
            while changes.changed().await.is_ok() {
                if this.update(cx, |this, cx| this.refresh(cx)).is_err() {
                    break;
                }
                app_state.update(cx, |state, cx| state.reload(cx));
            }
        })
        .detach();

        Self { state, gmail, form: None, busy: Busy::Idle, notice: None, confirm_disconnect: false }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.gmail = self.state.read(cx).core.gmail_state();
        cx.notify();
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            return;
        }
        let saved = self.gmail.client_id.clone();
        let client_id = cx.new(|cx| InputState::new(window, cx).placeholder("Client ID (ends in .apps.googleusercontent.com)"));
        let placeholder = if saved.is_empty() { "Client secret" } else { "Client secret (leave empty to keep the saved one)" };
        let client_secret = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(placeholder));
        client_id.update(cx, |i, cx| i.set_value(saved, window, cx));
        self.form = Some(Form { client_id, client_secret });
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.notice = Some(Notice { ok: false, text: text.into() });
    }

    /// Run `work` on the core runtime, then call `done` on the UI thread.
    fn run<T, W, D>(&mut self, busy: Busy, window: &mut Window, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: Future<Output = anyhow::Result<T>> + Send + 'static,
        D: FnOnce(&mut Self, anyhow::Result<T>, &mut Window, &mut Context<Self>) + 'static,
    {
        self.busy = busy;
        self.notice = None;
        cx.notify();
        let rx = runtime::spawn(work);
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the background task was cancelled")));
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = Busy::Idle;
                done(this, result, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.form.as_ref() else { return };
        let client_id = form.client_id.read(cx).value().trim().to_string();
        let secret = form.client_secret.read(cx).value().trim().to_string();
        if client_id.is_empty() {
            self.fail("Enter the OAuth client ID.");
            cx.notify();
            return;
        }
        let core = self.state.read(cx).core.clone();
        let poller_core = core.clone();
        self.run(
            Busy::Connecting,
            window,
            cx,
            async move {
                core.gmail_save_client(&client_id, Some(&secret))?;
                core.gmail_connect().await
            },
            move |this, result, window, cx| match result {
                Ok(account) => {
                    if let Some(form) = &this.form {
                        form.client_secret.update(cx, |i, cx| i.set_value("", window, cx));
                    }
                    this.refresh(cx);
                    this.notice = Some(Notice { ok: true, text: format!("Connected as {account}. Syncing\u{2026}") });
                    // First sync starts right away; later ones every few minutes.
                    mail::autostart(&poller_core);
                }
                Err(e) => this.fail(format!("Could not connect: {e:#}")),
            },
        );
    }

    fn cancel_connect(&mut self, cx: &mut Context<Self>) {
        self.state.read(cx).core.gmail_cancel_connect();
    }

    fn sync_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.state.read(cx).core.clone();
        self.run(
            Busy::Syncing,
            window,
            cx,
            async move { mail::sync_once(&core).await },
            |this, _result, _, cx| {
                // The outcome (or error) is saved by the sync and shown by `refresh`.
                this.refresh(cx);
            },
        );
    }

    /// Two-step: the first click arms the button, the second disconnects.
    fn disconnect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.confirm_disconnect {
            self.confirm_disconnect = true;
            cx.notify();
            return;
        }
        self.confirm_disconnect = false;
        let core = self.state.read(cx).core.clone();
        self.run(
            Busy::Disconnecting,
            window,
            cx,
            async move { core.gmail_disconnect().await },
            |this, result, _, cx| {
                this.refresh(cx);
                match result {
                    Ok(()) => this.notice = Some(Notice { ok: true, text: "Disconnected.".into() }),
                    Err(e) => this.fail(format!("Could not disconnect: {e:#}")),
                }
            },
        );
    }

    fn render_signed_out(&self, form: &Form, muted: Hsla, cx: &mut Context<Self>) -> Div {
        let connecting = self.busy == Busy::Connecting;
        div()
            .v_flex()
            .gap_3()
            .pt_2()
            .child(div().text_sm().text_color(muted).child(
                "Create an OAuth client of type \"Desktop app\" in the Google Cloud Console, enable the Gmail API, \
                 then paste its client ID and secret. The secret is kept in the OS keyring.",
            ))
            .child(field("Client ID", Input::new(&form.client_id), cx))
            .child(field("Client secret", Input::new(&form.client_secret).mask_toggle(), cx))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("gmail-connect")
                            .primary()
                            .label("Connect Gmail")
                            .loading(connecting)
                            .disabled(self.busy != Busy::Idle)
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                    )
                    .when(connecting, |d| {
                        d.child(div().text_sm().text_color(muted).child("waiting for browser\u{2026}")).child(
                            Button::new("gmail-cancel")
                                .ghost()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| this.cancel_connect(cx))),
                        )
                    }),
            )
    }

    fn render_signed_in(&self, account: &str, muted: Hsla, ok: Hsla, bad: Hsla, cx: &mut Context<Self>) -> Div {
        let idle = self.busy == Busy::Idle;
        let minutes = self.state.read(cx).core.gmail_poll_interval().as_secs() / 60;

        let last_sync = match self.gmail.last_sync {
            Some(at) => format!("Last sync: {}", at.with_timezone(&Local).format("%Y-%m-%d %H:%M")),
            None => "Not synced yet".to_string(),
        };
        let (result, result_ok) = match &self.gmail.last_report {
            Some(r) => match &r.error {
                Some(e) => (format!("Sync problem: {e}"), false),
                None => {
                    let mut text = format!("fetched {}, updated {}", r.fetched, r.updated);
                    if r.needs_review > 0 {
                        let verb = if r.needs_review == 1 { "needs" } else { "need" };
                        text.push_str(&format!(", {} {verb} review", r.needs_review));
                    }
                    (text, true)
                }
            },
            None => (String::new(), true),
        };

        div()
            .v_flex()
            .gap_3()
            .pt_2()
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_color(ok).child("\u{25CF}"))
                    .child(div().font_semibold().child(format!("Connected as {account}"))),
            )
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .text_sm()
                    .child(div().text_color(muted).child(last_sync))
                    .when(!result.is_empty(), |d| {
                        d.child(div().min_w_0().text_color(if result_ok { muted } else { bad }).child(result))
                    })
                    .child(
                        div()
                            .text_color(muted)
                            .child(format!("New mail is checked every {minutes} min while JobPilot is open.")),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("gmail-sync")
                            .primary()
                            .label("Sync now")
                            .loading(self.busy == Busy::Syncing)
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| this.sync_now(window, cx))),
                    )
                    .child(
                        Button::new("gmail-disconnect")
                            .danger()
                            .outline()
                            .label(if self.confirm_disconnect { "Click again to disconnect" } else { "Disconnect" })
                            .loading(self.busy == Busy::Disconnecting)
                            .disabled(!idle)
                            .on_click(cx.listener(|this, _, window, cx| this.disconnect(window, cx))),
                    ),
            )
    }
}

impl Render for GmailSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        let (muted, ok, bad) = {
            let t = cx.theme();
            (t.muted_foreground, t.success, t.danger)
        };

        let mut body = section(
            "Gmail",
            "Read-only access to spot confirmations, interviews and rejections for your applications.",
            cx,
        );
        match (self.gmail.account.clone(), self.form.as_ref()) {
            (Some(account), _) => body = body.child(self.render_signed_in(&account, muted, ok, bad, cx)),
            (None, Some(form)) => body = body.child(self.render_signed_out(form, muted, cx)),
            (None, None) => {}
        }
        body.when_some(self.notice.as_ref(), |d, n| {
            d.child(div().text_sm().min_w_0().text_color(if n.ok { ok } else { bad }).child(n.text.clone()))
        })
    }
}
