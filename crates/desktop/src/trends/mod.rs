//! Trends screen (P13b): where the market is going, from the sources you add.
//!
//! - "Collect & build digest" (the one primary button) collects the period's items, then asks the
//!   AI provider for a digest; every point links to the items it rests on. A source stuck at a
//!   login or captcha shows a banner with Continue (the paused run goes on) and "Build digest
//!   without it" (the digest alone, from what is stored).
//! - The latest digest with clickable citations, a skill-frequency chart, the history of reports.
//! - Sources (add, enable, remove) and the search API key that switches report search on.
//!
//! Views only: everything that touches the store, the keyring or the network runs through
//! `jobpilot_core::runtime::spawn` (the core's `trends` API). "Add to preferences" adds the skill
//! or domain to the saved preferences at once (Settings > Profile & preferences) and shows "Added".

mod demo;
mod report;
mod sources;

use std::collections::HashSet;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::Utc;
use jobpilot_core::graph::nodes::ChallengeKind;
use jobpilot_core::graph::{Outcome, Progress, ProgressEvent};
use jobpilot_core::health::{self, Problem};
use jobpilot_core::prefs::Preferences;
use jobpilot_core::store::{Report, Source};
use jobpilot_core::trends::{
    Digest, Period, SearchProvider, SourceKind, SourceStatus, TrendChallenge, TrendConfig, TrendRun,
};
use jobpilot_core::{runtime, Core};
use tokio::sync::broadcast::error::RecvError;

use crate::app::{AppState, Screen};
use crate::problem::{self, RetryFn};
use crate::settings::profile::{item_key, Field, Presence};
pub use demo::seed_demo;

/// Weekly digest or quarterly summary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Cadence {
    Weekly,
    Quarterly,
}

impl Cadence {
    /// The week or quarter that contains today.
    fn period(self) -> Period {
        let today = Utc::now().date_naive();
        match self {
            Cadence::Weekly => Period::week_of(today),
            Cadence::Quarterly => Period::quarter_of(today),
        }
    }
}

/// What a background job does, and what Retry repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Job {
    /// Collect, then build the digest.
    Collect(Period),
    /// The digest alone, from the stored items.
    Regenerate(Period),
    /// Carry on a paused or failed collection ("Continue", "Retry").
    Resume(Period),
    /// Carry on a failed digest-only run ("Retry").
    ResumeDigest(Period),
}

impl Job {
    async fn run(self, core: Arc<Core>) -> Result<Outcome<TrendRun>> {
        match self {
            Job::Collect(period) => core.collect_trends_and_digest(period).await,
            Job::Regenerate(period) => core.build_trends_digest(period).await,
            Job::Resume(period) => core.resume_trends_collection(period, TrendConfig::default()).await,
            Job::ResumeDigest(period) => core.resume_trends_digest(period).await,
        }
    }

    /// What to run when the user presses Retry after this job failed: carry on where it stopped.
    fn retry(self) -> Job {
        match self {
            Job::Collect(p) | Job::Resume(p) => Job::Resume(p),
            Job::Regenerate(p) | Job::ResumeDigest(p) => Job::ResumeDigest(p),
        }
    }
}

/// What the "Add to preferences" button of a topic shows and does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AddButton {
    /// Ready to add.
    Add,
    /// The save is running: pressing again does nothing.
    Adding,
    /// In the wanted lists (skills, domains).
    Added,
    /// In the avoid list: adding it would contradict the preferences.
    Avoided,
}

impl AddButton {
    fn new(presence: Presence, pending: bool) -> Self {
        match presence {
            Presence::Added => AddButton::Added,
            Presence::Avoided => AddButton::Avoided,
            Presence::Absent if pending => AddButton::Adding,
            Presence::Absent => AddButton::Add,
        }
    }

    fn enabled(self) -> bool {
        self == AddButton::Add
    }

