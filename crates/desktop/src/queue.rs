//! Queue screen (P12): found jobs best match first, with the brief and the score breakdown, and
//! the actions that turn a lead into an application (Prepare CV), drop it (Dismiss) or look at it
//! (Open posting). Views only: the data comes from `Core::queue_snapshot`, the work from
//! `Core::prepare_cv` / `discover_now` / `dismiss_lead_with_reason`, all on the core runtime.
//!
//! The one primary button is at the bottom right: "Prepare CV" while there are leads, "Search now"
//! when the queue is empty (`logic::queue_primary_for`). Everything else is a plain button.

mod logic;

use std::collections::{BTreeSet, HashMap};

use anyhow::{anyhow, Result};
use gpui_kit::component::button::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::discover::promote::{parse_prepare_thread, Dismissed, PREPARE_GRAPH};
use jobpilot_core::discover::queue::{criteria_rows, filter_items, QueueFilter, QueueItem, QueueSnapshot};
use jobpilot_core::discover::{WaitingSource, SOURCE_GRAPH};
use jobpilot_core::graph::{ProgressEvent, WaitReason};
use jobpilot_core::health;
use jobpilot_core::runtime;
use jobpilot_core::store::Application;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::unbounded_channel;

use crate::app::{AppState, Screen};
use crate::detail::insight_brief;
use crate::problem::{self, RetryFn};
use logic::{
    ago, dismissed_note, drop_unreported, is_web_link, meta_line, queue_primary_for, score_label, score_tone, source_name, source_needs_attention,
    source_status, QueueAction, QueueFacts, Tone,
};

/// Minimum scores the filter offers.
const MIN_SCORES: [f64; 3] = [50.0, 70.0, 85.0];

/// Where a lead stands in "Prepare CV" during this session.
#[derive(Clone, Debug)]
enum Job {
    /// Waiting for the CV before it to be written.
    Queued,
    /// Running, with the pipeline's progress line.
    Running(String),
}

impl Job {
    fn label(&self) -> &str {
        match self {
            Job::Queued => "Waiting for its turn",
            Job::Running(label) => label,
        }
    }
}

/// A CV that was written: the way into the application.
struct Prepared {
    app_id: i64,
    label: String,
}

/// An inline message under the header.
struct Note {
    ok: bool,
    text: String,
}

/// The open "Dismiss" form.
struct DismissForm {
    lead_id: i64,
    exclude_company: bool,
    error: Option<String>,
    busy: bool,
}

/// One "Prepare CV" that did not finish, as the list of failures shows it.
struct FailureRow {
    lead_id: i64,
    title: String,
    message: String,
    /// The application that was already created for the lead.
    app_id: Option<i64>,
}

/// The queue of found jobs.
pub struct QueueView {
    state: Entity<AppState>,
    /// The `AppState::queue_epoch` this view last loaded for.
    epoch: Option<u64>,
    snap: Option<QueueSnapshot>,
    load_error: Option<String>,
    filter: QueueFilter,
    location: Entity<InputState>,
    reason: Entity<InputState>,
    /// Ticked leads (several CVs at once).
    checked: BTreeSet<i64>,
    jobs: HashMap<i64, Job>,
    /// Failures this session that left no thread behind (no provider, ...), by lead.
    failed: HashMap<i64, String>,
    prepared: Vec<Prepared>,
    /// A search is running: its progress line.
    searching: Option<String>,
    note: Option<Note>,
    dismiss: Option<DismissForm>,
    /// Threads of waiting sources being continued.
    continuing: BTreeSet<String>,
}

