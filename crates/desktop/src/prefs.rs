//! Preferences screen (P9): the job-search chat on the left, the saved preferences on the right.
//!
//! Views only. The chat is the `prefs_chat` graph in `jobpilot_core::prefs` (`PrefsChat`); every
//! call goes through `runtime::spawn` and its result comes back with a fresh `ChatView`, so the UI
//! never blocks and never touches the database itself. Every merged patch is already saved, so
//! there is no Save button; hand edits on the right are saved the same way, and "Undo last change"
//! takes back the latest one (of either kind).
//!
//! Exactly one primary button: "Send", or "Confirm preferences" while the assistant waits for the
//! confirmation (`primary_for`).

use std::path::{Path, PathBuf};

use gpui_kit::component::button::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::health::{classify, Problem};
use jobpilot_core::llm::Role as Speaker;
use jobpilot_core::prefs::chat::{MAX_LIST_ITEMS, MAX_NOTES_CHARS};
use jobpilot_core::prefs::{cv_upload, ChatStatus, ChatView, Preferences, PrefsChat, Remote};
use jobpilot_core::runtime;

use crate::app::{AppState, Screen};

/// Width of the preference summary.
const SUMMARY_WIDTH: f32 = 380.0;
/// Longest list entry typed by hand, in characters.
const MAX_ITEM_CHARS: usize = 80;

// ---------------------------------------------------------------- pure logic

/// The one primary button of the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Primary {
    /// Send the typed message.
    Send,
    /// The assistant shows a summary and waits: confirm it.
    Confirm,
}

/// Which button is primary for the chat's status. The other one stays a plain button.
pub fn primary_for(status: Option<&ChatStatus>) -> Primary {
    match status {
        Some(ChatStatus::AwaitConfirm) => Primary::Confirm,
        _ => Primary::Send,
    }
}

/// A list field of the summary, with the accessors the hand edits use.
struct ListField {
    key: &'static str,
    label: &'static str,
    get: fn(&Preferences) -> &Vec<String>,
    get_mut: fn(&mut Preferences) -> &mut Vec<String>,
}

const LIST_FIELDS: &[ListField] = &[
    ListField { key: "roles", label: "Roles", get: |p| &p.roles, get_mut: |p| &mut p.roles },
    ListField { key: "seniority", label: "Seniority", get: |p| &p.seniority, get_mut: |p| &mut p.seniority },
    ListField { key: "must-skills", label: "Must-have skills", get: |p| &p.must_skills, get_mut: |p| &mut p.must_skills },
    ListField { key: "nice-skills", label: "Nice-to-have skills", get: |p| &p.nice_skills, get_mut: |p| &mut p.nice_skills },
    ListField { key: "avoid-skills", label: "Skills to avoid", get: |p| &p.avoid_skills, get_mut: |p| &mut p.avoid_skills },
    ListField { key: "domains", label: "Domains", get: |p| &p.domains, get_mut: |p| &mut p.domains },
    ListField { key: "avoid-domains", label: "Domains to avoid", get: |p| &p.avoid_domains, get_mut: |p| &mut p.avoid_domains },
    ListField { key: "locations", label: "Locations", get: |p| &p.locations, get_mut: |p| &mut p.locations },
    ListField { key: "company-size", label: "Company size", get: |p| &p.company_size, get_mut: |p| &mut p.company_size },
    ListField { key: "company-stage", label: "Company stage", get: |p| &p.company_stage, get_mut: |p| &mut p.company_stage },
    ListField { key: "languages", label: "Languages", get: |p| &p.languages, get_mut: |p| &mut p.languages },
    ListField {
        key: "exclude-companies",
        label: "Excluded companies",
        get: |p| &p.exclude_companies,
        get_mut: |p| &mut p.exclude_companies,
    },
];