    fn label(self) -> &'static str {
        if self == AddButton::Added {
            "Added"
        } else {
            "Add to preferences"
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            AddButton::Add | AddButton::Adding => "Add to preferences",
            AddButton::Added => "Added to your preferences",
            AddButton::Avoided => "In your avoid list",
        }
    }

    fn accessibility(self, topic: &str) -> String {
        match self {
            AddButton::Add | AddButton::Adding => format!("Add {topic} to preferences"),
            AddButton::Added => format!("{topic} is in your preferences"),
            AddButton::Avoided => format!("{topic} is in your avoid list"),
        }
    }
}

/// The line under the controls.
enum Notice {
    Info(String),
    Error { message: String, problem: Problem, job: Job },
}

/// What the screen shows, read in the background.
#[derive(Default)]
struct Snapshot {
    sources: Vec<Source>,
    reports: Vec<Report>,
    challenges: Vec<TrendChallenge>,
    keys: Vec<SearchProvider>,
    shown: Option<Digest>,
    /// The saved preferences: which skills and domains of the digest are added already.
    prefs: Preferences,
}

fn load_snapshot(core: &Core, want: Option<i64>) -> Result<Snapshot> {
    let shown = match want {
        Some(id) => core.trend_digest(id)?,
        None => None,
    };
    let shown = match shown {
        Some(found) => Some(found),
        None => core.latest_trend_digest()?,
    };
    Ok(Snapshot {
        sources: core.trend_sources()?,
        reports: core.trend_reports()?,
        challenges: core.trend_challenges()?,
        keys: core.saved_search_providers()?,
        shown,
        prefs: core.preferences()?,
    })
}

/// Input widgets; they need a `Window`, so they are created with the view.
struct Form {
    name: Entity<InputState>,
    url: Entity<InputState>,
    brave: Entity<InputState>,
    tavily: Entity<InputState>,
}

/// Colours copied out of the theme, so helpers can take the context mutably.
#[derive(Clone, Copy)]
struct Colors {
    muted: Hsla,
    border: Hsla,
    primary: Hsla,
    link: Hsla,
    danger: Hsla,
    warning: Hsla,
    accent: Hsla,
    track: Hsla,
}

fn colors(cx: &App) -> Colors {
    let t = cx.theme();
    Colors {
        muted: t.muted_foreground,
        border: t.border,
        primary: t.primary,
        link: t.link,
        danger: t.danger,
        warning: t.warning,
        accent: t.accent,
        track: t.muted,
    }
}

/// Market trends from the sources you add.
pub struct TrendsView {
    state: Entity<AppState>,
    snap: Snapshot,
    cadence: Cadence,
    /// The running job: the primary button and the actions that would start another are disabled.
    busy: Option<Job>,
    /// The graph node running now (from the core's progress events).
    step: Option<&'static str>,
    notice: Option<Notice>,
    /// How each source went in the last run.
    details: Vec<String>,
    /// The source kind whose address form is open.
    adding: Option<SourceKind>,
    /// Topics being added to the preferences right now (by field and lowercase text): a second
    /// click on the same topic waits for the first save.
    adding_topics: HashSet<(Field, String)>,
    /// Why the last change to the sources or the search key failed.
    sources_error: Option<String>,
    /// The report shown (`None`: the newest).
    selected: Option<i64>,
    /// The screen was shown at the last app-state change: reload when it becomes visible.
    visible: bool,
    form: Form,
}