impl QueueView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let location = cx.new(|cx| InputState::new(window, cx).placeholder("Location contains..."));
        cx.subscribe_in(&location, window, |this, input, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.filter.location = input.read(cx).value().to_string();
                this.normalize_selection(cx);
                cx.notify();
            }
        })
        .detach();
        let reason = cx.new(|cx| InputState::new(window, cx).placeholder("Why? (optional)"));
        let view = Self {
            state,
            epoch: None,
            snap: None,
            load_error: None,
            filter: QueueFilter::default(),
            location,
            reason,
            checked: BTreeSet::new(),
            jobs: HashMap::new(),
            failed: HashMap::new(),
            prepared: Vec::new(),
            searching: None,
            note: None,
            dismiss: None,
            continuing: BTreeSet::new(),
        };
        view.watch(cx);
        view
    }

    // ------------------------------------------------------------ data

    fn core(&self, cx: &App) -> std::sync::Arc<jobpilot_core::Core> {
        self.state.read(cx).core.clone()
    }

    /// Follow the graph runtime: the CV pipeline's progress, and sources that ended (their leads
    /// are saved by then).
    fn watch(&self, cx: &mut Context<Self>) {
        let mut rx = self.state.read(cx).core.graphs.subscribe();
        cx.spawn(async move |this, cx| loop {
            match rx.recv().await {
                Ok(progress) => {
                    if this.update(cx, |this, cx| this.on_progress(progress, cx)).is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
        })
        .detach();
    }

    fn on_progress(&mut self, p: jobpilot_core::graph::Progress, cx: &mut Context<Self>) {
        if p.graph == PREPARE_GRAPH {
            let Some(id) = parse_prepare_thread(&p.thread) else { return };
            let label = match &p.event {
                ProgressEvent::NodeStarted { node } if node == "promote" => Some("Creating the application".to_string()),
                ProgressEvent::NodeStarted { node } if node == "generate" => Some("Writing the CV".to_string()),
                ProgressEvent::Detail { text, .. } => Some(text.clone()),
                _ => None,
            };
            if let (Some(label), Some(job)) = (label, self.jobs.get_mut(&id)) {
                *job = Job::Running(label);
                cx.notify();
            }
        } else if p.graph == SOURCE_GRAPH
            && matches!(p.event, ProgressEvent::Done | ProgressEvent::Waiting(_) | ProgressEvent::Failed)
        {
            self.refresh(cx);
        }
    }

    /// Read the queue again (off the UI thread).
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let rx = runtime::spawn(async move { core.queue_snapshot() });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| this.on_snapshot(result, cx)).ok();
        })
        .detach();
    }

    fn on_snapshot(&mut self, result: Result<QueueSnapshot>, cx: &mut Context<Self>) {
        match result {
            Ok(snap) => {
                self.load_error = None;
                self.checked.retain(|id| snap.items.iter().any(|i| i.lead.id == *id));
                self.snap = Some(snap);
                self.normalize_selection(cx);
            }
            Err(e) => self.load_error = Some(format!("{e:#}")),
        }
        cx.notify();
    }

    fn items(&self) -> &[QueueItem] {
        self.snap.as_ref().map_or(&[], |s| s.items.as_slice())
    }

    fn shown(&self) -> Vec<&QueueItem> {
        filter_items(self.items(), &self.filter)
    }

    fn selected_id(&self, cx: &App) -> Option<i64> {
        self.state.read(cx).queue_selected
    }

    fn set_selected(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        self.state.update(cx, |s, _| s.queue_selected = id);
        // A form for another lead is not the open lead's.
        if self.dismiss.as_ref().is_some_and(|d| Some(d.lead_id) != id) {
            self.dismiss = None;
        }
    }

    /// The open lead must be one that is listed: else the first one.
    fn normalize_selection(&mut self, cx: &mut Context<Self>) {
        let shown: Vec<i64> = self.shown().iter().map(|i| i.lead.id).collect();
        let current = self.selected_id(cx);
        if current.is_none_or(|id| !shown.contains(&id)) {
            self.set_selected(shown.first().copied(), cx);
        }
    }

    /// The lead to open once `id` leaves the list: the one after it, else the one before.
    fn neighbour_of(&self, id: i64) -> Option<i64> {
        let shown = self.shown();
        let at = shown.iter().position(|i| i.lead.id == id)?;
        shown.get(at + 1).or_else(|| at.checked_sub(1).and_then(|p| shown.get(p))).map(|i| i.lead.id)
    }

    /// The leads "Prepare CV" acts on, in queue order: the ticked ones, else the open one.
    fn targets(&self, cx: &App) -> Vec<i64> {
        let shown = self.shown();
        if !self.checked.is_empty() {
            return shown.iter().map(|i| i.lead.id).filter(|id| self.checked.contains(id)).collect();
        }
        self.selected_id(cx).filter(|id| shown.iter().any(|i| i.lead.id == *id)).into_iter().collect()
    }

    // ------------------------------------------------------------ actions

    fn search_now(&mut self, cx: &mut Context<Self>) {
        if self.searching.is_some() {
            return;
        }
        self.searching = Some("Searching your sources".into());
        self.note = None;
        let core = self.core(cx);
        let rx = runtime::spawn(async move { core.discover_now().await });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| this.on_searched(result, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn on_searched(&mut self, result: Result<jobpilot_core::discover::RunSummary>, cx: &mut Context<Self>) {
        self.searching = None;
        self.note = Some(match result {
            Ok(s) => Note { ok: true, text: summary_line(&s) },
            Err(e) => {
                let message = format!("{e:#}");
                self.state.update(cx, |s, cx| s.needs_recheck(&message, cx));
                Note { ok: false, text: message }
            }
        });
        self.refresh(cx);
    }

    /// Write the CVs of `ids`, one after the other (the core queues across clicks too).
    fn prepare(&mut self, ids: Vec<i64>, cx: &mut Context<Self>) {
        let ids: Vec<i64> = ids.into_iter().filter(|id| !self.jobs.contains_key(id)).collect();
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            self.jobs.insert(*id, Job::Queued);
            self.failed.remove(id);
        }
        self.checked.clear();
        let core = self.core(cx);
        let (tx, mut rx) = unbounded_channel();
        let mut unreported: BTreeSet<i64> = ids.iter().copied().collect();
        // The core holds every lead of the batch as "being prepared" from the start.
        drop(runtime::spawn(async move {
            core.prepare_cvs_each(&ids, |id, result| {
                let _ = tx.send((id, result));
            })
            .await;
        }));
        cx.spawn(async move |this, cx| {
            while let Some((id, result)) = rx.recv().await {
                unreported.remove(&id);
                if this.update(cx, |this, cx| this.on_prepared(id, result, cx)).is_err() {
                    return;
                }
            }
            // The task ended without reporting these (it was cancelled or panicked): they are not
            // running any more, whatever the list says.
            if !unreported.is_empty() {
                this.update(cx, |this, cx| {
                    drop_unreported(&mut this.jobs, &mut this.failed, unreported);
                    this.refresh(cx);
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
        cx.notify();
    }

    fn on_prepared(&mut self, id: i64, result: Result<Application>, cx: &mut Context<Self>) {
        self.jobs.remove(&id);
        // The lead left the queue: open the one next to it.
        if self.selected_id(cx) == Some(id) {
            let next = self.neighbour_of(id);
            self.set_selected(next, cx);
        }
        match result {
            Ok(app) => {
                tracing::info!(lead = id, app = app.id, "CV prepared");
                self.prepared.push(Prepared { app_id: app.id, label: format!("{} \u{2014} {}", app.company, app.role) });
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.state.update(cx, |s, cx| s.needs_recheck(&message, cx));
                self.failed.insert(id, message);
            }
        }
        // The application is in the sidebar now, whether or not the CV worked out.
        self.state.update(cx, |s, cx| s.reload(cx));
        self.refresh(cx);
    }

    fn forget_failure(&mut self, lead_id: i64, cx: &mut Context<Self>) {
        self.failed.remove(&lead_id);
        let core = self.core(cx);
        let rx = runtime::spawn(async move { core.prepare_forget(lead_id) });
        cx.spawn(async move |this, cx| {
            let _ = rx.await;
            this.update(cx, |this, cx| this.refresh(cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn open_dismiss(&mut self, lead_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.reason.update(cx, |input, cx| input.set_value("", window, cx));
        self.dismiss = Some(DismissForm { lead_id, exclude_company: false, error: None, busy: false });
        cx.notify();
    }

    fn confirm_dismiss(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.dismiss.as_mut().filter(|f| !f.busy) else { return };
        form.busy = true;
        let (id, exclude) = (form.lead_id, form.exclude_company);
        let reason = self.reason.read(cx).value().to_string();
        let core = self.core(cx);
        let rx = runtime::spawn(async move { core.dismiss_lead_with_reason(id, &reason, exclude) });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| this.on_dismissed(id, result, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn on_dismissed(&mut self, id: i64, result: Result<Dismissed>, cx: &mut Context<Self>) {
        match result {
            Ok(out) => {
                self.dismiss = None;
                self.checked.remove(&id);
                if self.selected_id(cx) == Some(id) {
                    let next = self.neighbour_of(id);
                    self.set_selected(next, cx);
                }
                self.note = Some(Note { ok: out.not_excluded.is_none(), text: dismissed_note(&out) });
                self.refresh(cx);
            }
            Err(e) => {
                if let Some(form) = self.dismiss.as_mut() {
                    form.busy = false;
                    form.error = Some(format!("{e:#}"));
                }
            }
        }
        cx.notify();
    }

    /// Continue a source that waited for the user (a login, a captcha) or retry a failed one.
    fn continue_source(&mut self, thread: String, cx: &mut Context<Self>) {
        if !self.continuing.insert(thread.clone()) {
            return;
        }
        let core = self.core(cx);
        let t = thread.clone();
        let rx = runtime::spawn(async move { core.discover_resume(&t).await });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                this.continuing.remove(&thread);
                if let Err(e) = result {
                    this.note = Some(Note { ok: false, text: format!("{e:#}") });
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    fn skip_source(&mut self, thread: String, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let rx = runtime::spawn(async move { core.discover_cancel(&thread) });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                if let Err(e) = result {
                    this.note = Some(Note { ok: false, text: format!("{e:#}") });
                }
                this.refresh(cx);
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ pieces

    fn tone_color(&self, tone: Tone, cx: &App) -> Hsla {
        let theme = cx.theme();
        match tone {
            Tone::High => theme.success,
            Tone::Mid => theme.warning,
            Tone::Low => theme.danger,
            Tone::Unknown => theme.muted_foreground,
        }
    }

    fn header(&self, empty: bool, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let last = self.snap.as_ref().and_then(|s| s.last_run.as_ref()).map(|run| {
            let when = DateTime::parse_from_rfc3339(&run.finished_at)
                .map(|t| ago(Utc::now(), t.with_timezone(&Utc)))
                .unwrap_or_else(|_| "earlier".into());
            format!("Last search {when}: {}", summary_line(run))
        });
        div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child("Queue"))
                    .child(
                        div()
                            .h_flex()
                            .gap_2()
                            // When the queue is empty "Search now" is the primary button, below.
                            .when(!empty, |d| {
                                d.child(
                                    Button::new("queue-search")
                                        .outline()
                                        .icon(IconName::Search)
                                        .label("Search now")
                                        .loading(self.searching.is_some())
                                        .disabled(self.searching.is_some())
                                        .on_click(cx.listener(|this, _, _, cx| this.search_now(cx))),
                                )
                            })
                            .child(Button::new("queue-back").ghost().label("Back").on_click(cx.listener(
                                |this, _, _, cx| this.state.update(cx, |s, cx| s.show(Screen::Detail, cx)),
                            ))),
                    ),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(muted)
                    .when_some(self.searching.clone(), |d, label| {
                        d.child(Spinner::new().small().color(theme.primary)).child(label)
                    })
                    .when(self.searching.is_none(), |d| d.child(last.unwrap_or_else(|| "No search yet.".into()))),
            )
    }

    fn sources_line(&self, cx: &App) -> Option<Stateful<Div>> {
        let snap = self.snap.as_ref().filter(|s| !s.sources.is_empty())?;
        let theme = cx.theme();
        let now = Utc::now();
        Some(
            div()
                .id("queue-sources")
                .role(Role::List)
                .aria_label("Job sources")
                .h_flex()
                .flex_wrap()
                .gap_x_4()
                .gap_y_0p5()
                .text_xs()
                .children(snap.sources.iter().map(|s| {
                    let color = if source_needs_attention(s) { theme.warning } else { theme.muted_foreground };
                    let text = format!("{}: {}{}", source_name(s), source_status(s, now), if s.enabled { "" } else { " (off)" });
                    div().id(("source", s.id as usize)).role(Role::ListItem).aria_label(text.clone()).text_color(color).child(text)
                })),
        )
    }

    fn note_line(&self, cx: &App) -> Option<Stateful<Div>> {
        let note = self.note.as_ref()?;
        let theme = cx.theme();
        Some(
            div()
                .id("queue-note")
                .role(Role::Paragraph)
                .aria_label(note.text.clone())
                .text_sm()
                .text_color(if note.ok { theme.muted_foreground } else { theme.danger })
                .child(note.text.clone()),
        )
    }

    /// One row per source waiting for the user: Continue, Skip (and, once the browser hand-off
    /// exists, Open browser).
    fn waiting_banner(&self, w: &WaitingSource, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let (what, url) = match &w.reason {
            WaitReason::Login { url, .. } => ("needs you to sign in", Some(url.clone())),
            WaitReason::Captcha { url, .. } => ("shows a captcha", Some(url.clone())),
            WaitReason::Blocked { url, .. } => ("blocks automated access", Some(url.clone())),
            _ => ("is waiting for you", None),
        };
        let (thread, skip_thread) = (w.thread.clone(), w.thread.clone());
        let busy = self.continuing.contains(&w.thread);
        let id = w.source_id;
        div()
            .id(("waiting", id as usize))
            .role(Role::Group)
            .aria_label(format!("{} {what}", w.source_name))
            .h_flex()
            .items_center()
            .gap_3()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(theme.warning)
            .bg(theme.warning.opacity(0.08))
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().font_semibold().child(format!("{} {what}", w.source_name)))
                    .when_some(url.filter(|u| is_web_link(u)), |d, url| {
                        d.child(div().text_xs().truncate().text_color(theme.muted_foreground).child(url))
                    }),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    // TODO(P10 part B): "Open browser" goes here, next to Continue:
                    //   Button::new(("waiting-open", id)).outline().compact().icon(IconName::ExternalLink)
                    //       .label("Open browser").on_click(.. core.discover_open_browser(&thread) ..)
                    // `Core::discover_open_browser` lands with discover/handoff.rs; wire it after both merge.
                    .child(
                        Button::new(SharedString::from(format!("waiting-continue-{id}")))
                            .outline()
                            .compact()
                            .label("Continue")
                            .loading(busy)
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.continue_source(thread.clone(), cx))),
                    )
                    .child(
                        Button::new(SharedString::from(format!("waiting-skip-{id}")))
                            .ghost()
                            .compact()
                            .label("Skip")
                            .disabled(busy)
                            .on_click(cx.listener(move |this, _, _, cx| this.skip_source(skip_thread.clone(), cx))),
                    ),
            )
    }

    /// "Prepare CV" failures: the message, what to do about it, Retry.
    fn failure_rows(&self) -> Vec<FailureRow> {
        let mut rows = Vec::new();
        if let Some(snap) = &self.snap {
            for f in snap.failed.iter().filter(|f| !self.jobs.contains_key(&f.lead.id)) {
                rows.push(FailureRow {
                    lead_id: f.lead.id,
                    title: format!("{} \u{2014} {}", f.lead.company, f.lead.role),
                    message: f.failure.message.clone(),
                    app_id: f.lead.app_id,
                });
            }
        }
        for (id, message) in &self.failed {
            if rows.iter().any(|r| r.lead_id == *id) {
                continue;
            }
            let title = self
                .items()
                .iter()
                .find(|i| i.lead.id == *id)
                .map_or_else(|| format!("Lead {id}"), |i| format!("{} \u{2014} {}", i.lead.company, i.lead.role));
            rows.push(FailureRow { lead_id: *id, title, message: message.clone(), app_id: None });
        }
        rows.sort_by_key(|r| r.lead_id);
        rows
    }

    fn failure_card(&self, row: FailureRow, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let (id, app_id) = (row.lead_id, row.app_id);
        let problem = health::classify(&row.message);
        let retry: RetryFn = {
            let this = cx.entity();
            std::rc::Rc::new(move |_, cx| this.update(cx, |this, cx| this.prepare(vec![id], cx)))
        };
        let help = problem::help(&format!("prepare-error-{id}"), problem, &self.state, Some(retry), cx);
        div()
            .id(("failure", id as usize))
            .role(Role::Group)
            .aria_label(format!("The CV for {} could not be written", row.title))
            .v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(theme.danger)
            .child(div().text_sm().font_semibold().child(format!("The CV for {} could not be written", row.title)))
            .child(
                div()
                    .id(("failure-text", id as usize))
                    .max_h(px(72.))
                    .overflow_y_scroll()
                    .font_family("monospace")
                    .text_xs()
                    .text_color(theme.danger)
                    .child(row.message),
            )
            .children(help)
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .when(!problem.retryable(), |d| {
                        d.child(
                            Button::new(SharedString::from(format!("prepare-retry-{id}")))
                                .outline()
                                .compact()
                                .label("Try again")
                                .on_click(cx.listener(move |this, _, _, cx| this.prepare(vec![id], cx))),
                        )
                    })
                    .when_some(app_id, |d, app_id| {
                        d.child(
                            Button::new(SharedString::from(format!("prepare-open-{id}")))
                                .outline()
                                .compact()
                                .label("Open application")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.update(cx, |s, cx| s.select(app_id, cx))
                                })),
                        )
                    })
                    .child(
                        Button::new(SharedString::from(format!("prepare-forget-{id}")))
                            .ghost()
                            .compact()
                            .label("Forget")
                            .on_click(cx.listener(move |this, _, _, cx| this.forget_failure(id, cx))),
                    ),
            )
    }

    fn prepared_row(&self, index: usize, p: &Prepared, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let app_id = p.app_id;
        div()
            .id(("prepared", index))
            .role(Role::Group)
            .aria_label(format!("CV ready for {}", p.label))
            .h_flex()
            .items_center()
            .gap_3()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(theme.success)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .truncate()
                    .child(format!("CV ready for {}: read it, then approve it.", p.label)),
            )
            .child(
                Button::new(SharedString::from(format!("prepared-open-{index}")))
                    .outline()
                    .compact()
                    .label("Open application")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.select(app_id, cx));
                    })),
            )
            .child(
                Button::new(SharedString::from(format!("prepared-close-{index}")))
                    .ghost()
                    .compact()
                    .icon(IconName::Close)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index < this.prepared.len() {
                            this.prepared.remove(index);
                        }
                        cx.notify();
                    })),
            )
    }

    fn filter_bar(&self, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let sources: Vec<(i64, String)> =
            self.snap.iter().flat_map(|s| s.sources.iter()).map(|s| (s.id, source_name(s))).collect();
        let chip = |id: SharedString, label: String, on: bool| Button::new(id).ghost().compact().label(label).selected(on);
        let caption = |text: &'static str| div().text_xs().text_color(muted).child(text);
        let (shown, total) = (self.shown().len(), self.items().len());
        div()
            .id("queue-filters")
            .role(Role::Group)
            .aria_label("Filters")
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_x_4()
            .gap_y_1()
            .when(sources.len() > 1, |d| {
                d.child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_1()
                        .child(caption("Source"))
                        .child(
                            chip("flt-src-all".into(), "All".into(), self.filter.source_id.is_none())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.filter.source_id = None;
                                    this.normalize_selection(cx);
                                    cx.notify();
                                })),
                        )
                        .children(sources.iter().map(|(id, name)| {
                            let id = *id;
                            chip(
                                SharedString::from(format!("flt-src-{id}")),
                                name.clone(),
                                self.filter.source_id == Some(id),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.filter.source_id = Some(id);
                                this.normalize_selection(cx);
                                cx.notify();
                            }))
                        })),
                )
            })
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_1()
                    .child(caption("Min score"))
                    .child(chip("flt-score-any".into(), "Any".into(), self.filter.min_score.is_none()).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.filter.min_score = None;
                            this.normalize_selection(cx);
                            cx.notify();
                        }),
                    ))
                    .children(MIN_SCORES.iter().map(|min| {
                        let min = *min;
                        chip(
                            SharedString::from(format!("flt-score-{min:.0}")),
                            format!("{min:.0}+"),
                            self.filter.min_score == Some(min),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.filter.min_score = Some(min);
                            this.normalize_selection(cx);
                            cx.notify();
                        }))
                    })),
            )
            .child(div().w(px(200.)).child(Input::new(&self.location)))
            .child(div().text_xs().text_color(muted).child(format!("{shown} of {total} shown")))
    }

    fn lead_row(&self, item: &QueueItem, selected: bool, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let lead = &item.lead;
        let id = lead.id;
        let tone = self.tone_color(score_tone(lead.score), cx);
        let job = self.jobs.get(&id).cloned();
        let source = self
            .snap
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| Some(s.id) == lead.source_id))
            .map(source_name);
        let reasons = item
            .reasons(2)
            .iter()
            .map(|r| format!("{} {}: {}", r.criterion, r.score, r.text))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .when(selected, |d| d.bg(theme.accent))
            .rounded_md()
            .px_2()
            .py_1p5()
            .child(
                div().pt_0p5().child(
                    Checkbox::new(SharedString::from(format!("lead-check-{id}")))
                        .accessibility_label(format!("Select {}: {}", lead.company, lead.role))
                        .checked(self.checked.contains(&id))
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            if *checked {
                                this.checked.insert(id);
                            } else {
                                this.checked.remove(&id);
                            }
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .id(("lead", id as usize))
                    .role(Role::ListItem)
                    .aria_label(format!("{}: {}, score {}", lead.company, lead.role, score_label(lead.score)))
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.accent.opacity(0.5)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_selected(Some(id), cx);
                        cx.notify();
                    }))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_semibold()
                                    .child(format!("{} \u{2014} {}", lead.company, lead.role)),
                            )
                            .child(div().flex_shrink_0().font_semibold().text_color(tone).child(score_label(lead.score))),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(meta_line(&lead.location, source.as_deref())),
                    )
                    .when(!reasons.is_empty(), |d| {
                        d.child(div().truncate().text_xs().text_color(theme.muted_foreground).child(reasons))
                    })
                    .when_some(job, |d, job| {
                        d.child(
                            div()
                                .h_flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(theme.primary)
                                .child(Spinner::new().xsmall().color(theme.primary))
                                .child(job.label().to_string()),
                        )
                    }),
            )
    }

    fn list(&self, cx: &Context<Self>) -> Stateful<Div> {
        let selected = self.selected_id(cx);
        let rows: Vec<Div> = self.shown().into_iter().map(|item| self.lead_row(item, selected == Some(item.lead.id), cx)).collect();
        let theme = cx.theme();
        div()
            .id("queue-list")
            .role(Role::List)
            .aria_label("Leads")
            .v_flex()
            .w(px(340.))
            .flex_shrink_0()
            .h_full()
            .gap_0p5()
            .overflow_y_scroll()
            .pr_2()
            .border_r_1()
            .border_color(theme.border)
            .children(rows)
    }

    fn score_bar(&self, score: u32, tone: Hsla, cx: &App) -> Div {
        let theme = cx.theme();
        div()
            .w(px(80.))
            .h(px(6.))
            .flex_shrink_0()
            .rounded_full()
            .bg(theme.border)
            .child(div().h_full().w(px(f32::from(u16::try_from(score.min(100)).unwrap_or(0)) * 0.8)).rounded_full().bg(tone))
    }

    fn breakdown(&self, item: &QueueItem, cx: &App) -> Option<Stateful<Div>> {
        let scored = item.scored.as_ref()?;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let total_tone = self.tone_color(score_tone(item.lead.score), cx);
        let rows = criteria_rows(scored).into_iter().map(|r| {
            let tone = if r.detail.known { self.tone_color(score_tone(Some(f64::from(r.detail.score))), cx) } else { muted };
            let reason = if r.detail.known {
                r.detail.reason.clone()
            } else {
                "not judged: the posting or your preferences say nothing about it".to_string()
            };
            div()
                .v_flex()
                .gap_0p5()
                .text_sm()
                .child(
                    div()
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .child(div().w(px(76.)).flex_shrink_0().child(capitalize(r.criterion)))
                        .child(div().w(px(36.)).flex_shrink_0().text_xs().text_color(muted).child(format!("{}%", r.weight)))
                        .child(div().w(px(32.)).flex_shrink_0().font_semibold().text_color(tone).child(if r.detail.known {
                            r.detail.score.to_string()
                        } else {
                            "\u{2013}".to_string()
                        }))
                        .child(self.score_bar(if r.detail.known { r.detail.score } else { 0 }, tone, cx)),
                )
                .child(div().text_xs().text_color(if r.detail.known { theme.foreground } else { muted }).child(reason))
        });
        Some(
            div()
                .id("queue-breakdown")
                .role(Role::Group)
                .aria_label("Score breakdown")
                .v_flex()
                .gap_1p5()
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .child(
                    div()
                        .h_flex()
                        .items_baseline()
                        .gap_2()
                        .child(div().text_xs().text_color(muted).child("Match"))
                        .child(div().text_lg().font_semibold().text_color(total_tone).child(score_label(item.lead.score)))
                        .child(div().text_xs().text_color(muted).child("of 100, weighted by your preferences")),
                )
                .children(rows),
        )
    }

    fn dismiss_form(&self, item: &QueueItem, form: &DismissForm, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let company = item.lead.company.clone();
        div()
            .id("queue-dismiss")
            .role(Role::Group)
            .aria_label("Dismiss this lead")
            .v_flex()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .child(div().font_semibold().child(format!("Dismiss {} \u{2014} {}", item.lead.company, item.lead.role)))
            .child(Input::new(&self.reason))
            .when(!company.trim().is_empty(), |d| {
                d.child(
                    Checkbox::new("dismiss-exclude")
                        .label(format!("Don't show {} again", company.trim()))
                        .checked(form.exclude_company)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            if let Some(form) = this.dismiss.as_mut() {
                                form.exclude_company = *checked;
                            }
                            cx.notify();
                        })),
                )
            })
            .when_some(form.error.clone(), |d, e| d.child(div().text_sm().text_color(theme.danger).child(e)))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("dismiss-confirm")
                            .outline()
                            .compact()
                            .label("Dismiss lead")
                            .loading(form.busy)
                            .disabled(form.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_dismiss(cx))),
                    )
                    .child(Button::new("dismiss-cancel").ghost().compact().label("Cancel").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.dismiss = None;
                            cx.notify();
                        },
                    ))),
            )
    }

    fn lead_panel(&self, item: &QueueItem, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let lead = &item.lead;
        let source = self
            .snap
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| Some(s.id) == lead.source_id))
            .map(source_name);
        let url = lead.url.clone();
        div()
            .id("queue-lead")
            .role(Role::Group)
            .aria_label(format!("{}: {}", lead.company, lead.role))
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_3()
            .overflow_y_scroll()
            .child(
                div()
                    .v_flex()
                    .gap_0p5()
                    .child(div().text_lg().font_semibold().child(format!("{} \u{2014} {}", lead.company, lead.role)))
                    .child(div().text_sm().text_color(muted).child(meta_line(&lead.location, source.as_deref())))
                    .when(is_web_link(&url), |d| {
                        let target = url.clone();
                        d.child(
                            div()
                                .id("lead-url")
                                .text_sm()
                                .text_color(theme.link)
                                .underline()
                                .cursor_pointer()
                                .truncate()
                                .child(url.clone())
                                .on_click(move |_, _, cx| cx.open_url(&target)),
                        )
                    }),
            )
            .children(self.breakdown(item, cx))
            .child(match &item.insight {
                Some(insight) => insight_brief(insight, cx).into_any_element(),
                None => div()
                    .text_sm()
                    .text_color(muted)
                    .child("No company brief for this lead.")
                    .into_any_element(),
            })
    }

    /// "Open posting" and "Dismiss" for the open lead, plain buttons.
    fn lead_actions(&self, item: &QueueItem, cx: &Context<Self>) -> Div {
        let (id, url) = (item.lead.id, item.lead.url.clone());
        div()
            .h_flex()
            .gap_2()
            .child(
                Button::new("queue-open-posting")
                    .outline()
                    .icon(IconName::ExternalLink)
                    .label("Open posting")
                    .disabled(!is_web_link(&url))
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(
                Button::new("queue-dismiss-open")
                    .outline()
                    .label("Dismiss")
                    .disabled(self.jobs.contains_key(&id))
                    .on_click(cx.listener(move |this, _, window, cx| this.open_dismiss(id, window, cx))),
            )
    }
}

