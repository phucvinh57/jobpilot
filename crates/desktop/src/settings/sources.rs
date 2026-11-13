//! Settings > Job sources (P10): where job discovery looks, and in which browser session.
//!
//! Views only: what a source accepts is checked by `Core::add_job_source`; the sources, their last
//! run and the sessions come from `Core`. A source that needs a login is read in a managed session
//! the user logged in to under Browser sessions; a source that is paused at a login, captcha or block
//! page is continued from the Queue.

use std::future::Future;

use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::discover::{source_url_hint, JOB_SOURCE_KINDS};
use jobpilot_core::runtime;
use jobpilot_core::store::{Session, Source};

use super::section;
use super::sessions::{ago, host};
use crate::app::AppState;

/// Longest status text shown on a row (a failure note can be long).
const STATUS_CHARS: usize = 90;

/// How the last run went, for the colour of the status tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Never,
    Ok,
    NeedsYou,
    Failed,
    Other,
}

/// The status of a source as one line and its tone: when it last ran and how it ended.
fn status_line(source: &Source, now: DateTime<Utc>) -> (String, Outcome) {
    let when = |t: DateTime<Utc>| {
        let age = now - t;
        if age.num_minutes() < 1 {
            "just now".to_string()
        } else {
            ago(t)
        }
    };
    let status = source.last_status.as_deref().map(str::trim).unwrap_or_default();
    match (source.last_run_at, status) {
        (None, "") => ("never run".into(), Outcome::Never),
        (_, "needs_human") => ("needs you: continue it from the Queue".into(), Outcome::NeedsYou),
        (at, status) => {
            let outcome = if status == "ok" {
                Outcome::Ok
            } else if status.starts_with("failed") {
                Outcome::Failed
            } else {
                Outcome::Other
            };
            let status = if status.chars().count() > STATUS_CHARS {
                format!("{}...", status.chars().take(STATUS_CHARS).collect::<String>().trim_end())
            } else {
                status.to_string()
            };
            let line = match (at, status.is_empty()) {
                (Some(t), false) => format!("{status} - {}", when(t)),
                (Some(t), true) => format!("ran {}", when(t)),
                (None, _) => status,
            };
            (line, outcome)
        }
    }
}

/// The label of a source kind.
fn kind_label(kind: &str) -> &str {
    JOB_SOURCE_KINDS.iter().find(|(k, _)| *k == kind).map(|(_, label)| *label).unwrap_or(kind)
}

pub struct SourcesSection {
    state: Entity<AppState>,
    sources: Vec<Source>,
    sessions: Vec<Session>,
    /// Sources paused for the user right now.
    waiting: Vec<i64>,
    /// Reload from the store on the next render.
    dirty: bool,
    /// A reload is running on the core runtime.
    loading: bool,
    /// The first reload has finished (before that the list is not "empty", it is not there yet).
    loaded: bool,
    initialised: bool,
    form_open: bool,
    kind: &'static str,
    name_input: Option<Entity<InputState>>,
    url_input: Option<Entity<InputState>>,
    adding: bool,
    /// First click on Remove asks for confirmation; the second one removes.
    confirm_remove: Option<i64>,
    error: Option<String>,
}

impl SourcesSection {
    pub fn new(state: Entity<AppState>) -> Self {
        Self {
            state,
            sources: Vec::new(),
            sessions: Vec::new(),
            waiting: Vec::new(),
            dirty: true,
            loading: false,
            loaded: false,
            initialised: false,
            form_open: false,
            kind: JOB_SOURCE_KINDS[0].0,
            name_input: None,
            url_input: None,
            adding: false,
            confirm_remove: None,
            error: None,
        }
    }