/// `list` plus `text`; `None` when there is nothing to add (blank, already there ignoring case, full).
pub fn with_item(list: &[String], text: &str) -> Option<Vec<String>> {
    let text: String = text.trim().chars().take(MAX_ITEM_CHARS).collect();
    if text.is_empty() || list.len() >= MAX_LIST_ITEMS || list.iter().any(|i| i.eq_ignore_ascii_case(&text)) {
        return None;
    }
    let mut out = list.to_vec();
    out.push(text);
    Some(out)
}

/// `list` without the entry at `index` (unchanged when there is none).
pub fn without_item(list: &[String], index: usize) -> Vec<String> {
    let mut out = list.to_vec();
    if index < out.len() {
        out.remove(index);
    }
    out
}

/// The minimum salary box: `90000`, `90,000 usd` or `90000 EUR`. An empty box clears the salary.
/// Returns the amount and the currency to set (`None` keeps the current currency).
pub fn parse_salary(text: &str) -> Result<(Option<u64>, Option<String>), String> {
    let text = text.trim();
    if text.is_empty() {
        return Ok((None, None));
    }
    let mut parts = text.split_whitespace();
    let number: String = parts.next().unwrap_or_default().chars().filter(|c| *c != ',' && *c != '_').collect();
    let amount: u64 = number.parse().map_err(|_| "Enter a whole number, e.g. 90000 USD.".to_string())?;
    if amount > 1_000_000_000 {
        return Err("That salary is too large.".into());
    }
    let currency = match parts.next() {
        None => None,
        Some(c) if c.len() == 3 && c.chars().all(|ch| ch.is_ascii_alphabetic()) => Some(c.to_ascii_uppercase()),
        Some(_) => return Err("The currency is a 3-letter code, e.g. USD.".into()),
    };
    if parts.next().is_some() {
        return Err("Enter an amount and a currency, e.g. 90000 USD.".into());
    }
    Ok(((amount > 0).then_some(amount), currency))
}

/// What to show for the failure of the chat (a local error, or the failed thread).
struct Failure {
    message: String,
    problem: Problem,
    /// A failed turn can be run again.
    retry: bool,
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into())
}

fn shorten(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((i, _)) => format!("{}...", &text[..i]),
        None => text.to_string(),
    }
}

// ---------------------------------------------------------------- view

/// What the view is waiting for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Work {
    Idle,
    /// A chat call (open, send, attach, confirm, retry).
    Turn,
    /// A hand edit or an undo.
    Edit,
}

/// What a chat call does.
enum Action {
    Open,
    Send(String),
    Attach(PathBuf),
    Accept,
    Retry,
}

/// Input widgets; created on the first render because they need a `Window`.
struct Form {
    message: Entity<InputState>,
    salary: Entity<InputState>,
    /// One "add" box per entry of [`LIST_FIELDS`].
    lists: Vec<Entity<InputState>>,
}

/// The preference chat and the saved preferences.
pub struct PreferencesView {
    state: Entity<AppState>,
    form: Option<Form>,
    /// Set while this screen is shown; cleared when another screen is, so the next visit reloads.
    shown: bool,
    view: Option<ChatView>,
    work: Work,
    /// The message being answered, shown before it is stored.
    pending: Option<String>,
    error: Option<String>,
    /// A neutral line under the messages (a file that cannot be used, an undo done).
    notice: Option<String>,
    scroll: ScrollHandle,
}