impl Render for QueueView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shown again (or "Next in queue" sent us here): read the queue anew.
        let epoch = self.state.read(cx).queue_epoch;
        if self.epoch != Some(epoch) {
            self.epoch = Some(epoch);
            self.refresh(cx);
        }

        let theme = cx.theme();
        let (total, selected) = (self.items().len(), self.selected_id(cx));
        let shown = self.shown().len();
        let targets = self.targets(cx);
        let facts = QueueFacts {
            total,
            shown,
            targets: targets.len(),
            idle_targets: targets.iter().filter(|id| !self.jobs.contains_key(id)).count(),
            searching: self.searching.is_some(),
            sources: self.snap.as_ref().map_or(0, |s| s.sources.iter().filter(|s| s.enabled).count()),
        };
        let primary = queue_primary_for(&facts);
        let action = primary.action;
        let loaded = self.snap.is_some();

        let waiting: Vec<Stateful<Div>> = self
            .snap
            .as_ref()
            .map(|s| s.waiting.clone())
            .unwrap_or_default()
            .iter()
            .map(|w| self.waiting_banner(w, cx))
            .collect();
        let failures: Vec<Stateful<Div>> = self.failure_rows().into_iter().map(|r| self.failure_card(r, cx)).collect();
        let prepared: Vec<Stateful<Div>> = self.prepared.iter().enumerate().map(|(i, p)| self.prepared_row(i, p, cx)).collect();

        let open_item = selected.and_then(|id| self.items().iter().find(|i| i.lead.id == id)).cloned();
        let body = if total == 0 {
            div()
                .id("queue-empty")
                .role(Role::Paragraph)
                .aria_label("The queue is empty")
                .v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .gap_1()
                .text_center()
                .child(div().text_lg().font_semibold().child(if loaded { "The queue is empty" } else { "Loading the queue" }))
                .child(div().max_w(px(420.)).text_color(theme.muted_foreground).child(
                    "Jobs found from your sources appear here, best match first. Press Search now to look for new ones.",
                ))
                .into_any_element()
        } else {
            let panel = match &open_item {
                Some(item) => {
                    let form = self.dismiss.as_ref().filter(|f| f.lead_id == item.lead.id);
                    div()
                        .v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .gap_3()
                        .pl_2()
                        .when_some(form, |d, form| d.child(self.dismiss_form(item, form, cx)))
                        .child(self.lead_panel(item, cx))
                }
                None => div()
                    .flex_1()
                    .pl_2()
                    .text_color(theme.muted_foreground)
                    .child(if shown == 0 { "No lead matches the filters." } else { "Select a lead to see why it matches." }),
            };
            div().h_flex().flex_1().min_h_0().items_start().gap_2().child(self.list(cx)).child(panel).into_any_element()
        };

        div()
            .id("queue")
            .role(Role::Group)
            .aria_label("Queue")
            .v_flex()
            .size_full()
            .gap_3()
            .p_4()
            .child(self.header(total == 0, cx))
            .children(self.sources_line(cx))
            .children(self.note_line(cx))
            .when_some(self.load_error.clone(), |d, message| {
                d.child(div().text_sm().text_color(theme.danger).child(format!("The queue could not be read: {message}")))
            })
            .children(waiting)
            .children(failures)
            .children(prepared)
            .when(total > 0, |d| d.child(self.filter_bar(cx)))
            .child(body)
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(div().h_flex().gap_2().children(open_item.as_ref().map(|item| self.lead_actions(item, cx))))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_3()
                            .when_some(primary.hint.clone(), |d, hint| {
                                d.child(div().min_w_0().text_sm().text_color(theme.muted_foreground).child(hint))
                            })
                            .child(
                                Button::new("queue-primary")
                                    .primary()
                                    .label(action.label())
                                    .loading(primary.loading)
                                    .disabled(!primary.enabled)
                                    .on_click(cx.listener(move |this, _, _, cx| match action {
                                        QueueAction::PrepareCv => {
                                            let ids = this.targets(cx);
                                            this.prepare(ids, cx)
                                        }
                                        QueueAction::SearchNow => this.search_now(cx),
                                    })),
                            ),
                    ),
            )
    }
}