    fn init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.initialised = true;
        self.name_input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Name (optional)")));
        self.url_input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Board name or address")));
        // Switching screens notifies the app state: pick up runs that finished meanwhile.
        cx.observe(&self.state, |this, _, cx| {
            this.dirty = true;
            cx.notify();
        })
        .detach();
    }

    /// Read the sources, the sessions and the paused runs from the store on the core runtime (the UI
    /// never reads the database itself). A change made while it runs sets `dirty` again, and the
    /// render after it reloads once more.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.state.read(cx).core.clone();
        self.dirty = false;
        self.loading = true;
        let rx = runtime::spawn(async move {
            let waiting = core.discover_waiting().map(|w| w.into_iter().map(|w| w.source_id).collect::<Vec<_>>()).unwrap_or_default();
            (core.job_sources(), core.sessions().list(), waiting)
        });
        cx.spawn(async move |this, cx| {
            let loaded = rx.await;
            this.update(cx, |this, cx| {
                this.loading = false;
                match loaded {
                    Ok((sources, sessions, waiting)) => {
                        match sources {
                            Ok(list) => this.sources = list,
                            Err(e) => this.error = Some(format!("{e:#}")),
                        }
                        match sessions {
                            Ok(list) => this.sessions = list,
                            Err(e) => this.error = Some(format!("{e:#}")),
                        }
                        this.waiting = waiting;
                        this.loaded = true;
                    }
                    Err(_) => this.error = Some("the task was cancelled".into()),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Run `fut` on the core runtime; `on_ok` runs on the UI thread afterwards.
    fn run(&mut self, fut: impl Future<Output = anyhow::Result<()>> + Send + 'static, on_ok: impl FnOnce(&mut Self) + 'static, cx: &mut Context<Self>) {
        self.error = None;
        self.confirm_remove = None;
        cx.notify();
        let rx = runtime::spawn(fut);
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(Ok(())) => on_ok(this),
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
        self.error = None;
        for input in [&self.name_input, &self.url_input].into_iter().flatten() {
            input.update(cx, |i, cx| i.set_value(String::new(), window, cx));
        }
        cx.notify();
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let (Some(n), Some(u)) = (&self.name_input, &self.url_input) else { return };
        let (name, url) = (n.read(cx).value().to_string(), u.read(cx).value().to_string());
        let (core, kind) = (self.state.read(cx).core.clone(), self.kind);
        self.adding = true;
        let rx = runtime::spawn(async move { core.add_job_source(kind, &name, &url, None).map(|_| ()) });
        self.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.adding = false;
                match result {
                    Ok(Ok(())) => this.form_open = false,
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

    fn set_enabled(&mut self, id: i64, enabled: bool, cx: &mut Context<Self>) {
        let core = self.state.read(cx).core.clone();
        self.run(async move { core.set_job_source_enabled(id, enabled) }, |_| {}, cx);
    }

    fn set_session(&mut self, id: i64, session: Option<i64>, cx: &mut Context<Self>) {
        let core = self.state.read(cx).core.clone();
        self.run(async move { core.set_job_source_session(id, session) }, |_| {}, cx);
    }

    fn remove(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.confirm_remove != Some(id) {
            self.confirm_remove = Some(id);
            cx.notify();
            return;
        }
        let core = self.state.read(cx).core.clone();
        self.run(async move { core.remove_job_source(id) }, |_| {}, cx);
    }
}

impl Render for SourcesSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.initialised {
            self.init(window, cx);
        }
        if self.dirty && !self.loading {
            self.reload(cx);
        }
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let now = Utc::now();

        let rows: Vec<_> = self
            .sources
            .iter()
            .map(|s| {
                let id = s.id;
                let name = if s.name.trim().is_empty() { kind_label(&s.kind).to_string() } else { s.name.trim().to_string() };
                let (line, outcome) = status_line(s, now);
                let waiting = self.waiting.contains(&id);
                let tag = match if waiting { Outcome::NeedsYou } else { outcome } {
                    Outcome::Ok => Tag::success(),
                    Outcome::NeedsYou => Tag::warning(),
                    Outcome::Failed => Tag::danger(),
                    Outcome::Never | Outcome::Other => Tag::secondary(),
                };
                let tag_text = match (waiting, outcome) {
                    (true, _) | (_, Outcome::NeedsYou) => "needs you",
                    (_, Outcome::Ok) => "ok",
                    (_, Outcome::Failed) => "failed",
                    (_, Outcome::Never) => "new",
                    (_, Outcome::Other) => "idle",
                };
                let address = if s.url.trim().is_empty() { String::new() } else { host(&s.url) };
                let confirm = self.confirm_remove == Some(id);

                let session_chips = {
                    let none = Button::new(SharedString::from(format!("source-{id}-session-none")))
                        .small()
                        .label("No session")
                        .selected(s.session_id.is_none())
                        .on_click(cx.listener(move |this, _, _, cx| this.set_session(id, None, cx)));
                    let chips = self.sessions.iter().map(|session| {
                        let sid = session.id;
                        Button::new(SharedString::from(format!("source-{id}-session-{sid}")))
                            .small()
                            .label(session.name.clone())
                            .selected(s.session_id == Some(sid))
                            .on_click(cx.listener(move |this, _, _, cx| this.set_session(id, Some(sid), cx)))
                    });
                    div().h_flex().flex_wrap().items_center().gap_1().child(div().text_xs().text_color(muted).child("Browser session:")).child(none).children(chips)
                };

                div()
                    .id(SharedString::from(format!("source-{id}")))
                    .role(Role::ListItem)
                    .aria_label(format!("{name}: {line}"))
                    .v_flex()
                    .gap_1()
                    .p_2()
                    .border_1()
                    .border_color(border)
                    .rounded_md()
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .child(
                                Switch::new(SharedString::from(format!("source-{id}-enabled")))
                                    .checked(s.enabled)
                                    .accessibility_label(format!("Search {name}"))
                                    .on_click(cx.listener(move |this, checked: &bool, _, cx| this.set_enabled(id, *checked, cx))),
                            )
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .child(div().font_semibold().truncate().child(name.clone()))
                                    .child(div().text_xs().text_color(muted).truncate().child(format!("{} {address}", kind_label(&s.kind)).trim().to_string())),
                            )
                            .child(tag.child(tag_text))
                            .child(
                                Button::new(SharedString::from(format!("source-{id}-remove")))
                                    .small()
                                    .outline()
                                    .danger()
                                    .label(if confirm { "Sure?" } else { "Remove" })
                                    .on_click(cx.listener(move |this, _, _, cx| this.remove(id, cx))),
                            ),
                    )
                    .child(div().text_xs().text_color(muted).child(line))
                    .child(session_chips)
            })
            .collect();

        let form = if self.form_open {
            let kind_chips = JOB_SOURCE_KINDS.iter().map(|(kind, label)| {
                let kind: &'static str = kind;
                Button::new(SharedString::from(format!("new-source-kind-{kind}")))
                    .small()
                    .label(*label)
                    .selected(self.kind == kind)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.kind = kind;
                        cx.notify();
                    }))
            });
            div()
                .v_flex()
                .gap_2()
                .p_3()
                .border_1()
                .border_color(border)
                .rounded_md()
                .child(div().h_flex().flex_wrap().gap_1().children(kind_chips))
                .children(self.name_input.as_ref().map(Input::new))
                .children(self.url_input.as_ref().map(Input::new))
                .child(div().text_xs().text_color(muted).child(source_url_hint(self.kind)))
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("save-source")
                                .small()
                                .primary()
                                .label("Add")
                                .loading(self.adding)
                                .disabled(self.adding)
                                .on_click(cx.listener(|this, _, _, cx| this.add(cx))),
                        )
                        .child(
                            Button::new("cancel-source")
                                .small()
                                .ghost()
                                .label("Cancel")
                                .disabled(self.adding)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.form_open = false;
                                    this.error = None;
                                    cx.notify();
                                })),
                        ),
                )
                .into_any_element()
        } else {
            div()
                .h_flex()
                .child(
                    Button::new("add-source")
                        .small()
                        .primary()
                        .label("Add source")
                        .on_click(cx.listener(|this, _, window, cx| this.open_form(window, cx))),
                )
                .into_any_element()
        };

        section(
            "Job sources",
            "Where to look for jobs. Public boards and Hacker News need no login; for a site that wants one, pick a browser session you logged in to.",
            cx,
        )
        .child(div().v_flex().gap_2().children(rows))
        .when(self.loaded && self.sources.is_empty() && !self.form_open, |d| {
            d.child(div().text_sm().text_color(muted).child("No sources yet. Add a company board, a careers page or Hacker News."))
        })
        .child(form)
        .children(self.error.clone().map(|e| div().text_sm().text_color(cx.theme().danger).child(e)))
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: the glob would bring in gpui's `test` attribute and shadow `#[test]`.
    use jobpilot_core::chrono::{DateTime, Utc};
    use jobpilot_core::store::{Source, SourcePurpose};

    use super::{kind_label, status_line, Outcome};

    fn source(last_run_at: Option<DateTime<Utc>>, last_status: Option<&str>) -> Source {
        Source {
            id: 1,
            kind: "greenhouse".into(),
            name: "Acme".into(),
            url: "https://boards.greenhouse.io/acme".into(),
            purpose: SourcePurpose::Jobs,
            enabled: true,
            session_id: None,
            last_run_at,
            last_status: last_status.map(str::to_string),
        }
    }

    #[test]
    fn a_status_line_says_when_and_how_the_last_run_ended() {
        let now = Utc::now();
        let hours = |h| Some(now - jobpilot_core::chrono::Duration::hours(h));
        assert_eq!(status_line(&source(None, None), now), ("never run".to_string(), Outcome::Never));
        assert_eq!(status_line(&source(hours(3), Some("ok")), now), ("ok - 3h ago".to_string(), Outcome::Ok));
        assert_eq!(status_line(&source(Some(now), Some("ok")), now).0, "ok - just now");
        let (line, outcome) = status_line(&source(hours(1), Some("failed: fetching boards.test: HTTP 503")), now);
        assert_eq!((line.as_str(), outcome), ("failed: fetching boards.test: HTTP 503 - 1h ago", Outcome::Failed));
        assert_eq!(status_line(&source(None, Some("needs_human")), now).1, Outcome::NeedsYou);
        assert_eq!(status_line(&source(hours(2), Some("needs_human")), now).1, Outcome::NeedsYou, "a paused source is not finished");
        assert_eq!(status_line(&source(hours(30), Some("paused for today: repeated challenges")), now).1, Outcome::Other);
        assert_eq!(status_line(&source(hours(2), None), now).0, "ran 2h ago");
        let long = format!("failed: {}", "x".repeat(300));
        let (line, _) = status_line(&source(hours(1), Some(&long)), now);
        assert!(line.chars().count() < 120 && line.contains("..."), "{line}");
    }

    #[test]
    fn kinds_have_labels_and_unknown_ones_show_as_they_are() {
        assert_eq!(kind_label("career_page"), "Career page");
        assert_eq!(kind_label("yc"), "Y Combinator");
        assert_eq!(kind_label("linkedin"), "linkedin");
    }
}
