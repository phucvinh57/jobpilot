//! Today screen (P12; the queue, formerly "Job list"): what needs the user, then found jobs best
//! match first, with the brief and the score breakdown, and the actions that turn a lead into an
//! application (Prepare CV), drop it (Dismiss) or look at it (Open posting). At the top, a link box
//! with "Add job" (and a file drop on the window) adds a job the user found themselves:
//! `Core::add_lead` reads it, scores it and briefs it, and it appears selected.
//! Views only: the data comes from `Core::queue_snapshot`, the work from `Core::add_lead` /
//! `prepare_cv` / `discover_now` / `dismiss_lead_with_reason`, all on the core runtime.
//!
//! The one primary button sits in the open lead's panel: "Prepare CV" while there are leads,
//! "Search now" (in the empty state) when the list is empty (`logic::queue_primary_for`).
//! Everything else is a plain button.

mod brief;
pub(crate) mod logic;

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui_kit::component::button::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Local, Utc};
use jobpilot_core::discover::promote::{parse_prepare_thread, Dismissed, PREPARE_GRAPH};
use jobpilot_core::discover::queue::{filter_items, QueueFilter, QueueItem, QueueSnapshot};
use jobpilot_core::discover::{WaitingSource, SOURCE_GRAPH};
use jobpilot_core::graph::{ProgressEvent, WaitReason};
use jobpilot_core::health;
use jobpilot_core::pipeline::JdInput;
use jobpilot_core::runtime;
use jobpilot_core::store::{Application, Lead, LeadState};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::mpsc::unbounded_channel;

use crate::app::jobs::parse_input;
use crate::app::{AppState, FADE};
use crate::brief::{company_block, interview_block};
use crate::problem::{self, RetryFn};
use crate::ui;
use brief::shorten;
use logic::{
    ago, dismissed_note, drop_unreported, is_web_link, meta_line, queue_behind, queue_primary_for, score_label, source_name,
    source_needs_attention, source_status, waiting_label, QueueAction, QueueFacts, QueuePrimary, Queued, ALREADY_READING,
};

/// The first look at the daily-run setting after the app starts, and the gap between looks after that.
const DAILY_FIRST_CHECK: Duration = Duration::from_secs(60);
const DAILY_CHECK_EVERY: Duration = Duration::from_secs(15 * 60);

/// Minimum scores the filter offers.
const MIN_SCORES: [f64; 3] = [50.0, 70.0, 85.0];

/// Width of the lead list (px); the open lead's panel takes the rest.
const LIST_WIDTH: f32 = 380.0;
/// The list shrinks to this (px) in a narrow window before the page scrolls sideways.
const LIST_MIN_WIDTH: f32 = 260.0;
/// The panel is never narrower than this (px).
const PANEL_MIN_WIDTH: f32 = 300.0;
/// The two panes never get shorter than this (px): a short window scrolls the page instead.
const PANES_MIN_HEIGHT: f32 = 440.0;
/// A "Needs you" banner is at least this wide (px), so two fit side by side on a wide window.
const NEEDS_MIN_WIDTH: f32 = 360.0;
/// Characters of a failure message the danger banner shows; the whole text goes under it.
const FAILURE_SUMMARY_CHARS: usize = 140;
/// Size of the open lead's title (px).
const PANEL_TITLE: f32 = 24.0;

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

/// A link, file or text that did not become a lead: a message under the link box.
type AddFailure = logic::AddFailure<JdInput>;

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

/// The Job list: found jobs, and the box that adds one.
pub struct QueueView {
    state: Entity<AppState>,
    /// The `AppState::queue_epoch` this view last loaded for.
    epoch: Option<u64>,
    snap: Option<QueueSnapshot>,
    /// The skill names of the master profile, to tell the skills a posting asks for that the
    /// profile has from those it lacks (empty when it cannot be read). Read again only when the
    /// screen is shown (see [`refresh_all`](Self::refresh_all)), not on every refresh.
    profile_skills: Vec<String>,
    load_error: Option<String>,
    filter: QueueFilter,
    location: Entity<InputState>,
    reason: Entity<InputState>,
    /// Paste a job link (or the posting's text) and press Enter.
    link: Entity<InputState>,
    /// The posting being read ("Reading the posting..."), if any.
    adding: Option<JdInput>,
    /// Links pasted and files dropped while one is being read: read one after another.
    waiting: VecDeque<JdInput>,
    /// Why the last inputs were not added, one per input, the newest first (each has its own Retry).
    add_failures: logic::AddFailures<JdInput>,
    /// A short remark under the link box ([`ALREADY_READING`]); the next paste or typing clears it.
    link_note: Option<&'static str>,
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
    /// Threads whose browser window is being opened.
    opening: BTreeSet<String>,
    /// Why "Open browser" failed, by thread (cleared by the next try, Continue or Skip).
    open_errors: HashMap<String, String>,
}