impl TrendsView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |cx: &mut Context<Self>, window: &mut Window, placeholder: &str, masked: bool| {
            let placeholder = placeholder.to_string();
            cx.new(|cx| InputState::new(window, cx).masked(masked).placeholder(placeholder))
        };
        let form = Form {
            name: input(cx, window, "Name (optional)", false),
            url: input(cx, window, "https://example.com/feed.xml", false),
            brave: input(cx, window, "Brave Search API key", true),
            tavily: input(cx, window, "Tavily API key", true),
        };
        // Reload when the screen is shown: sources, reports and waiting runs may have changed.
        cx.observe(&state, |this, state, cx| {
            let visible = state.read(cx).screen == Screen::Trends;
            if visible && !this.visible {
                this.reload(cx);
            }
            this.visible = visible;
        })
        .detach();

        // Which step of a running job it is at.
        let mut events = state.read(cx).core.graphs.subscribe();
        cx.spawn(async move |this, cx| loop {
            match events.recv().await {
                Ok(progress) => {
                    if this.update(cx, |this, cx| this.on_progress(&progress, cx)).is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => {}
                Err(RecvError::Closed) => break,
            }
        })
        .detach();

        let mut view = Self {
            state,
            snap: Snapshot::default(),
            cadence: Cadence::Weekly,
            busy: None,
            step: None,
            notice: None,
            details: Vec::new(),
            adding: None,
            adding_topics: HashSet::new(),
            sources_error: None,
            selected: None,
            visible: false,
            form,
        };
        view.reload(cx);
        view
    }

    fn core(&self, cx: &App) -> Arc<Core> {
        self.state.read(cx).core.clone()
    }

    /// Read everything the screen shows, off the UI thread.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let want = self.selected;
        let rx = runtime::spawn(async move { load_snapshot(&core, want) });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                match result {
                    Ok(snap) => {
                        if this.selected.is_none() {
                            this.selected = snap.shown.as_ref().map(|d| d.report.id);
                        }
                        this.snap = snap;
                    }
                    Err(e) => this.fail(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn fail(&mut self, message: String) {
        // Not a job: there is nothing to resume, so no Retry (it would restart a collection).
        self.notice = Some(Notice::Info(format!("Could not read the saved data: {message}")));
    }

    fn on_progress(&mut self, progress: &Progress, cx: &mut Context<Self>) {
        if self.busy.is_none() || !progress.graph.starts_with("trends") {
            return;
        }
        if let ProgressEvent::NodeStarted { node } = &progress.event {
            self.step = Some(step_label(node));
            cx.notify();
        }
    }

    // ------------------------------------------------------------ jobs

    /// Start a collection or digest in the background; one at a time.
    fn start(&mut self, job: Job, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(job);
        self.step = Some("Starting");
        self.notice = None;
        self.details.clear();
        cx.notify();
        let rx = runtime::spawn(job.run(self.core(cx)));
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| this.finish(job, result, cx)).ok();
        })
        .detach();
    }

    fn finish(&mut self, job: Job, result: Result<Outcome<TrendRun>>, cx: &mut Context<Self>) {
        self.busy = None;
        self.step = None;
        match result {
            Ok(Outcome::Done(run)) => {
                if let Some(id) = run.digest.report_id {
                    self.selected = Some(id);
                }
                let (text, details) = summarize(&run);
                self.notice = Some(Notice::Info(text));
                self.details = details;
            }
            Ok(Outcome::Waiting(_)) => {
                self.notice = Some(Notice::Info(
                    "A source needs you before the digest can be built: see the banner below.".into(),
                ));
            }
            Ok(Outcome::Failed(failure)) => {
                self.notice = Some(Notice::Error { message: failure.message, problem: failure.problem, job: job.retry() });
            }
            Ok(Outcome::Cancelled) => self.notice = Some(Notice::Info("Cancelled.".into())),
            Err(e) => {
                let message = format!("{e:#}");
                let problem = health::classify(&message);
                self.notice = Some(Notice::Error { message, problem, job: job.retry() });
            }
        }
        // A missing AI provider: let the setup banner know.
        if matches!(&self.notice, Some(Notice::Error { problem: Problem::NoProvider, .. })) {
            self.state.update(cx, |s, cx| s.refresh_health(cx));
        }
        self.reload(cx);
        cx.notify();
    }

    /// A quick store or keyring call on the core runtime; `done` runs on the UI thread with its
    /// result.
    fn task<T, W, D>(&mut self, window: &mut Window, cx: &mut Context<Self>, work: W, done: D)
    where
        T: Send + 'static,
        W: Future<Output = Result<T>> + Send + 'static,
        D: FnOnce(&mut Self, Result<T>, &mut Window, &mut Context<Self>) + 'static,
    {
        let rx = runtime::spawn(work);
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update_in(cx, |this, window, cx| {
                done(this, result, window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, id: i64, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.reload(cx);
    }

    /// What the "Add to preferences" button of `topic` shows now.
    fn add_button(&self, field: Field, topic: &str) -> AddButton {
        AddButton::new(field.presence(&self.snap.prefs, topic), self.adding_topics.contains(&(field, item_key(topic))))
    }

    /// "Add to preferences": put `topic` into the saved skills or domains at once, off the UI
    /// thread. Nothing is cancelled and the screen does not change; the button shows a spinner
    /// while the save runs (more clicks on it do nothing), then reads "Added".
    fn add_to_preferences(&mut self, field: Field, topic: String, cx: &mut Context<Self>) {
        if !self.add_button(field, &topic).enabled() {
            return;
        }
        let key = (field, item_key(&topic));
        self.adding_topics.insert(key.clone());
        cx.notify();
        let core = self.core(cx);
        let shown = topic.clone();
        // `add` says whether it changed anything: a no-op saves nothing and keeps the undo point.
        let rx = runtime::spawn(async move { core.edit_preferences(move |prefs| field.add(prefs, &topic)).await });
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                this.adding_topics.remove(&key);
                match result {
                    Ok(prefs) => {
                        let added = field.presence(&prefs, &shown) == Presence::Added;
                        if added {
                            // The setup banner follows whether preferences were saved.
                            this.state.update(cx, |s, cx| {
                                if !s.prefs_saved {
                                    s.prefs_saved = true;
                                    cx.notify();
                                }
                            });
                        } else {
                            this.notice = Some(Notice::Info(field.why_not_added(&prefs, &shown)));
                        }
                        this.snap.prefs = prefs;
                    }
                    Err(e) => this.notice = Some(Notice::Info(format!("Could not add \"{shown}\" to your preferences: {e:#}"))),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ render

    fn render_controls(&self, c: &Colors, cx: &Context<Self>) -> Div {
        let running = self.busy.is_some();
        let period = self.cadence.period();
        let cadence = |id: &'static str, label: &'static str, value: Cadence| {
            Button::new(id).label(label).selected(self.cadence == value).disabled(running).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.cadence = value;
                    cx.notify();
                },
            ))
        };
        div()
            .v_flex()
            .gap_2()
            .child(
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(cadence("trends-weekly", "Weekly", Cadence::Weekly))
                    .child(cadence("trends-quarterly", "Quarterly", Cadence::Quarterly))
                    .child(
                        Button::new("trends-collect")
                            .primary()
                            .label("Collect & build digest")
                            .loading(running)
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| this.start(Job::Collect(period), cx))),
                    )
                    .child(
                        Button::new("trends-regenerate")
                            .outline()
                            .icon(IconName::RefreshCw)
                            .label("Regenerate from stored items")
                            .disabled(running)
                            .on_click(cx.listener(move |this, _, _, cx| this.start(Job::Regenerate(period), cx))),
                    )
                    .when_some(self.step, |d, step| {
                        d.child(Spinner::new().xsmall().color(c.primary))
                            .child(div().id("trends-step").role(Role::Paragraph).aria_label(step).text_sm().text_color(c.muted).child(step))
                    }),
            )
            .child(div().text_sm().text_color(c.muted).child(format!(
                "{}: collects from your sources, then your AI provider writes the digest. Every point links to the items it rests on.",
                period.label()
            )))
    }

    fn render_notice(&self, c: &Colors, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let (border, body) = match self.notice.as_ref()? {
            Notice::Info(text) => {
                let mut body = div().v_flex().gap_1().child(div().text_sm().child(text.clone()));
                for line in &self.details {
                    body = body.child(div().text_xs().text_color(c.muted).child(line.clone()));
                }
                (c.border, body)
            }
            Notice::Error { message, problem, job } => {
                let job = *job;
                let view = cx.entity();
                let retry: RetryFn = Rc::new(move |_, cx| view.update(cx, |this, cx| this.start(job, cx)));
                let mut body = div().v_flex().gap_1().child(div().text_sm().text_color(c.danger).min_w_0().child(message.clone()));
                if let Some(help) = problem::help("trends-error", *problem, &self.state, Some(retry), cx) {
                    body = body.child(help);
                }
                (c.danger, body)
            }
        };
        Some(div().id("trends-notice").role(Role::Group).aria_label("Result").p_3().rounded_md().border_1().border_color(border).child(body))
    }

    /// One banner per collection that waits at a login, captcha or block page.
    fn render_challenges(&self, c: &Colors, cx: &Context<Self>) -> Vec<Stateful<Div>> {
        let running = self.busy.is_some();
        self.snap
            .challenges
            .iter()
            .enumerate()
            .map(|(i, wait)| {
                let what = match wait.kind {
                    ChallengeKind::Login => "needs you to sign in",
                    ChallengeKind::Captcha => "shows a captcha",
                    ChallengeKind::Blocked => "blocks automated access",
                };
                let (period, url) = (wait.period, wait.url.clone());
                div()
                    .id(("trends-waiting", i))
                    .role(Role::Group)
                    .aria_label(format!("{} {what}", wait.source_name))
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .border_1()
                    .border_color(c.warning)
                    .child(div().text_sm().font_semibold().child(format!("{} {what} ({})", wait.source_name, period.label())))
                    .child(div().text_sm().text_color(c.muted).child(
                        "Open the page, deal with it yourself, then continue. The other sources are already stored: you can build the digest without this one.",
                    ))
                    .child(
                        div()
                            .h_flex()
                            .flex_wrap()
                            .gap_2()
                            .child(
                                Button::new(("trends-open-page", i))
                                    .outline()
                                    .compact()
                                    .icon(IconName::ExternalLink)
                                    .label("Open page")
                                    .when(report::is_web_url(&url), |b| b.on_click(move |_, _, cx| cx.open_url(&url))),
                            )
                            .child(
                                Button::new(("trends-continue", i))
                                    .outline()
                                    .compact()
                                    .label("Continue")
                                    .disabled(running)
                                    .on_click(cx.listener(move |this, _, _, cx| this.start(Job::Resume(period), cx))),
                            )
                            .child(
                                Button::new(("trends-without", i))
                                    .outline()
                                    .compact()
                                    .label("Build digest without it")
                                    .disabled(running)
                                    .on_click(cx.listener(move |this, _, _, cx| this.start(Job::Regenerate(period), cx))),
                            ),
                    )
            })
            .collect()
    }
}