impl PreferencesView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, state, cx| {
            if state.read(cx).screen != Screen::Preferences {
                this.shown = false;
            }
        })
        .detach();
        Self {
            state,
            form: None,
            shown: false,
            view: None,
            work: Work::Idle,
            pending: None,
            error: None,
            notice: None,
            scroll: ScrollHandle::new(),
        }
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            return;
        }
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Type a message (Enter)"));
        cx.subscribe_in(&message, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.submit(window, cx);
            }
        })
        .detach();
        let salary = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. 90000 USD per year (Enter)"));
        cx.subscribe_in(&salary, window, |this, input, event: &InputEvent, _, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let text = input.read(cx).value().to_string();
                this.set_salary(&text, cx);
            }
        })
        .detach();
        let lists = (0..LIST_FIELDS.len())
            .map(|ix| {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder("Add (Enter)"));
                cx.subscribe_in(&input, window, move |this, input, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        let text = input.read(cx).value().to_string();
                        if this.edit_list(ix, |list| with_item(list, &text), cx) {
                            input.update(cx, |input, cx| input.set_value("", window, cx));
                        }
                    }
                })
                .detach();
                input
            })
            .collect();
        self.form = Some(Form { message, salary, lists });
    }

    fn status(&self) -> Option<&ChatStatus> {
        self.view.as_ref().map(|v| &v.status)
    }

    fn busy(&self) -> bool {
        self.work != Work::Idle
    }

    /// The failure to show: an error of the last call, else a failed thread.
    fn failure(&self) -> Option<Failure> {
        if let Some(message) = &self.error {
            return Some(Failure { problem: classify(message), message: message.clone(), retry: false });
        }
        match self.status() {
            Some(ChatStatus::Failed(f)) => Some(Failure { message: f.message.clone(), problem: f.problem, retry: true }),
            _ => None,
        }
    }

    // ------------------------------------------------------------ chat calls

    /// Run a chat call on the core runtime; the new view arrives with its result.
    fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        self.work = Work::Turn;
        self.error = None;
        self.notice = None;
        let typed = match &action {
            Action::Send(text) => Some(text.clone()),
            _ => None,
        };
        self.pending = match &action {
            Action::Send(text) => Some(text.clone()),
            Action::Attach(path) => Some(format!("I attached my CV ({}).", file_name(path))),
            _ => None,
        };
        cx.notify();
        let core = self.state.read(cx).core.clone();
        let rx = runtime::spawn(async move {
            let chat = PrefsChat::new(core.clone());
            let result = match action {
                Action::Open => chat.open().await,
                Action::Send(text) => chat.send(&text).await,
                Action::Attach(path) => chat.attach_cv(&path).await,
                Action::Accept => chat.accept().await,
                Action::Retry => chat.retry().await,
            };
            (result, core.prefs_chat_view())
        });
        cx.spawn_in(window, async move |this, cx| {
            let done = rx.await;
            this.update_in(cx, |this, window, cx| {
                this.work = Work::Idle;
                this.pending = None;
                match done {
                    Ok((result, view)) => {
                        if let Err(e) = result {
                            this.error = Some(format!("{e:#}"));
                            // The message was not taken: give it back.
                            if let (Some(text), Some(form)) = (typed, &this.form) {
                                form.message.update(cx, |input, cx| input.set_value(text, window, cx));
                            }
                        }
                        match view {
                            Ok(view) => this.apply(view, cx),
                            Err(e) => this.error = this.error.take().or(Some(format!("{e:#}"))),
                        }
                    }
                    Err(_) => this.error = Some("the task was cancelled".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show a fresh view; the setup banner follows whether preferences were saved.
    fn apply(&mut self, view: ChatView, cx: &mut Context<Self>) {
        let saved = view.saved;
        self.view = Some(view);
        self.scroll.scroll_to_bottom();
        if self.state.read(cx).prefs_saved != saved {
            self.state.update(cx, |s, cx| s.reload(cx));
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.form.as_ref().map(|f| f.message.clone()) else { return };
        let text = message.read(cx).value().trim().to_string();
        if text.is_empty() || self.busy() {
            return;
        }
        message.update(cx, |input, cx| input.set_value("", window, cx));
        self.run(Action::Send(text), window, cx);
    }

    fn pick_cv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy() {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose your CV (.pdf, .md or .txt)".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |this, cx| {
                        this.notice = Some(format!("The file dialog did not open: {e:#}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update_in(cx, |this, window, cx| this.attach(&paths, window, cx)).ok();
        })
        .detach();
    }

    fn attach(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        match paths.iter().find(|p| cv_upload::is_supported(p)) {
            Some(path) => self.run(Action::Attach(path.clone()), window, cx),
            None => {
                self.notice = Some("Use a .pdf, .md or .txt file.".into());
                cx.notify();
            }
        }
    }

    // ------------------------------------------------------------ hand edits

    /// Save `prefs` (or undo) off the UI thread and show the result.
    fn save_with<F>(&mut self, work: F, cx: &mut Context<Self>)
    where
        F: FnOnce(&jobpilot_core::Core) -> anyhow::Result<()> + Send + 'static,
    {
        if self.busy() {
            return;
        }
        self.work = Work::Edit;
        self.error = None;
        self.notice = None;
        cx.notify();
        let core = self.state.read(cx).core.clone();
        let rx = runtime::spawn(async move {
            work(&core)?;
            core.prefs_chat_view()
        });
        cx.spawn(async move |this, cx| {
            let done = rx.await;
            this.update(cx, |this, cx| {
                this.work = Work::Idle;
                match done {
                    Ok(Ok(view)) => this.apply(view, cx),
                    Ok(Err(e)) => this.error = Some(format!("{e:#}")),
                    Err(_) => this.error = Some("the task was cancelled".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn edit(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) -> bool {
        let Some(view) = &self.view else { return false };
        if self.busy() {
            return false;
        }
        let mut prefs = view.preferences.clone();
        change(&mut prefs);
        self.save_with(
            move |core| {
                core.update_preferences(prefs)?;
                Ok(())
            },
            cx,
        );
        true
    }

    /// Change list field `ix` with `f` (`None` = nothing to change). `true` when a save started.
    fn edit_list(&mut self, ix: usize, f: impl FnOnce(&[String]) -> Option<Vec<String>>, cx: &mut Context<Self>) -> bool {
        let Some(field) = LIST_FIELDS.get(ix) else { return false };
        let Some(new) = self.view.as_ref().and_then(|v| f((field.get)(&v.preferences))) else { return false };
        self.edit(|prefs| *(field.get_mut)(prefs) = new, cx)
    }

    fn set_salary(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_salary(text) {
            Ok((amount, currency)) => {
                self.edit(
                    |prefs| {
                        prefs.salary_min = amount;
                        if amount.is_none() {
                            prefs.currency = None;
                        } else if currency.is_some() {
                            prefs.currency = currency;
                        }
                    },
                    cx,
                );
            }
            Err(message) => {
                self.notice = Some(message);
                cx.notify();
            }
        }
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.save_with(
            |core| {
                core.undo_preferences()?;
                Ok(())
            },
            cx,
        );
    }

    // ------------------------------------------------------------ rendering

    fn bubble(&self, id: impl Into<ElementId>, speaker: Speaker, text: &str, muted: bool, cx: &App) -> Div {
        let theme = cx.theme();
        let user = speaker == Speaker::User;
        let who = if user { "You" } else { "Assistant" };
        div()
            .w_full()
            .h_flex()
            .when(user, |d| d.justify_end())
            .child(
                div()
                    .id(id)
                    .role(Role::ListItem)
                    .aria_label(format!("{who}: {}", shorten(text, 80)))
                    .max_w(relative(0.85))
                    .min_w_0()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .text_sm()
                    .bg(if user { theme.primary.opacity(0.14) } else { theme.secondary })
                    .when(muted, |d| d.text_color(theme.muted_foreground))
                    .child(text.to_string()),
            )
    }

    fn render_messages(&self, view: &ChatView, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let turn = self.work == Work::Turn;
        let mut rows: Vec<Div> = view
            .messages
            .iter()
            .enumerate()
            .map(|(i, m)| self.bubble(("prefs-message", i), m.role, &m.content, false, cx))
            .collect();
        let unsent = view.unsent.as_ref().filter(|_| matches!(view.status, ChatStatus::Failed(_)) && self.pending.is_none());
        let shown_pending = self.pending.as_ref().or(unsent);
        if let Some(text) = shown_pending {
            rows.push(self.bubble("prefs-pending", Speaker::User, text, true, cx));
            if unsent.is_some() {
                rows.push(
                    div().w_full().h_flex().justify_end().child(div().text_xs().text_color(theme.danger).child("Not sent")),
                );
            }
        }
        div()
            .id("prefs-messages")
            .role(Role::Log)
            .aria_label("Conversation")
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .p_2()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .children(rows)
            .when(turn, |d| {
                d.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .text_sm()
                        .text_color(muted)
                        .child(Spinner::new().xsmall().color(theme.primary))
                        .child("Thinking..."),
                )
            })
            .when(view.messages.is_empty() && !turn, |d| {
                d.child(div().text_sm().text_color(muted).child("The assistant will ask a few questions about the job you want."))
            })
    }

    fn render_chips(&self, view: &ChatView, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let ready = !self.busy() && matches!(view.status, ChatStatus::Idle | ChatStatus::AwaitUser | ChatStatus::AwaitConfirm);
        if !ready || view.questions.iter().all(|q| q.options.is_empty()) {
            return None;
        }
        let muted = cx.theme().muted_foreground;
        let groups: Vec<Div> = view
            .questions
            .iter()
            .enumerate()
            .filter(|(_, q)| !q.options.is_empty())
            .map(|(qi, q)| {
                let chips: Vec<Button> = q
                    .options
                    .iter()
                    .enumerate()
                    .map(|(oi, option)| {
                        let reply = option.clone();
                        Button::new(SharedString::from(format!("prefs-chip-{qi}-{oi}")))
                            .outline()
                            .compact()
                            .label(option.clone())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.run(Action::Send(reply.clone()), window, cx)
                            }))
                    })
                    .collect();
                div()
                    .v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(muted).child(q.text.clone()))
                    .child(div().h_flex().flex_wrap().gap_1().children(chips))
            })
            .collect();
        Some(div().id("prefs-quick-replies").role(Role::Group).aria_label("Quick replies").v_flex().gap_2().children(groups))
    }

    fn render_failure(&self, cx: &mut Context<Self>) -> Option<Stateful<Div>> {
        let failure = self.failure()?;
        let danger = cx.theme().danger;
        let help = crate::problem::help("prefs-error", failure.problem, &self.state, None, cx);
        Some(
            div()
                .id("prefs-error")
                .role(Role::Alert)
                .aria_label("Error")
                .v_flex()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(danger)
                .child(div().text_sm().text_color(danger).child(shorten(&failure.message, 300)))
                .children(help)
                .when(failure.retry, |d| {
                    d.child(
                        div().h_flex().child(
                            Button::new("prefs-retry")
                                .outline()
                                .compact()
                                .label("Retry")
                                .disabled(self.busy())
                                .on_click(cx.listener(|this, _, window, cx| this.run(Action::Retry, window, cx))),
                        ),
                    )
                }),
        )
    }

    fn render_chat(&mut self, view: &ChatView, form_message: &Entity<InputState>, cx: &mut Context<Self>) -> Div {
        let (border, muted, accent, primary) = {
            let t = cx.theme();
            (t.border, t.muted_foreground, t.accent, t.primary)
        };
        let busy = self.busy();
        let which = primary_for(Some(&view.status));
        let send = Button::new("prefs-send")
            .label("Send")
            .loading(self.work == Work::Turn && which == Primary::Send)
            .disabled(busy)
            .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)));
        let send = if which == Primary::Send { send.primary() } else { send.outline() };

        let confirm = (which == Primary::Confirm).then(|| {
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(primary)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_sm()
                        .child("Is the summary right? Confirm to finish, or type a correction."),
                )
                .child(
                    Button::new("prefs-confirm")
                        .primary()
                        .label("Confirm preferences")
                        .loading(self.work == Work::Turn)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| this.run(Action::Accept, window, cx))),
                )
        });
        let accepted = matches!(view.status, ChatStatus::Accepted).then(|| {
            div().text_xs().text_color(muted).child("Preferences confirmed. Send a message any time to change them.")
        });
        let cv_line = view.cv.as_ref().map(|cv| {
            let cut = if cv.truncated { ", cut to the limit" } else { "" };
            format!("CV attached: {} ({} characters{cut}). It is only used as context for this chat.", cv.name, cv.chars)
        });

        div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_2()
            .p_2()
            .border_1()
            .border_color(border)
            .rounded_md()
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(accent))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| this.attach(paths.paths(), window, cx)))
            .child(self.render_messages(view, cx))
            .children(self.render_failure(cx))
            .children(confirm)
            .children(self.render_chips(view, cx))
            .children(accepted)
            .when_some(self.notice.clone(), |d, notice| d.child(div().text_xs().text_color(cx.theme().warning).child(notice)))
            .when_some(cv_line, |d, line| d.child(div().text_xs().text_color(muted).child(line)))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("prefs-attach")
                            .outline()
                            .icon(IconName::FileText)
                            .label("Attach CV")
                            .tooltip("A .pdf, .md or .txt file; you can also drop it on this pane")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, window, cx| this.pick_cv(window, cx))),
                    )
                    .child(div().flex_1().min_w_0().child(
                        Input::new(form_message).accessibility_id("prefs-input").aria_label("Message").disabled(busy),
                    ))
                    .child(send),
            )
    }

    fn section_title(&self, text: &'static str, cx: &App) -> Div {
        div().text_xs().font_semibold().text_color(cx.theme().muted_foreground).child(text)
    }

    fn render_list_field(&mut self, ix: usize, prefs: &Preferences, input: &Entity<InputState>, cx: &mut Context<Self>) -> Stateful<Div> {
        let field = &LIST_FIELDS[ix];
        let (muted, border) = {
            let t = cx.theme();
            (t.muted_foreground, t.border)
        };
        let busy = self.busy();
        let items = (field.get)(prefs);
        let chips: Vec<Button> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                Button::new(SharedString::from(format!("prefs-{}-{i}", field.key)))
                    .outline()
                    .compact()
                    .icon(IconName::Close)
                    .label(item.clone())
                    .tooltip(format!("Remove {item}"))
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit_list(ix, |list| Some(without_item(list, i)), cx);
                    }))
            })
            .collect();
        div()
            .id(SharedString::from(format!("prefs-field-{}", field.key)))
            .role(Role::Group)
            .aria_label(field.label)
            .v_flex()
            .gap_1()
            .pb_2()
            .border_b_1()
            .border_color(border)
            .child(div().text_xs().text_color(muted).child(field.label))
            .when(!chips.is_empty(), |d| d.child(div().h_flex().flex_wrap().gap_1().children(chips)))
            .child(Input::new(input).accessibility_id(format!("prefs-add-{}", field.key)).aria_label(format!("Add to {}", field.label)).disabled(busy))
    }

    fn render_summary(&mut self, view: &ChatView, cx: &mut Context<Self>) -> Div {
        let (border, muted, primary, secondary) = {
            let t = cx.theme();
            (t.border, t.muted_foreground, t.primary, t.secondary)
        };
        let busy = self.busy();
        let prefs = view.preferences.clone();
        let (salary_input, list_inputs) = match &self.form {
            Some(f) => (f.salary.clone(), f.lists.clone()),
            None => return div(),
        };

        let remote = div().h_flex().flex_wrap().gap_1().children([Remote::Any, Remote::Remote, Remote::Hybrid, Remote::Onsite].map(
            |r| {
                Button::new(SharedString::from(format!("prefs-remote-{}", r.label())))
                    .outline()
                    .compact()
                    .label(r.label())
                    .selected(prefs.remote == r)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.edit(|p| p.remote = r, cx);
                    }))
            },
        ));
        let salary_text = match (prefs.salary_min, prefs.currency.as_deref()) {
            (Some(n), Some(c)) => format!("At least {n} {c} per year"),
            (Some(n), None) => format!("At least {n} per year (no currency yet)"),
            (None, _) => "Not set".to_string(),
        };
        let weights = [
            ("Skills", prefs.weights.skills),
            ("Domain", prefs.weights.domain),
            ("Location", prefs.weights.location),
            ("Salary", prefs.weights.salary),
            ("Seniority", prefs.weights.seniority),
        ];
        let weight_rows = weights.map(|(label, value)| {
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(div().w(px(70.)).text_xs().child(label))
                .child(
                    div()
                        .flex_1()
                        .h(px(6.))
                        .rounded_full()
                        .bg(secondary)
                        .child(div().h_full().rounded_full().bg(primary).w(relative(value as f32 / 100.0))),
                )
                .child(div().w(px(28.)).text_xs().text_color(muted).child(value.to_string()))
        });
        let fields: Vec<Stateful<Div>> =
            (0..LIST_FIELDS.len()).map(|ix| self.render_list_field(ix, &prefs, &list_inputs[ix], cx)).collect();

        let visa = match prefs.visa_needed {
            Some(true) => Some("Needs visa sponsorship"),
            Some(false) => Some("Does not need visa sponsorship"),
            None => None,
        };
        let notes = (!prefs.notes.is_empty()).then(|| shorten(&prefs.notes, MAX_NOTES_CHARS.min(400)));

        div()
            .v_flex()
            .w(px(SUMMARY_WIDTH))
            .flex_shrink_0()
            .h_full()
            .gap_2()
            .p_2()
            .border_1()
            .border_color(border)
            .rounded_md()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().font_semibold().child("Your preferences"))
                    .child(
                        Button::new("prefs-undo")
                            .outline()
                            .compact()
                            .icon(IconName::Undo2)
                            .label("Undo last change")
                            .disabled(busy || !view.can_undo)
                            .on_click(cx.listener(|this, _, _, cx| this.undo(cx))),
                    ),
            )
            .child(div().text_xs().text_color(muted).child(if view.saved {
                "Every change is saved at once; edit anything here or tell the assistant."
            } else {
                "Nothing saved yet. Answer the assistant, or fill things in here."
            }))
            .child(
                div()
                    .id("prefs-summary")
                    .role(Role::Group)
                    .aria_label("Saved preferences")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .overflow_y_scroll()
                    .child(self.section_title("Where", cx))
                    .child(remote)
                    .child(
                        div()
                            .v_flex()
                            .gap_1()
                            .pb_2()
                            .border_b_1()
                            .border_color(border)
                            .child(div().text_xs().text_color(muted).child("Minimum salary"))
                            .child(div().text_sm().child(salary_text))
                            .child(
                                Input::new(&salary_input)
                                    .accessibility_id("prefs-salary")
                                    .aria_label("Minimum salary")
                                    .disabled(busy),
                            ),
                    )
                    .child(self.section_title("What", cx))
                    .children(fields)
                    .child(self.section_title("How much each counts", cx))
                    .child(div().v_flex().gap_1().pb_2().children(weight_rows))
                    .when_some(visa, |d, v| d.child(div().text_sm().child(v)))
                    .when_some(notes, |d, n| d.child(div().v_flex().gap_1().child(div().text_xs().text_color(muted).child("Notes")).child(div().text_sm().child(n)))),
            )
    }
}