/// `Found 3 new leads, 1 source waiting for you` for the header and the note after a search.
fn summary_line(s: &jobpilot_core::discover::RunSummary) -> String {
    let plural = |n: u32, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = vec![format!("found {}", plural(s.saved, "new lead", "new leads"))];
    if s.waiting > 0 {
        parts.push(format!("{} waiting for you", plural(s.waiting, "source", "sources")));
    }
    if s.failed > 0 {
        parts.push(format!("{} failed", plural(s.failed, "source", "sources")));
    }
    let line = parts.join(", ");
    let mut chars = line.chars();
    chars.next().map_or(line.clone(), |c| c.to_uppercase().chain(chars).collect())
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

#[cfg(test)]
mod tests {
    use jobpilot_core::discover::RunSummary;

    use super::{capitalize, summary_line};

    #[test]
    fn the_search_summary_reads_as_a_sentence() {
        let run = RunSummary { saved: 3, ..RunSummary::default() };
        assert_eq!(summary_line(&run), "Found 3 new leads");
        let one = RunSummary { saved: 1, waiting: 1, failed: 2, ..RunSummary::default() };
        assert_eq!(summary_line(&one), "Found 1 new lead, 1 source waiting for you, 2 sources failed");
        assert_eq!(summary_line(&RunSummary::default()), "Found 0 new leads");
    }

    #[test]
    fn criteria_are_capitalised() {
        assert_eq!(capitalize("skills"), "Skills");
        assert_eq!(capitalize(""), "");
    }
}