impl Render for TrendsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = colors(cx);
        div()
            .id("trends")
            .role(Role::Group)
            .aria_label("Trends")
            .v_flex()
            .size_full()
            .gap_4()
            .p_6()
            .overflow_y_scroll()
            .child(div().text_2xl().font_semibold().child("Trends"))
            .child(self.render_controls(&c, cx))
            .children(self.render_notice(&c, cx))
            .children(self.render_challenges(&c, cx))
            .child(self.render_digest(&c, cx))
            .child(self.render_history(&c, cx))
            .child(self.render_sources(&c, cx))
            .child(self.render_search(&c, cx))
    }
}

/// Label of the graph node that just started.
fn step_label(node: &str) -> &'static str {
    match node {
        "plan" => "Planning the sources",
        "hackernews" | "yc" | "feeds" | "report_search" => "Reading the sources",
        "store_items" => "Storing the items",
        "wait_human" => "Checking for sources that need you",
        "extract" => "Reading the items with your AI provider",
        "count" => "Counting skills",
        "synthesize" => "Writing the digest",
        "enforce_citations" => "Checking the citations",
        "save_report" => "Saving the digest",
        _ => "Working",
    }
}

/// The headline of a finished run and one line per source (and per thing that went less well).
fn summarize(run: &TrendRun) -> (String, Vec<String>) {
    let d = &run.digest;
    let headline = if d.report_id.is_some() {
        format!("Digest ready: {} point(s) from {} item(s).", d.points.len(), d.items.len())
    } else {
        format!("Collected {} new item(s).", run.stored.new)
    };
    let mut lines: Vec<String> = run
        .outcomes
        .iter()
        .map(|o| {
            let name = SourceKind::parse(&o.kind).map_or("Report search", SourceKind::label);
            let status = match &o.status {
                SourceStatus::Ok => format!("{} new item(s)", o.items),
                SourceStatus::Skipped { reason } => format!("skipped ({reason})"),
                SourceStatus::TimedOut => "timed out, left out of this run".to_string(),
                SourceStatus::Failed { error } => format!("failed ({error})"),
                SourceStatus::NeedsHuman { .. } => "needs you".to_string(),
            };
            format!("{name}: {status}")
        })
        .collect();
    if d.failed_batches > 0 {
        lines.push(format!("{} batch(es) of items could not be read by the AI provider and were left out.", d.failed_batches));
    }
    if d.dropped > 0 {
        lines.push(format!("{} point(s) were left out: no valid source, or a quote that is not in the source.", d.dropped));
    }
    (headline, lines)
}