impl Render for PreferencesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        if !self.shown {
            // First paint of this visit: read the chat (and let the assistant open it when nothing
            // was said yet).
            self.shown = true;
            cx.defer_in(window, |this, window, cx| this.run(Action::Open, window, cx));
        }
        let (muted, border) = {
            let t = cx.theme();
            (t.muted_foreground, t.border)
        };
        let message = self.form.as_ref().map(|f| f.message.clone());
        let view = self.view.clone();

        let body = match (view, message) {
            (Some(view), Some(message)) => {
                let chat = self.render_chat(&view, &message, cx);
                let summary = self.render_summary(&view, cx);
                div().h_flex().flex_1().min_h_0().gap_3().child(chat).child(summary).into_any_element()
            }
            _ => div()
                .flex_1()
                .p_2()
                .text_sm()
                .text_color(muted)
                .child(match self.failure() {
                    Some(f) => f.message,
                    None => "Loading...".to_string(),
                })
                .into_any_element(),
        };

        div()
            .id("preferences")
            .role(Role::Group)
            .aria_label("Preferences")
            .v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Preferences"))
                    .child(Button::new("preferences-back").ghost().label("Back").on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.show(Screen::Detail, cx))
                    }))),
            )
            .child(div().h(px(1.)).bg(border))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::graph::Failure;
    use jobpilot_core::health::Problem;
    use jobpilot_core::prefs::chat::MAX_LIST_ITEMS;
    use jobpilot_core::prefs::{ChatStatus, Preferences};

    // Not `super::*`: it would pull in gpui's `test` attribute and shadow the standard one.
    use super::{parse_salary, primary_for, with_item, without_item, Primary, LIST_FIELDS, MAX_ITEM_CHARS};

    #[test]
    fn exactly_one_primary_button_at_a_time() {
        let failed = ChatStatus::Failed(Failure { problem: Problem::Network, node: "ask".into(), message: "x".into() });
        let cases = [
            (None, Primary::Send),
            (Some(ChatStatus::Idle), Primary::Send),
            (Some(ChatStatus::Working), Primary::Send),
            (Some(ChatStatus::AwaitUser), Primary::Send),
            (Some(ChatStatus::AwaitConfirm), Primary::Confirm),
            (Some(ChatStatus::Accepted), Primary::Send),
            (Some(failed), Primary::Send),
        ];
        for (status, expected) in cases {
            // `Primary` has two values and the function returns one of them: the other is a plain button.
            assert_eq!(primary_for(status.as_ref()), expected, "{status:?}");
        }
    }

    #[test]
    fn items_are_added_without_blanks_duplicates_or_overflow() {
        let list = vec!["Rust".to_string()];
        assert_eq!(with_item(&list, "  Go "), Some(vec!["Rust".into(), "Go".into()]));
        assert_eq!(with_item(&list, "rust"), None, "already there, ignoring case");
        assert_eq!(with_item(&list, "   "), None);
        let full: Vec<String> = (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect();
        assert_eq!(with_item(&full, "one more"), None);
        assert_eq!(with_item(&[], &"x".repeat(500)).unwrap()[0].chars().count(), MAX_ITEM_CHARS);
        assert_eq!(without_item(&["a".into(), "b".into()], 0), ["b"]);
        assert_eq!(without_item(&["a".into()], 5), ["a"], "an index past the end changes nothing");
    }

    #[test]
    fn salary_box_parses_amount_and_currency() {
        assert_eq!(parse_salary("90000"), Ok((Some(90_000), None)));
        assert_eq!(parse_salary(" 90,000 usd "), Ok((Some(90_000), Some("USD".into()))));
        assert_eq!(parse_salary("1_500 EUR"), Ok((Some(1_500), Some("EUR".into()))));
        assert_eq!(parse_salary(""), Ok((None, None)), "empty clears");
        assert_eq!(parse_salary("0"), Ok((None, None)));
        for bad in ["lots", "90k", "90000 dollars", "90000 USD per year", "5000000000"] {
            assert!(parse_salary(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_list_field_reads_and_writes_its_own_list() {
        let mut prefs = Preferences::default();
        for (ix, field) in LIST_FIELDS.iter().enumerate() {
            (field.get_mut)(&mut prefs).push(format!("item {ix}"));
        }
        for (ix, field) in LIST_FIELDS.iter().enumerate() {
            assert_eq!((field.get)(&prefs), &vec![format!("item {ix}")], "{}", field.key);
        }
        let keys: std::collections::HashSet<_> = LIST_FIELDS.iter().map(|f| f.key).collect();
        assert_eq!(keys.len(), LIST_FIELDS.len(), "element ids stay unique");
    }
}