impl QueueView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // A file dropped on the window arrives through the shared state.
        cx.observe_in(&state, window, |this, state, window, cx| {
            if let Some(dropped) = state.update(cx, |s, _| s.dropped.take()) {
                this.on_dropped(dropped, window, cx);
            }
            cx.notify();
        })
        .detach();
        let link = cx.new(|cx| InputState::new(window, cx).placeholder("Paste a job link and press Enter, or drop a file"));
        cx.subscribe_in(&link, window, |this, input, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.submit_link(window, cx),
            InputEvent::Change => {
                // Typing again clears the "could not add" messages and the remark. Emptying the box
                // (after Enter, or its clear button) does not: it would hide what Enter just said.
                if !input.read(cx).value().is_empty() {
                    this.add_failures.clear();
                    this.link_note.take();
                }
                // "Add job" is enabled while there is something to add.
                cx.notify();
            }
            _ => {}
        })
        .detach();
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
            profile_skills: Vec::new(),
            load_error: None,
            filter: QueueFilter::default(),
            location,
            reason,
            link,
            adding: None,
            waiting: VecDeque::new(),
            add_failures: logic::AddFailures::default(),
            link_note: None,
            checked: BTreeSet::new(),
            jobs: HashMap::new(),
            failed: HashMap::new(),
            prepared: Vec::new(),
            searching: None,
            note: None,
            dismiss: None,
            continuing: BTreeSet::new(),
            opening: BTreeSet::new(),
            open_errors: HashMap::new(),
        };
        view.watch(cx);
        view.watch_daily(cx);
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

    /// The optional daily run: every few minutes ask the core whether one is due (setting on, a
    /// source, nothing running, a day since the last run) and, if so, start it like "Search now",
    /// so the progress line, the queue and the challenge hand-off work the same. This view lives
    /// as long as the app, so the timer does too.
    fn watch_daily(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let mut wait = DAILY_FIRST_CHECK;
            loop {
                cx.background_executor().timer(wait).await;
                wait = DAILY_CHECK_EVERY;
                let Ok(core) = this.update(cx, |this, cx| (this.searching.is_none()).then(|| this.core(cx))) else { break };
                let Some(core) = core else { continue };
                let rx = runtime::spawn({
                    let core = core.clone();
                    async move { core.discover_due(jobpilot_core::chrono::Utc::now()) }
                });
                match rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled"))) {
                    Ok(true) => {
                        // Skip if the user started a search meanwhile; record the attempt only for a
                        // daily run that really starts.
                        let Ok(started) = this.update(cx, |this, cx| {
                            let idle = this.searching.is_none();
                            if idle {
                                this.search_now(cx);
                            }
                            idle
                        }) else {
                            break;
                        };
                        if started {
                            let rx = runtime::spawn(async move { core.discover_daily_started(jobpilot_core::chrono::Utc::now()) });
                            if let Ok(Err(e)) = rx.await {
                                tracing::warn!("recording the daily search failed: {e:#}");
                            }
                        }
                    }
                    Ok(false) => {}
                    Err(e) => tracing::warn!("checking whether the daily search is due failed: {e:#}"),
                }
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

    /// Read the queue again (off the UI thread), after something changed it.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(false, cx);
    }

    /// [`refresh`](Self::refresh), and the profile's skills too: the profile is a file the user
    /// edits outside the app, so it is read when Today is shown (and the first time), not on every
    /// source progress event or click.
    fn refresh_all(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    fn load(&mut self, with_skills: bool, cx: &mut Context<Self>) {
        // The sidebar badge follows every change the list has, shown or not.
        self.state.update(cx, |s, cx| s.refresh_lead_count(cx));
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            let skills = with_skills.then(|| core.profile_skills());
            (core.queue_snapshot(), skills)
        });
        cx.spawn(async move |this, cx| {
            let (result, skills) = rx.await.unwrap_or_else(|_| (Err(anyhow!("the task was cancelled")), None));
            this.update(cx, |this, cx| this.on_snapshot(result, skills, cx)).ok();
        })
        .detach();
    }

    fn on_snapshot(&mut self, result: Result<QueueSnapshot>, skills: Option<Vec<String>>, cx: &mut Context<Self>) {
        match result {
            Ok(snap) => {
                self.load_error = None;
                if let Some(skills) = skills {
                    self.profile_skills = skills;
                }
                self.checked.retain(|id| snap.items.iter().any(|i| i.lead.id == *id));
                self.snap = Some(snap);
                self.normalize_selection(cx);
            }
            Err(e) => self.load_error = Some(format!("{e:#}")),
        }
        self.sync_needs_you(cx);
        cx.notify();
    }

    /// What the "NEEDS YOU" header counts: sources waiting at a login, failed CVs and CVs to read.
    /// The sidebar badge shows this same number (see [`sync_needs_you`](Self::sync_needs_you)).
    fn needs_you_count(&self) -> usize {
        self.snap.as_ref().map_or(0, |s| s.waiting.len()) + self.failure_rows().len() + self.prepared.len()
    }

    /// Tell the sidebar the number the header shows, so the two never differ. Call it after anything
    /// that changes the banners: the snapshot, a CV that was written or failed, a failure forgotten,
    /// a message closed.
    fn sync_needs_you(&self, cx: &mut Context<Self>) {
        let count = self.needs_you_count();
        self.state.update(cx, |s, cx| s.set_needs_you(count, cx));
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

    /// "Add job" or Enter in the link box: add what was typed or pasted, and empty the box once it
    /// is taken (a blank box, or too many inputs waiting, leaves it as it is).
    fn submit_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.link.read(cx).value();
        let Some(jd) = parse_input(&raw) else { return };
        if self.add_lead(jd, window, cx) {
            self.link.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    /// Read the posting behind `input`, score it and add it to the list as a lead (never an
    /// application: Prepare CV does that). While another posting is being read this one waits its
    /// turn (see [`logic::MAX_WAITING`]); `false` only when too many are waiting already.
    fn add_lead(&mut self, input: JdInput, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.link_note = None;
        if self.adding.is_some() {
            // The failure of this input goes only when it is really queued again: its Retry
            // cannot queue the same input twice, and a refusal leaves it where it is.
            let queued = queue_behind(self.adding.as_ref(), &mut self.waiting, &mut self.add_failures, input);
            if queued == Queued::Duplicate {
                self.link_note = Some(ALREADY_READING);
            }
            cx.notify();
            return queued != Queued::Full;
        }
        self.add_failures.clear_input(&input);
        self.start_add(input, window, cx);
        true
    }

    /// Start reading `input` (nothing else is being read).
    fn start_add(&mut self, input: JdInput, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = Some(input.clone());
        let core = self.core(cx);
        let run = input.clone();
        let rx = runtime::spawn(async move { core.add_lead(run).await });
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update_in(cx, |this, window, cx| this.on_lead_added(input, result, window, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn on_lead_added(&mut self, input: JdInput, result: Result<Lead>, window: &mut Window, cx: &mut Context<Self>) {
        self.adding = None;
        match result {
            Ok(lead) => {
                tracing::info!(lead = lead.id, "lead added from the link box");
                // Only this input's own failure: the others are still true.
                self.add_failures.clear_input(&input);
                match (lead.state, lead.app_id) {
                    // The same job was found before and its CV exists: that is where it is now.
                    (LeadState::Promoted, Some(app_id)) => self.state.update(cx, |s, cx| s.select(app_id, cx)),
                    (LeadState::Dismissed, _) => {
                        self.note = Some(Note { ok: false, text: "You dismissed this job before, so it is not in the list.".into() });
                    }
                    _ => {
                        // Show the new lead, whatever the filters were.
                        self.filter = QueueFilter::default();
                        self.location.update(cx, |input, cx| input.set_value("", window, cx));
                        self.set_selected(Some(lead.id), cx);
                    }
                }
                self.refresh(cx);
            }
            Err(e) => {
                let message = format!("{e:#}");
                self.state.update(cx, |s, cx| s.needs_recheck(&message, cx));
                self.add_failures.push(AddFailure { message, input: Some(input) });
            }
        }
        // The next one that was waiting, a failure above stays on screen meanwhile.
        if let Some(next) = self.waiting.pop_front() {
            self.start_add(next, window, cx);
        }
        cx.notify();
    }

    /// A file dropped on the window: a lead like a pasted link, or the reason it was refused.
    fn on_dropped(&mut self, dropped: Result<JdInput, String>, window: &mut Window, cx: &mut Context<Self>) {
        match dropped {
            Ok(input) => {
                self.add_lead(input, window, cx);
            }
            Err(message) => {
                self.add_failures.push(AddFailure { message, input: None });
                cx.notify();
            }
        }
    }

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
        self.sync_needs_you(cx);
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
                    this.sync_needs_you(cx);
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
        self.sync_needs_you(cx);
        // The application is in the sidebar now, whether or not the CV worked out.
        self.state.update(cx, |s, cx| s.reload(cx));
        self.refresh(cx);
    }

    fn forget_failure(&mut self, lead_id: i64, cx: &mut Context<Self>) {
        self.failed.remove(&lead_id);
        self.sync_needs_you(cx);
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
        let rx = runtime::spawn(async move { core.dismiss_lead_with_reason(id, &reason, exclude).await });
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

    /// "Open browser" for a source that waits at a login, captcha or block page: a visible browser on
    /// that page (in the source's bound session, if any). The user deals with the page; Continue goes on.
    fn open_browser(&mut self, thread: String, cx: &mut Context<Self>) {
        if !self.opening.insert(thread.clone()) {
            return;
        }
        self.open_errors.remove(&thread);
        let core = self.core(cx);
        let t = thread.clone();
        let rx = runtime::spawn(async move { core.discover_open_browser(&t).await });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                this.opening.remove(&thread);
                if let Err(e) = result {
                    this.open_errors.insert(thread, format!("{e:#}"));
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    /// Continue a source that waited for the user (a login, a captcha) or retry a failed one.
    fn continue_source(&mut self, thread: String, cx: &mut Context<Self>) {
        if !self.continuing.insert(thread.clone()) {
            return;
        }
        self.open_errors.remove(&thread);
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
        self.open_errors.remove(&thread);
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

    /// The title block: today's date, "Today", the last search (or the one running), and "Search now".
    fn header(&self, empty: bool, cx: &Context<Self>) -> Div {
        let p = ui::palette(cx);
        let last = self.snap.as_ref().and_then(|s| s.last_run.as_ref()).map(|run| {
            let when = DateTime::parse_from_rfc3339(&run.finished_at)
                .map(|t| ago(Utc::now(), t.with_timezone(&Utc)))
                .unwrap_or_else(|_| "earlier".into());
            format!("Last search {when}: {}", summary_line(run))
        });
        div()
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_end()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(ui::caption(Local::now().format("%A, %-d %B").to_string(), cx))
                            .child(ui::heading("Today", ui::Level::H1, cx)),
                    )
                    // When the list is empty "Search now" is the primary button, in the empty state.
                    .when(!empty, |d| {
                        d.child(
                            ui::secondary_button("queue-search", "Search now")
                                .icon(IconName::Search)
                                .loading(self.searching.is_some())
                                .disabled(self.searching.is_some())
                                .on_click(cx.listener(|this, _, _, cx| this.search_now(cx))),
                        )
                    }),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .text_sm()
                    .text_color(p.muted)
                    .when_some(self.searching.clone(), |d, label| d.child(Spinner::new().small().color(p.accent)).child(label))
                    .when(self.searching.is_none(), |d| {
                        d.child(div().min_w_0().child(last.unwrap_or_else(|| "No search yet.".into())))
                    }),
            )
    }

    /// The link box with "Add job", and under it what is happening to the last thing put in: a
    /// spinner while the posting is read, or why it failed with what to do about it.
    fn link_box(&self, cx: &Context<Self>) -> Div {
        let p = ui::palette(cx);
        let blank = self.link.read(cx).value().trim().is_empty();
        let failures: Vec<Stateful<Div>> = self
            .add_failures
            .iter()
            .enumerate()
            .map(|(n, f)| {
                let problem = health::classify(&f.message);
                let run = |input: JdInput| -> RetryFn {
                    let this = cx.entity();
                    std::rc::Rc::new(move |window, cx| {
                        let input = input.clone();
                        this.update(cx, |this, cx| {
                            this.add_lead(input, window, cx);
                        })
                    })
                };
                let help = problem::help(&format!("add-lead-error-{n}"), problem, &self.state, f.input.clone().map(run), cx);
                // A plain error has no advice and no Retry: offer to run it again.
                let again = f.input.clone().filter(|_| problem.advice().is_none()).map(|input| {
                    let run = run(input);
                    Button::new(("add-lead-again", n))
                        .outline()
                        .compact()
                        .label("Try again")
                        .on_click(move |_, window, cx| run(window, cx))
                });
                div()
                    .id(("add-lead-failure", n))
                    .role(Role::Group)
                    .aria_label(format!("The posting could not be added: {}", f.message))
                    .v_flex()
                    .gap_1()
                    .child(div().text_sm().text_color(p.danger_fg).child(f.message.clone()))
                    .children(help)
                    .children(again.map(|b| div().h_flex().child(b)))
            })
            .collect();
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.link)
                                .prefix(Icon::new(IconName::Plus).size_4().text_color(p.muted))
                                .cleanable(true),
                        ),
                    )
                    // A plain button: "Prepare CV" stays the only primary one.
                    .child(
                        ui::secondary_button("queue-add-job", "Add job")
                            .disabled(blank)
                            .on_click(cx.listener(|this, _, window, cx| this.submit_link(window, cx))),
                    ),
            )
            .when(self.adding.is_some(), |d| {
                d.child(
                    div()
                        .id("add-lead-progress")
                        .role(Role::Status)
                        .aria_label(match waiting_label(self.waiting.len()) {
                            Some(n) => format!("Reading the posting, {n}"),
                            None => "Reading the posting".to_string(),
                        })
                        .h_flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .rounded(px(ui::tokens::RADIUS_BUTTON))
                        .border_1()
                        .border_color(p.border)
                        .bg(p.surface)
                        .text_sm()
                        .text_color(p.muted)
                        .child(Spinner::new().small().color(p.accent))
                        .child("Reading the posting\u{2026}")
                        .children(waiting_label(self.waiting.len()).map(|n| div().child(format!("\u{b7} {n}")))),
                )
            })
            .children(self.link_note.map(|note| {
                div().id("add-lead-note").role(Role::Paragraph).aria_label(note).text_sm().text_color(p.muted).child(note)
            }))
            .children(failures)
    }

    fn sources_line(&self, cx: &App) -> Option<Stateful<Div>> {
        let snap = self.snap.as_ref().filter(|s| !s.sources.is_empty())?;
        let p = ui::palette(cx);
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
                .text_size(px(ui::tokens::TEXT_SMALL))
                .children(snap.sources.iter().map(|s| {
                    let color = if source_needs_attention(s) { p.warn_fg } else { p.subtle };
                    let text = format!("{}: {}{}", source_name(s), source_status(s, now), if s.enabled { "" } else { " (off)" });
                    div().id(("source", s.id as usize)).role(Role::ListItem).aria_label(text.clone()).text_color(color).child(text)
                })),
        )
    }

    fn note_line(&self, cx: &App) -> Option<Stateful<Div>> {
        let note = self.note.as_ref()?;
        let p = ui::palette(cx);
        Some(
            div()
                .id("queue-note")
                .role(Role::Paragraph)
                .aria_label(note.text.clone())
                .text_sm()
                .text_color(if note.ok { p.muted } else { p.danger_fg })
                .child(note.text.clone()),
        )
    }

    /// One source waiting for the user, as a "Needs you" banner: Open browser (a visible window on
    /// the page), Continue and Skip. When the window could not be opened, the reason and what to do
    /// about it (no Chrome: Settings) come under the banner.
    fn waiting_banner(&self, w: &WaitingSource, cx: &Context<Self>) -> Stateful<Div> {
        let p = ui::palette(cx);
        let (what, url) = match &w.reason {
            WaitReason::Login { url, .. } => ("needs you to sign in", Some(url.clone())),
            WaitReason::Captcha { url, .. } => ("shows a captcha", Some(url.clone())),
            WaitReason::Blocked { url, .. } => ("blocks automated access", Some(url.clone())),
            _ => ("is waiting for you", None),
        };
        let (thread, skip_thread, open_thread) = (w.thread.clone(), w.thread.clone(), w.thread.clone());
        let opening = self.opening.contains(&w.thread);
        let busy = self.continuing.contains(&w.thread);
        let id = w.source_id;
        let open_error = self.open_errors.get(&w.thread).map(|message| {
            let problem = health::classify(message);
            let retry: RetryFn = {
                let (this, thread) = (cx.entity(), w.thread.clone());
                std::rc::Rc::new(move |_, cx| this.update(cx, |this, cx| this.open_browser(thread.clone(), cx)))
            };
            let help = problem::help(&format!("waiting-open-error-{id}"), problem, &self.state, Some(retry), cx);
            div()
                .id(("waiting-open-error", id as usize))
                .role(Role::Paragraph)
                .aria_label(format!("The browser could not be opened: {message}"))
                .v_flex()
                .gap_1()
                .child(div().text_sm().text_color(p.danger_fg).child(format!("The browser could not be opened: {message}")))
                .children(help)
        });
        let title = format!("{} {what}", w.source_name);
        let body = match url.filter(|u| is_web_link(u)) {
            Some(url) => shorten(&url, 90),
            None => "Deal with the page in the browser, then press Continue.".to_string(),
        };
        let actions = div()
            .h_flex()
            .flex_shrink_0()
            .gap_2()
            .child(
                ui::secondary_button(SharedString::from(format!("waiting-open-{id}")), "Open browser")
                    .icon(IconName::ExternalLink)
                    .loading(opening)
                    .disabled(opening || busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.open_browser(open_thread.clone(), cx))),
            )
            .child(
                ui::secondary_button(SharedString::from(format!("waiting-continue-{id}")), "Continue")
                    .loading(busy)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.continue_source(thread.clone(), cx))),
            )
            .child(
                ui::ghost_button(SharedString::from(format!("waiting-skip-{id}")), "Skip")
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| this.skip_source(skip_thread.clone(), cx))),
            );
        div()
            .id(("waiting", id as usize))
            .role(Role::Group)
            .aria_label(format!("{} {what}", w.source_name))
            .v_flex()
            .flex_1()
            .min_w(px(NEEDS_MIN_WIDTH))
            .gap_1()
            .child(ui::banner(ui::Tone::Warn, &title, &body, actions, cx))
            .children(open_error)
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

    /// A CV that could not be written, as a danger banner: the first line of the message (the whole
    /// text under it when there is more), what to do about it, Retry.
    fn failure_card(&self, row: FailureRow, cx: &Context<Self>) -> Stateful<Div> {
        let p = ui::palette(cx);
        let (id, app_id) = (row.lead_id, row.app_id);
        let problem = health::classify(&row.message);
        let retry: RetryFn = {
            let this = cx.entity();
            std::rc::Rc::new(move |_, cx| this.update(cx, |this, cx| this.prepare(vec![id], cx)))
        };
        let help = problem::help(&format!("prepare-error-{id}"), problem, &self.state, Some(retry), cx);
        let title = format!("The CV for {} could not be written", row.title);
        let first_line = row.message.lines().next().unwrap_or_default().trim();
        let body = shorten(first_line, FAILURE_SUMMARY_CHARS);
        let has_more = row.message.lines().count() > 1 || first_line.chars().count() > FAILURE_SUMMARY_CHARS;
        let actions = div()
            .h_flex()
            .flex_shrink_0()
            .gap_2()
            .when(!problem.retryable(), |d| {
                d.child(
                    ui::secondary_button(SharedString::from(format!("prepare-retry-{id}")), "Try again")
                        .on_click(cx.listener(move |this, _, _, cx| this.prepare(vec![id], cx))),
                )
            })
            .when_some(app_id, |d, app_id| {
                d.child(
                    ui::secondary_button(SharedString::from(format!("prepare-open-{id}")), "Open application")
                        .on_click(cx.listener(move |this, _, _, cx| this.state.update(cx, |s, cx| s.select(app_id, cx)))),
                )
            })
            .child(
                ui::ghost_button(SharedString::from(format!("prepare-forget-{id}")), "Forget")
                    .on_click(cx.listener(move |this, _, _, cx| this.forget_failure(id, cx))),
            );
        div()
            .id(("failure", id as usize))
            .role(Role::Group)
            .aria_label(title.clone())
            .v_flex()
            .flex_1()
            .min_w(px(NEEDS_MIN_WIDTH))
            .gap_1()
            .child(ui::banner(ui::Tone::Danger, &title, &body, actions, cx))
            .when(has_more, |d| {
                d.child(
                    div()
                        .id(("failure-text", id as usize))
                        .max_h(px(72.))
                        .overflow_y_scroll()
                        .font_family("monospace")
                        .text_size(px(ui::tokens::TEXT_SMALL))
                        .text_color(p.danger_fg)
                        .child(row.message.clone()),
                )
            })
            .children(help)
    }

    /// A CV that was written, as a banner that points at the application: read it, then approve it.
    fn prepared_row(&self, index: usize, prepared: &Prepared, cx: &Context<Self>) -> Stateful<Div> {
        let app_id = prepared.app_id;
        let title = format!("CV ready for {}", prepared.label);
        let actions = div()
            .h_flex()
            .flex_shrink_0()
            .gap_2()
            .child(
                ui::secondary_button(SharedString::from(format!("prepared-open-{index}")), "Open application").on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.state.update(cx, |s, cx| s.select(app_id, cx));
                    }),
                ),
            )
            .child(
                Button::new(SharedString::from(format!("prepared-close-{index}")))
                    .ghost()
                    .icon(IconName::Close)
                    .accessibility_label("Dismiss this message")
                    .size(px(ui::tokens::BUTTON_HEIGHT))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if index < this.prepared.len() {
                            this.prepared.remove(index);
                        }
                        this.sync_needs_you(cx);
                        cx.notify();
                    })),
            );
        div()
            .id(("prepared", index))
            .role(Role::Group)
            .aria_label(title.clone())
            .flex_1()
            .min_w(px(NEEDS_MIN_WIDTH))
            .child(ui::banner(ui::Tone::Ok, &title, "Read it, then approve it.", actions, cx))
    }

    /// "Needs you": sources waiting at a login, failed CVs and CVs to read. `None` when there is nothing.
    fn needs_you(&self, banners: Vec<Stateful<Div>>, cx: &App) -> Option<Stateful<Div>> {
        if banners.is_empty() {
            return None;
        }
        Some(
            div()
                .id("queue-needs-you")
                .role(Role::Group)
                .aria_label("Needs you")
                .v_flex()
                .gap_2p5()
                .child(
                    div()
                        .text_size(px(ui::tokens::TEXT_SMALL))
                        .font_semibold()
                        .text_color(ui::palette(cx).warn_fg)
                        .child(format!("NEEDS YOU \u{b7} {}", self.needs_you_count())),
                )
                .child(div().h_flex().flex_wrap().gap_3().children(banners)),
        )
    }

    fn filter_bar(&self, cx: &Context<Self>) -> Stateful<Div> {
        let sources: Vec<(i64, String)> =
            self.snap.iter().flat_map(|s| s.sources.iter()).map(|s| (s.id, source_name(s))).collect();
        let caption = |text: &'static str| ui::caption(text, cx);
        let (shown, total) = (self.shown().len(), self.items().len());
        div()
            .id("queue-filters")
            .role(Role::Group)
            .aria_label("Filters")
            .h_flex()
            .flex_wrap()
            .items_center()
            .gap_x_4()
            .gap_y_2()
            .when(sources.len() > 1, |d| {
                d.child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1p5()
                        .child(caption("Source"))
                        .child(ui::filter_chip("flt-src-all", "All", self.filter.source_id.is_none()).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.filter.source_id = None;
                                this.normalize_selection(cx);
                                cx.notify();
                            },
                        )))
                        .children(sources.iter().map(|(id, name)| {
                            let id = *id;
                            ui::filter_chip(
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
                    .flex_wrap()
                    .items_center()
                    .gap_1p5()
                    .child(caption("Min score"))
                    .child(ui::filter_chip("flt-score-any", "Any", self.filter.min_score.is_none()).on_click(cx.listener(
                        |this, _, _, cx| {
                            this.filter.min_score = None;
                            this.normalize_selection(cx);
                            cx.notify();
                        },
                    )))
                    .children(MIN_SCORES.iter().map(|min| {
                        let min = *min;
                        ui::filter_chip(
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
            .child(ui::caption(format!("{shown} of {total} shown"), cx))
    }

    /// One lead as a card: a tick box (only on hover, or once any box is ticked), the score badge,
    /// the role, where it is, why it matches, and which source found it and when.
    fn lead_row(&self, item: &QueueItem, selected: bool, cx: &Context<Self>) -> Stateful<Div> {
        let p = ui::palette(cx);
        let lead = &item.lead;
        let id = lead.id;
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
        let group = SharedString::from(format!("lead-card-{id}"));
        let any_checked = !self.checked.is_empty();
        let small = px(ui::tokens::TEXT_SMALL);
        div()
            .id(("lead-card", id as usize))
            .group(group.clone())
            .h_flex()
            .items_center()
            .rounded(px(ui::tokens::RADIUS_CARD - 2.0))
            .border_1()
            .border_color(if selected { p.accent } else { p.border })
            .bg(if selected { p.surface } else { transparent_black() })
            .hover(move |d| d.bg(p.surface))
            .child(
                div()
                    .pl_3()
                    .when(!any_checked, |d| d.opacity(0.).group_hover(group, |s| s.opacity(1.)))
                    .child(
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
                    .h_flex()
                    .items_center()
                    .flex_1()
                    .min_w_0()
                    .gap_3()
                    .py_3()
                    .pl_2()
                    .pr_3()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_selected(Some(id), cx);
                        cx.notify();
                    }))
                    .child(ui::score_badge(lead.score, cx))
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().truncate().font_semibold().text_color(p.ink).child(lead.role.clone()))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(ui::tokens::TEXT_BODY - 1.0))
                                    .text_color(p.muted)
                                    .child(format!("{} \u{b7} {}", lead.company, meta_line(&lead.location, None))),
                            )
                            .when(!reasons.is_empty(), |d| d.child(div().truncate().text_size(small).text_color(p.subtle).child(reasons)))
                            .when_some(job, |d, job| {
                                d.child(
                                    div()
                                        .h_flex()
                                        .items_center()
                                        .gap_1()
                                        .text_size(small)
                                        .text_color(p.accent)
                                        .child(Spinner::new().xsmall().color(p.accent))
                                        .child(job.label().to_string()),
                                )
                            }),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_shrink_0()
                            .items_end()
                            .text_size(small)
                            .text_color(p.subtle)
                            .children(source)
                            .child(ago(Utc::now(), lead.found_at)),
                    ),
            )
    }

    /// The left pane: "New leads" and the cards, which scroll on their own.
    fn list(&self, total: usize, cx: &Context<Self>) -> Div {
        let selected = self.selected_id(cx);
        let rows: Vec<Stateful<Div>> =
            self.shown().into_iter().map(|item| self.lead_row(item, selected == Some(item.lead.id), cx)).collect();
        div()
            .v_flex()
            .w(px(LIST_WIDTH))
            .min_w(px(LIST_MIN_WIDTH))
            .h_full()
            .gap_2p5()
            .child(
                div()
                    .h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(ui::heading("New leads", ui::Level::H2, cx))
                    .child(div().text_color(ui::palette(cx).muted).child(format!("\u{b7} {total}"))),
            )
            .child(
                div()
                    .id("queue-list")
                    .role(Role::List)
                    .aria_label("Leads")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_2()
                    .overflow_y_scroll()
                    .children(rows),
            )
    }

    fn dismiss_form(&self, item: &QueueItem, form: &DismissForm, cx: &Context<Self>) -> Stateful<Div> {
        let p = ui::palette(cx);
        let company = item.lead.company.clone();
        ui::card(cx)
            .id("queue-dismiss")
            .role(Role::Group)
            .aria_label("Dismiss this lead")
            .bg(p.surface_alt)
            .gap_2p5()
            .child(div().font_semibold().text_color(p.ink).child(format!("Dismiss {} \u{2014} {}", item.lead.company, item.lead.role)))
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
            .when_some(form.error.clone(), |d, e| d.child(div().text_sm().text_color(p.danger_fg).child(e)))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        ui::secondary_button("dismiss-confirm", "Dismiss lead")
                            .loading(form.busy)
                            .disabled(form.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.confirm_dismiss(cx))),
                    )
                    .child(ui::ghost_button("dismiss-cancel", "Cancel").on_click(cx.listener(|this, _, _, cx| {
                        this.dismiss = None;
                        cx.notify();
                    }))),
            )
    }

    /// The one primary button of the screen (`queue_primary_for`).
    fn primary_button(&self, primary: &QueuePrimary, cx: &Context<Self>) -> Button {
        let action = primary.action;
        ui::primary_button("queue-primary", action.label())
            .loading(primary.loading)
            .disabled(!primary.enabled)
            .on_click(cx.listener(move |this, _, _, cx| match action {
                QueueAction::PrepareCv => {
                    let ids = this.targets(cx);
                    this.prepare(ids, cx)
                }
                QueueAction::SearchNow => this.search_now(cx),
            }))
    }

    /// The primary button with its hint under it and, for the open lead, the plain "Open posting"
    /// and "Dismiss" buttons beside it.
    fn actions(&self, open: Option<&QueueItem>, primary: &QueuePrimary, cx: &Context<Self>) -> Div {
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(self.primary_button(primary, cx))
                    .when_some(open, |d, item| {
                        let (id, url) = (item.lead.id, item.lead.url.clone());
                        d.child(
                            ui::secondary_button("queue-open-posting", "Open posting")
                                .icon(IconName::ExternalLink)
                                .disabled(!is_web_link(&url))
                                .on_click(move |_, _, cx| cx.open_url(&url)),
                        )
                        .child(
                            ui::secondary_button("queue-dismiss-open", "Dismiss")
                                .disabled(self.jobs.contains_key(&id))
                                .on_click(cx.listener(move |this, _, window, cx| this.open_dismiss(id, window, cx))),
                        )
                    }),
            )
            .when_some(primary.hint.clone(), |d, hint| d.child(ui::caption(hint, cx)))
    }

    /// The right pane for the open lead: title, score, the actions, why it fits, its skills, the
    /// company and the interview process. It scrolls on its own.
    fn lead_panel(&self, item: &QueueItem, primary: &QueuePrimary, cx: &Context<Self>) -> Stateful<Div> {
        let p = ui::palette(cx);
        let lead = &item.lead;
        let source = self
            .snap
            .as_ref()
            .and_then(|s| s.sources.iter().find(|s| Some(s.id) == lead.source_id))
            .map(source_name);
        let url = lead.url.clone();
        let score = p.tone(ui::score_tone_of(lead.score));
        let form = self.dismiss.as_ref().filter(|f| f.lead_id == lead.id);

        let header = div()
            .h_flex()
            .flex_wrap()
            .items_start()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(240.))
                    .gap_1()
                    .child(div().text_sm().text_color(p.muted).child(format!("{} \u{b7} {}", lead.company, meta_line(&lead.location, source.as_deref()))))
                    .child(
                        div()
                            .text_size(px(PANEL_TITLE))
                            .line_height(relative(1.25))
                            .font_family(ui::tokens::FONT_HEADING)
                            .font_semibold()
                            .text_color(p.ink)
                            .child(lead.role.clone()),
                    )
                    .when(is_web_link(&url), |d| {
                        let target = url.clone();
                        d.child(
                            div()
                                .id("lead-url")
                                .role(Role::Link)
                                .aria_label(format!("Posting: {url}"))
                                .text_sm()
                                .text_color(p.accent)
                                .underline()
                                .cursor_pointer()
                                .truncate()
                                .child(url.clone())
                                .on_click(move |_, _, cx| cx.open_url(&target)),
                        )
                    }),
            )
            .child(
                div()
                    .v_flex()
                    .items_end()
                    .child(
                        div()
                            .text_size(px(36.))
                            .line_height(relative(1.))
                            .font_family(ui::tokens::FONT_HEADING)
                            .font_semibold()
                            .text_color(score.fg)
                            .child(score_label(lead.score)),
                    )
                    .child(ui::caption("fit score", cx)),
            );

        let breakdown = brief::breakdown(item, cx);
        let skills = item.insight.as_ref().and_then(|i| brief::skills_block(i, &self.profile_skills, cx));
        let fit = (breakdown.is_some() || skills.is_some()).then(|| {
            div()
                .h_flex()
                .flex_wrap()
                .items_start()
                .gap_6()
                .children(breakdown.map(|b| b.flex_1().min_w(px(220.))))
                .children(skills.map(|s| s.flex_1().min_w(px(220.))))
        });
        let about = div().border_t_1().border_color(p.border).pt_5();
        let about = match &item.insight {
            Some(insight) => about
                .h_flex()
                .flex_wrap()
                .items_start()
                .gap_6()
                .child(company_block(insight, cx).flex_1().min_w(px(220.)))
                .child(interview_block(insight, cx).flex_1().min_w(px(220.))),
            None => about.child(div().text_sm().text_color(p.muted).child("No company brief for this lead.")),
        };

        div()
            .id("queue-lead")
            .role(Role::Group)
            .aria_label(format!("{}: {}", lead.company, lead.role))
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .overflow_y_scroll()
            .child(
                ui::card(cx)
                    .p_6()
                    .gap_5()
                    .rounded(px(ui::tokens::RADIUS_CARD + 2.0))
                    .child(header)
                    .child(self.actions(Some(item), primary, cx))
                    .when_some(form, |d, form| d.child(self.dismiss_form(item, form, cx)))
                    .children(fit)
                    .child(about),
            )
    }
}