#[cfg(test)]
mod tests {
    use jobpilot_core::trends::{Period, SourceOutcome, SourceStatus, TrendRun};

    use super::{step_label, summarize, AddButton, Job};
    use crate::settings::profile::Presence;

    fn period() -> Period {
        Period::parse("2026-W46").unwrap()
    }

    #[test]
    fn the_add_button_follows_where_the_topic_stands() {
        assert_eq!(AddButton::new(Presence::Absent, false), AddButton::Add);
        assert_eq!(AddButton::new(Presence::Absent, true), AddButton::Adding, "a save is running: no second click");
        assert_eq!(AddButton::new(Presence::Added, false), AddButton::Added);
        assert_eq!(AddButton::new(Presence::Added, true), AddButton::Added, "saved: it reads Added even before the flag clears");
        assert_eq!(AddButton::new(Presence::Avoided, false), AddButton::Avoided);
        for state in [AddButton::Adding, AddButton::Added, AddButton::Avoided] {
            assert!(!state.enabled());
        }
        assert!(AddButton::Add.enabled());
        assert_eq!(AddButton::Added.label(), "Added");
        assert_eq!(AddButton::Avoided.label(), "Add to preferences");
        assert_eq!(AddButton::Avoided.tooltip(), "In your avoid list");
        assert!(AddButton::Avoided.accessibility("Go").contains("avoid"));
    }