impl Render for QueueView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shown again (or "Next job" sent us here): read the list anew.
        let epoch = self.state.read(cx).queue_epoch;
        if self.epoch != Some(epoch) {
            self.epoch = Some(epoch);
            self.refresh_all(cx);
        }

        let p = ui::palette(cx);
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
        let loaded = self.snap.is_some();

        let waiting = self.snap.as_ref().map(|s| s.waiting.clone()).unwrap_or_default();
        let banners: Vec<Stateful<Div>> = waiting
            .iter()
            .map(|w| self.waiting_banner(w, cx))
            .chain(self.failure_rows().into_iter().map(|r| self.failure_card(r, cx)))
            .chain(self.prepared.iter().enumerate().map(|(i, prepared)| self.prepared_row(i, prepared, cx)))
            .collect();

        let open_item = selected.and_then(|id| self.items().iter().find(|i| i.lead.id == id)).cloned();
        let body = if total == 0 {
            div()
                .id("queue-empty")
                .role(Role::Group)
                .aria_label("No jobs yet")
                .v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .child(
                    ui::empty_state(
                        IconName::Inbox,
                        if loaded { "No jobs yet" } else { "Loading the job list" },
                        "Paste a job link in the box above, or press Search now to look in your sources. Jobs appear here, best match first.",
                        Some(self.actions(None, &primary, cx).items_center().into_any_element()),
                        cx,
                    )
                    .max_w(px(520.)),
                )
                .into_any_element()
        } else {
            let panel = match &open_item {
                Some(item) => div()
                    .v_flex()
                    .flex_1()
                    .min_w(px(PANEL_MIN_WIDTH))
                    .h_full()
                    .child(self.lead_panel(item, &primary, cx))
                    // The panel fades and slides in whenever another lead is opened.
                    .with_animation(
                        ElementId::NamedInteger("lead-panel".into(), item.lead.id as u64),
                        Animation::new(FADE).with_easing(ease_in_out),
                        |this, delta| this.opacity(delta).relative().left(px(14. * (1. - delta))),
                    )
                    .into_any_element(),
                None => div()
                    .flex_1()
                    .min_w(px(PANEL_MIN_WIDTH))
                    .child(
                        ui::empty_state(
                            IconName::Search,
                            if shown == 0 { "No lead matches the filters" } else { "Select a lead" },
                            if shown == 0 { "Loosen a filter to see more jobs." } else { "Pick a job on the left to see why it matches." },
                            Some(self.actions(None, &primary, cx).items_center().into_any_element()),
                            cx,
                        ),
                    )
                    .into_any_element(),
            };
            div()
                .h_flex()
                .flex_1()
                .min_h(px(PANES_MIN_HEIGHT))
                .items_start()
                .gap_5()
                .child(self.list(total, cx))
                .child(panel)
                .into_any_element()
        };

        div()
            .id("queue")
            .role(Role::Group)
            .aria_label("Today")
            .v_flex()
            .size_full()
            .gap_5()
            .px_8()
            .py_7()
            // A short window scrolls the page; the panes keep their own scroll inside it.
            .overflow_y_scroll()
            .child(
                div()
                    .v_flex()
                    .gap_3()
                    .child(self.header(total == 0, cx))
                    .child(self.link_box(cx))
                    .children(self.sources_line(cx))
                    .children(self.note_line(cx))
                    .when_some(self.load_error.clone(), |d, message| {
                        d.child(div().text_sm().text_color(p.danger_fg).child(format!("The job list could not be read: {message}")))
                    }),
            )
            .children(self.needs_you(banners, cx))
            .when(total > 0, |d| d.child(self.filter_bar(cx)))
            .child(body)
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

#[cfg(test)]
mod tests {
    use jobpilot_core::discover::RunSummary;

    use super::summary_line;

    #[test]
    fn the_search_summary_reads_as_a_sentence() {
        let run = RunSummary { saved: 3, ..RunSummary::default() };
        assert_eq!(summary_line(&run), "Found 3 new leads");
        let one = RunSummary { saved: 1, waiting: 1, failed: 2, ..RunSummary::default() };
        assert_eq!(summary_line(&one), "Found 1 new lead, 1 source waiting for you, 2 sources failed");
        assert_eq!(summary_line(&RunSummary::default()), "Found 0 new leads");
    }
}