    #[test]
    fn retry_carries_on_where_the_job_stopped() {
        let p = period();
        assert_eq!(Job::Collect(p).retry(), Job::Resume(p));
        assert_eq!(Job::Resume(p).retry(), Job::Resume(p));
        assert_eq!(Job::Regenerate(p).retry(), Job::ResumeDigest(p));
        assert_eq!(Job::ResumeDigest(p).retry(), Job::ResumeDigest(p));
    }

    #[test]
    fn a_finished_run_is_summarised_per_source() {
        let mut run = TrendRun::new(period());
        run.outcomes = vec![
            SourceOutcome { source_id: Some(1), kind: "hackernews".into(), status: SourceStatus::Ok, items: 4 },
            SourceOutcome { source_id: Some(2), kind: "yc".into(), status: SourceStatus::TimedOut, items: 0 },
            SourceOutcome { source_id: None, kind: "report_search".into(), status: SourceStatus::Skipped { reason: "no search API key saved".into() }, items: 0 },
        ];
        let (headline, lines) = summarize(&run);
        assert_eq!(headline, "Collected 0 new item(s).");
        assert_eq!(
            lines,
            ["Hacker News: 4 new item(s)", "YC companies: timed out, left out of this run", "Report search: skipped (no search API key saved)"]
        );

        run.digest.report_id = Some(3);
        run.digest.items = vec![1, 2, 3];
        run.digest.failed_batches = 1;
        run.digest.dropped = 2;
        let (headline, lines) = summarize(&run);
        assert_eq!(headline, "Digest ready: 0 point(s) from 3 item(s).");
        assert!(lines.iter().any(|l| l.starts_with("1 batch(es)")) && lines.iter().any(|l| l.starts_with("2 point(s) were left out")));
    }

    #[test]
    fn every_graph_node_has_a_label() {
        for node in [
            "plan", "hackernews", "yc", "feeds", "report_search", "store_items", "wait_human", "extract", "count", "synthesize",
            "enforce_citations", "save_report",
        ] {
            assert_ne!(step_label(node), "Working", "{node}");
        }
        assert_eq!(step_label("something else"), "Working");
    }
}
