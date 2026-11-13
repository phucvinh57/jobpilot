//! Apply tab: review the filled application form, then submit (P5).
//!
//! States per application: idle (what would happen) -> preparing (spinner + steps) -> review (editable
//! label/value rows, attached PDF, screenshot) -> submitting -> done, or handoff (failure / CAPTCHA /
//! unsupported ATS with "Open visible browser"). All browser work lives in `jobpilot_core::apply` and
//! runs on the core runtime; this view only starts it and shows the result. Nothing is submitted
//! except by the explicit "Submit application" click (or the detail pane's primary Submit button).

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui_kit::component::button::*;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::apply::{
    self, ApplyInfo, ApplyOptions, Edit, FieldKind, Handoff, Prepared, Review, Row, SessionState, Source, Submitted,
};
use jobpilot_core::browser::SessionStatus;
use jobpilot_core::runtime;
use jobpilot_core::store::{Application, Status};

use crate::app::AppState;

use super::PrimaryAction;

/// Choice fields with at most this many options are shown as buttons; longer lists are typed.
const MAX_CHIPS: usize = 8;

#[derive(Clone)]
enum Phase {
    Idle,
    Preparing,
    Review,
    Submitting,
    Done(Option<PathBuf>),
    Handoff { handoff: Handoff, headed: bool },
}

/// How one review row is edited.
enum FieldInput {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
    /// Buttons / checkbox / read-only: the value lives here.
    Value(String),
}

struct AppView {
    phase: Phase,
    review: Option<Review>,
    inputs: HashMap<String, FieldInput>,
    /// Bumped when `review` changes, so inputs are rebuilt once per review.
    generation: usize,
    built: usize,
    remember: HashSet<String>,
    error: Option<String>,
    progress: Arc<Mutex<Vec<String>>>,
    busy: Option<&'static str>,
}

impl AppView {
    fn new() -> Self {
        Self {
            phase: Phase::Idle,
            review: None,
            inputs: HashMap::new(),
            generation: 0,
            built: 0,
            remember: HashSet::new(),
            error: None,
            progress: Arc::default(),
            busy: None,
        }
    }

    fn set_review(&mut self, review: Review) {
        self.review = Some(review);
        self.generation += 1;
        self.phase = Phase::Review;
    }
}

pub struct ApplyTab {
    state: Entity<AppState>,
    views: HashMap<i64, AppView>,
    dry_run: bool,
    /// Application the tab was last rendered for (to pick up a run that is already live).
    current: Option<i64>,
    url_input: Option<Entity<InputState>>,
}

impl ApplyTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state, views: HashMap::new(), dry_run: false, current: None, url_input: None }
    }

    /// Called by the detail pane's primary button for `ReviewAndSubmit` and `Submit`
    /// (the pane has already switched to this tab).
    pub fn on_primary(&mut self, action: PrimaryAction, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(app_id) = self.state.read(cx).selected else { return };
        let phase = self.view(app_id).phase.clone();
        match (action, phase) {
            (_, Phase::Preparing | Phase::Submitting) => {}
            // Status says "waiting for confirmation" and the form is open: this click is the confirmation.
            (PrimaryAction::Submit, Phase::Review) => self.confirm(app_id, cx),
            // Nothing is open (the app was restarted, or this is a retry): open and fill the form again.
            // Never submits.
            _ => self.start_prepare(app_id, cx),
        }
    }

    fn view(&mut self, app_id: i64) -> &mut AppView {
        self.views.entry(app_id).or_insert_with(AppView::new)
    }

    fn core(&self, cx: &App) -> Arc<jobpilot_core::Core> {
        self.state.read(cx).core.clone()
    }

    fn reload_state(&self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.reload(cx));
    }

    /// Run `fut` on the core runtime; `on_done` runs on the UI thread with its result.
    fn run<T: Send + 'static>(
        &mut self,
        fut: impl Future<Output = anyhow::Result<T>> + Send + 'static,
        on_done: impl FnOnce(&mut Self, anyhow::Result<T>, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) {
        let rx = runtime::spawn(fut);
        cx.spawn(async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow::anyhow!("the task was cancelled")));
            this.update(cx, |this, cx| {
                on_done(this, result, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Repaint while a background step runs so the progress lines move.
    fn watch(&mut self, app_id: i64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| loop {
            cx.background_executor().timer(Duration::from_millis(350)).await;
            match this.update(cx, |this, cx| {
                cx.notify();
                matches!(this.view(app_id).phase, Phase::Preparing | Phase::Submitting)
            }) {
                Ok(true) => {}
                _ => break,
            }
        })
        .detach();
    }

    // ---- actions ----

    fn start_prepare(&mut self, app_id: i64, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let dry_run = self.dry_run;
        let progress = {
            let v = self.view(app_id);
            v.phase = Phase::Preparing;
            v.error = None;
            v.review = None;
            v.inputs.clear();
            v.remember.clear();
            v.progress = Arc::default();
            v.progress.clone()
        };
        let sink = progress.clone();
        self.run(
            async move {
                apply::prepare(&core, app_id, ApplyOptions { dry_run }, move |stage| {
                    sink.lock().unwrap_or_else(|e| e.into_inner()).push(stage.label());
                })
                .await
            },
            move |this, result, cx| {
                this.view(app_id).busy = None;
                this.finish_prepare(app_id, result);
                this.reload_state(cx);
            },
            cx,
        );
        self.watch(app_id, cx);
        cx.notify();
    }

    fn finish_prepare(&mut self, app_id: i64, result: anyhow::Result<Prepared>) {
        let v = self.view(app_id);
        match result {
            Ok(Prepared::Review(r)) => v.set_review(r),
            Ok(Prepared::Handoff(h)) => v.phase = Phase::Handoff { handoff: h, headed: false },
            Err(e) => {
                v.phase = if v.review.is_some() { Phase::Review } else { Phase::Idle };
                v.error = Some(format!("{e:#}"));
            }
        }
    }

    /// Current value of every editable row, as edits for the core.
    fn edits(&self, app_id: i64, cx: &App) -> Vec<Edit> {
        let Some(v) = self.views.get(&app_id) else { return Vec::new() };
        let Some(review) = &v.review else { return Vec::new() };
        review
            .rows
            .iter()
            .filter(|r| r.editable)
            .filter_map(|r| {
                let value = v.inputs.get(&r.id).map(|i| input_value(i, cx)).unwrap_or_else(|| r.value.clone());
                Some(Edit { id: r.id.clone(), value, remember: v.remember.contains(&r.id) })
            })
            .collect()
    }

    /// Required rows that are still empty, from what is typed right now.
    fn missing(&self, app_id: i64, cx: &App) -> usize {
        let Some(v) = self.views.get(&app_id) else { return 0 };
        let Some(review) = &v.review else { return 0 };
        review
            .rows
            .iter()
            .filter(|r| r.editable && r.required && r.kind != FieldKind::File)
            .filter(|r| v.inputs.get(&r.id).map(|i| input_value(i, cx)).unwrap_or_else(|| r.value.clone()).trim().is_empty())
            .count()
    }

    /// The explicit confirmation: submit, or continue to the next step when the form has more.
    fn confirm(&mut self, app_id: i64, cx: &mut Context<Self>) {
        let Some(review) = self.view(app_id).review.clone() else { return };
        if review.dry_run && review.complete {
            return;
        }
        if self.missing(app_id, cx) > 0 {
            self.view(app_id).error = Some("Fill the required fields first.".into());
            cx.notify();
            return;
        }
        let edits = self.edits(app_id, cx);
        let core = self.core(cx);
        {
            let v = self.view(app_id);
            v.error = None;
            v.progress = Arc::default();
            v.phase = if review.complete { Phase::Submitting } else { Phase::Preparing };
        }
        if review.complete {
            self.run(
                async move { apply::submit(&core, app_id, edits).await },
                move |this, result, cx| {
                    let v = this.view(app_id);
                    match result {
                        Ok(Submitted::Applied { proof }) => {
                            v.phase = Phase::Done(Some(proof).filter(|p| !p.as_os_str().is_empty()));
                            v.review = None;
                        }
                        Ok(Submitted::DryRun) => v.phase = Phase::Review,
                        Ok(Submitted::Handoff(h)) => v.phase = Phase::Handoff { handoff: h, headed: false },
                        Err(e) => {
                            v.phase = Phase::Review;
                            v.error = Some(format!("{e:#}"));
                        }
                    }
                    this.reload_state(cx);
                },
                cx,
            );
        } else {
            let sink = self.view(app_id).progress.clone();
            self.run(
                async move {
                    apply::advance(&core, app_id, edits, move |stage| {
                        sink.lock().unwrap_or_else(|e| e.into_inner()).push(stage.label());
                    })
                    .await
                },
                move |this, result, cx| {
                    this.finish_prepare(app_id, result);
                    this.reload_state(cx);
                },
                cx,
            );
        }
        self.watch(app_id, cx);
        cx.notify();
    }

    fn discard(&mut self, app_id: i64, cx: &mut Context<Self>) {
        let core = self.core(cx);
        self.view(app_id).busy = Some("Closing the browser...");
        self.run(
            async move { apply::discard(&core, app_id).await },
            move |this, result, cx| {
                let v = this.view(app_id);
                v.busy = None;
                if let Err(e) = result {
                    v.error = Some(format!("{e:#}"));
                } else {
                    *v = AppView::new();
                }
                this.reload_state(cx);
            },
            cx,
        );
        cx.notify();
    }

    fn open_visible(&mut self, app_id: i64, cx: &mut Context<Self>) {
        let core = self.core(cx);
        {
            let v = self.view(app_id);
            v.busy = Some("Opening the browser...");
            v.error = None;
        }
        self.run(
            async move { apply::open_visible(&core, app_id).await },
            move |this, result, _| {
                let v = this.view(app_id);
                v.busy = None;
                match result {
                    Ok(()) => {
                        if let Phase::Handoff { headed, .. } = &mut v.phase {
                            *headed = true;
                        }
                    }
                    Err(e) => v.error = Some(format!("{e:#}")),
                }
            },
            cx,
        );
        cx.notify();
    }

    fn mark(&mut self, app_id: i64, applied: bool, cx: &mut Context<Self>) {
        let core = self.core(cx);
        self.view(app_id).busy = Some("Saving...");
        self.run(
            async move { apply::finish_manually(&core, app_id, applied).await },
            move |this, result, cx| {
                let v = this.view(app_id);
                v.busy = None;
                match result {
                    Ok(()) => *v = AppView::new(),
                    Err(e) => v.error = Some(format!("{e:#}")),
                }
                this.reload_state(cx);
            },
            cx,
        );
        cx.notify();
    }

    fn save_url(&mut self, app_id: i64, cx: &mut Context<Self>) {
        let Some(input) = self.url_input.clone() else { return };
        let raw = input.read(cx).value().to_string();
        let core = self.core(cx);
        match core.set_apply_url(app_id, &raw) {
            Ok(_) => {
                self.view(app_id).error = None;
                self.reload_state(cx);
            }
            Err(e) => self.view(app_id).error = Some(format!("{e:#}")),
        }
        cx.notify();
    }

    fn set_value(&mut self, app_id: i64, field: &str, value: String, cx: &mut Context<Self>) {
        self.view(app_id).inputs.insert(field.to_string(), FieldInput::Value(value));
        cx.notify();
    }

    fn toggle_remember(&mut self, app_id: i64, field: &str, cx: &mut Context<Self>) {
        let v = self.view(app_id);
        if !v.remember.remove(field) {
            v.remember.insert(field.to_string());
        }
        cx.notify();
    }

    // ---- inputs ----

    /// Build the editors of a new review once (needs a window).
    fn ensure_inputs(&mut self, app_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let v = self.views.entry(app_id).or_insert_with(AppView::new);
        if v.built == v.generation {
            return;
        }
        v.built = v.generation;
        v.inputs.clear();
        let Some(review) = v.review.clone() else { return };
        for row in &review.rows {
            let input = if !row.editable || row.kind == FieldKind::File || row.kind == FieldKind::Checkbox || uses_chips(row) {
                FieldInput::Value(row.value.clone())
            } else if row.kind == FieldKind::Textarea || row.value.chars().count() > 90 {
                let value = row.value.clone();
                FieldInput::Area(cx.new(|cx| {
                    let mut s = TextareaState::new(window, cx).auto_grow(2, 6).placeholder("Your answer");
                    s.set_value(value, window, cx);
                    s
                }))
            } else {
                let value = row.value.clone();
                let placeholder = if row.options.is_empty() { "Your answer".to_string() } else { format!("One of: {}", row.options.join(" | ")) };
                FieldInput::Line(cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder(placeholder);
                    s.set_value(value, window, cx);
                    s
                }))
            };
            v.inputs.insert(row.id.clone(), input);
        }
    }

    /// A run that is already open in the core (switching tabs, selecting another application).
    fn adopt_live(&mut self, app_id: i64, cx: &App) {
        let core = self.core(cx);
        let v = self.views.entry(app_id).or_insert_with(AppView::new);
        if !matches!(v.phase, Phase::Idle) {
            return;
        }
        match apply::live(&core, app_id) {
            apply::Live::Idle => {}
            apply::Live::Review(r) => v.set_review(r),
            apply::Live::Handoff { handoff, headed } => v.phase = Phase::Handoff { handoff, headed },
        }
    }
}

fn uses_chips(row: &Row) -> bool {
    matches!(row.kind, FieldKind::Select | FieldKind::Radio | FieldKind::CheckboxGroup | FieldKind::Combobox)
        && !row.options.is_empty()
        && row.options.len() <= MAX_CHIPS
}

fn input_value(input: &FieldInput, cx: &App) -> String {
    match input {
        FieldInput::Line(e) => e.read(cx).value().to_string(),
        FieldInput::Area(e) => e.read(cx).value().to_string(),
        FieldInput::Value(v) => v.clone(),
    }
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

fn host(url: &str) -> String {
    url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or(url).to_string()
}

/// Screenshot preview, scaled to the pane width.
fn preview(path: &PathBuf, cx: &App) -> Div {
    div()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .overflow_hidden()
        .child(img(path.clone()).w_full())
}

impl Render for ApplyTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(app) = self.state.read(cx).selected_app().cloned() else {
            return div().into_any_element();
        };
        if self.url_input.is_none() {
            self.url_input = Some(cx.new(|cx| InputState::new(window, cx).placeholder("Link of the application form, https://...")));
        }
        if self.current != Some(app.id) {
            self.current = Some(app.id);
        }
        self.adopt_live(app.id, cx);
        self.ensure_inputs(app.id, window, cx);

        let phase = self.view(app.id).phase.clone();
        let body = match phase {
            Phase::Idle => self.render_idle(&app, cx),
            Phase::Preparing => self.render_working(&app, "Preparing the application", cx),
            Phase::Submitting => self.render_working(&app, "Submitting", cx),
            Phase::Review => self.render_review(&app, cx),
            Phase::Done(proof) => self.render_done(&app, proof, cx),
            Phase::Handoff { handoff, headed } => self.render_handoff(&app, &handoff, headed, cx),
        };
        let theme = cx.theme();
        div()
            .id("apply-tab")
            .v_flex()
            .size_full()
            .gap_3()
            .overflow_y_scroll()
            .text_color(theme.foreground)
            .child(body)
            .into_any_element()
    }
}

// ---- rendering ----

impl ApplyTab {
    fn error_line(&mut self, app_id: i64, cx: &App) -> Option<Div> {
        let err = self.view(app_id).error.clone()?;
        Some(div().text_sm().text_color(cx.theme().danger).child(err))
    }

    fn render_idle(&mut self, app: &Application, cx: &mut Context<Self>) -> AnyElement {
        let core = self.core(cx);
        let info: ApplyInfo = apply::apply_info(&core, app);
        let applied = matches!(app.status, Status::Applied | Status::Interview | Status::Rejected | Status::Offer);
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let app_id = app.id;
        let url_line = match &info.apply_url {
            Some(url) => div().v_flex().child(div().text_sm().truncate().child(url.clone())).child(
                div().text_xs().text_color(muted).child(match info.url_source {
                    Some(apply::UrlSource::Analysis) => "found in the job description",
                    _ => "set for this application",
                }),
            ),
            None => div().v_flex().gap_2().child(div().text_sm().text_color(theme.warning).child("No apply URL found in the job description.")).child(
                div()
                    .h_flex()
                    .gap_2()
                    .children(self.url_input.clone().map(|i| div().flex_1().child(Input::new(&i))))
                    .child(Button::new("save-apply-url").primary().label("Save").on_click(cx.listener(move |this, _, _, cx| this.save_url(app_id, cx)))),
            ),
        };
        let session_line = match &info.session {
            SessionState::NotNeeded => "No login needed: runs in a throwaway browser profile.".to_string(),
            SessionState::Matched { name, status, .. } => match status {
                SessionStatus::Ok => format!("Session: {name} (logged in)"),
                SessionStatus::New => format!("Session: {name} (not verified yet: check it in Settings)"),
                SessionStatus::Expired => format!("Session: {name} (expired: log in again in Settings)"),
            },
            SessionState::Missing { domain } => format!("No browser session for {domain}: create one in Settings > Browser sessions."),
        };
        let session_color = match &info.session {
            SessionState::Missing { .. } => theme.danger,
            SessionState::Matched { status: SessionStatus::Expired, .. } => theme.warning,
            _ => muted,
        };
        let blocker = info.blocker();
        let forced = info.dry_run_forced;
        let dry = self.dry_run || forced;
        let proof = applied.then(|| apply::latest_proof(&core, &app.slug)).flatten();

        div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .border_1()
                    .border_color(theme.border)
                    .rounded_md()
                    .child(div().h_flex().gap_2().items_center().child(div().font_semibold().child(info.ats.label())).child(Tag::secondary().child("detected")))
                    .child(url_line)
                    .child(div().text_sm().text_color(session_color).child(session_line)),
            )
            .child(div().text_sm().text_color(muted).child(if applied {
                "This application was sent. Replies are tracked from your email."
            } else {
                "Press Review & Submit to open the form and fill it in. You review every answer first; nothing is sent until you confirm."
            }))
            .when(!applied, |d| {
                d.child(
                    Checkbox::new("dry-run")
                        .label("Dry run: fill the form but never submit")
                        .checked(dry)
                        .disabled(forced)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.dry_run = *checked;
                            cx.notify();
                        })),
                )
                .when(forced, |d| d.child(div().text_xs().text_color(muted).child("JOBPILOT_DRY_RUN is set: submitting is disabled.")))
            })
            .when_some(blocker, |d, why| d.child(div().text_sm().text_color(theme.warning).child(why)))
            .when_some(proof, |d, p| d.child(div().text_sm().font_semibold().child("Confirmation")).child(preview(&p, cx)))
            .children(self.error_line(app.id, cx))
            .into_any_element()
    }

    fn render_working(&mut self, app: &Application, title: &str, cx: &mut Context<Self>) -> AnyElement {
        let lines = self.view(app.id).progress.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let theme = cx.theme();
        let last = lines.len().saturating_sub(1);
        div()
            .v_flex()
            .gap_3()
            .p_3()
            .border_1()
            .border_color(theme.border)
            .rounded_md()
            .child(div().h_flex().gap_2().items_center().child(Spinner::new()).child(div().font_semibold().child(title.to_string())))
            .children(lines.iter().enumerate().map(|(i, l)| {
                div().text_sm().text_color(if i == last { theme.foreground } else { theme.muted_foreground }).child(format!("{} {l}", if i == last { ">" } else { "ok" }))
            }))
            .child(div().text_xs().text_color(theme.muted_foreground).child("A headless browser is working on the form. This can take up to a minute."))
            .into_any_element()
    }

    fn render_review(&mut self, app: &Application, cx: &mut Context<Self>) -> AnyElement {
        let app_id = app.id;
        let Some(review) = self.view(app_id).review.clone() else { return self.render_idle(app, cx) };
        let missing = self.missing(app_id, cx);
        let remember = self.view(app_id).remember.clone();
        let busy = self.view(app_id).busy;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let editable = review.rows.iter().filter(|r| r.editable).count();
        let needs = review.rows.iter().filter(|r| r.editable && (r.needs_input || r.warning.is_some())).count();

        let (action_label, action_ready) = if review.complete { ("Submit application", !review.dry_run) } else { ("Continue", true) };
        let header = div()
            .h_flex()
            .gap_2()
            .items_center()
            .justify_between()
            .child(
                div()
                    .v_flex()
                    .min_w_0()
                    .child(div().font_semibold().child(format!("{} - {}", review.ats.label(), host(&review.url))))
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{editable} fields on this step, {needs} to check{}",
                        review.session.as_ref().map(|s| format!(" - session {s}")).unwrap_or_default()
                    ))),
            )
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(
                        Button::new("discard-review")
                            .outline()
                            .label(if review.dry_run { "Close dry run" } else { "Discard" })
                            .disabled(busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| this.discard(app_id, cx))),
                    )
                    .when(action_ready, |d| {
                        d.child(
                            Button::new("submit-review")
                                .primary()
                                .label(action_label)
                                .disabled(missing > 0 || busy.is_some())
                                .on_click(cx.listener(move |this, _, _, cx| this.confirm(app_id, cx))),
                        )
                    }),
            );

        let banner = if review.dry_run {
            Some("Dry run: the form is filled but will never be submitted.".to_string())
        } else if !review.complete {
            Some("This form has more steps. Fill the highlighted fields, then press Continue (nothing is submitted yet).".to_string())
        } else if missing > 0 {
            Some(format!("{missing} required field(s) still empty: fill them to enable Submit."))
        } else {
            None
        };

        let rows: Vec<AnyElement> = review.rows.iter().map(|row| self.render_row(app_id, row, &remember, cx)).collect();

        let attached = review.attachment.as_ref().map(|p| file_name(&p.to_string_lossy()));
        let shots = review.screenshots.clone();
        let notes = review.notes.clone();
        let theme = cx.theme();
        div()
            .v_flex()
            .gap_3()
            .child(header)
            .when_some(banner, |d, b| d.child(div().text_sm().p_2().rounded_md().bg(theme.warning.opacity(0.12)).child(b)))
            .children(self.error_line(app_id, cx))
            .children(notes.into_iter().map(|n| div().text_sm().text_color(theme.warning).child(n)))
            .child(div().v_flex().gap_2().children(rows))
            .when_some(attached, |d, name| {
                d.child(div().h_flex().gap_2().items_center().child(Tag::secondary().child("attached")).child(div().text_sm().child(name)))
            })
            .when(!shots.is_empty(), |d| {
                d.child(div().text_sm().font_semibold().child("Filled form"))
                    .children(shots.iter().map(|p| preview(p, cx)))
            })
            .into_any_element()
    }

    fn render_row(&mut self, app_id: i64, row: &Row, remember: &HashSet<String>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let flagged = row.editable && (row.needs_input || row.warning.is_some());
        let current = self.views.get(&app_id).and_then(|v| v.inputs.get(&row.id)).map(|i| input_value(i, cx)).unwrap_or_else(|| row.value.clone());
        let empty_required = row.editable && row.required && row.kind != FieldKind::File && current.trim().is_empty();

        let tag = match (row.needs_input, row.source) {
            (true, _) | (_, Source::Unknown) => Tag::danger().child("needs input"),
            (_, Source::Profile) => Tag::success().child("profile"),
            (_, Source::Generated) => Tag::warning().child("generated"),
            (_, s) => Tag::secondary().child(s.as_str()),
        };

        let editor: AnyElement = if !row.editable || row.kind == FieldKind::File {
            let shown = if row.kind == FieldKind::File { file_name(&row.value) } else { row.value.clone() };
            div().text_sm().text_color(if shown.is_empty() { muted } else { theme.foreground }).child(if shown.is_empty() { "(empty)".to_string() } else { shown }).into_any_element()
        } else {
            match self.views.get(&app_id).and_then(|v| v.inputs.get(&row.id)) {
                Some(FieldInput::Line(e)) => Input::new(e).into_any_element(),
                Some(FieldInput::Area(e)) => Textarea::new(e).into_any_element(),
                _ if row.kind == FieldKind::Checkbox => {
                    let id = row.id.clone();
                    Checkbox::new(SharedString::from(format!("cb-{}", row.id)))
                        .label("Yes")
                        .checked(current == "yes")
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.set_value(app_id, &id, if *checked { "yes" } else { "no" }.to_string(), cx)
                        }))
                        .into_any_element()
                }
                _ => {
                    let multi = row.kind == FieldKind::CheckboxGroup;
                    let chosen: Vec<String> = if multi { current.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect() } else { vec![current.clone()] };
                    div()
                        .h_flex()
                        .flex_wrap()
                        .gap_2()
                        .children(row.options.iter().enumerate().map(|(i, option)| {
                            let selected = chosen.iter().any(|c| c.eq_ignore_ascii_case(option));
                            let (id, option_value, all) = (row.id.clone(), option.clone(), chosen.clone());
                            let button = Button::new(SharedString::from(format!("chip-{}-{i}", row.id))).small().label(option.clone());
                            let button = if selected { button.primary() } else { button.outline() };
                            button.on_click(cx.listener(move |this, _, _, cx| {
                                let next = if multi {
                                    let mut all = all.clone();
                                    if let Some(pos) = all.iter().position(|c| c.eq_ignore_ascii_case(&option_value)) {
                                        all.remove(pos);
                                    } else {
                                        all.push(option_value.clone());
                                    }
                                    all.join("; ")
                                } else if selected {
                                    String::new()
                                } else {
                                    option_value.clone()
                                };
                                this.set_value(app_id, &id, next, cx);
                            }))
                        }))
                        .into_any_element()
                }
            }
        };

        let can_remember = row.rememberable() && !current.trim().is_empty();
        let id = row.id.clone();
        div()
            .v_flex()
            .gap_1()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(if flagged || empty_required { theme.warning } else { theme.border })
            .when(flagged || empty_required, |d| d.bg(theme.warning.opacity(0.08)))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().text_sm().font_semibold().child(format!("{}{}", row.label, if row.required { " *" } else { "" })))
                    .when(!row.editable, |d| d.child(Tag::secondary().child(format!("step {}", row.step + 1))))
                    .child(tag),
            )
            .child(editor)
            .when_some(row.warning.clone(), |d, w| d.child(div().text_xs().text_color(theme.warning).child(w)))
            .when(can_remember, |d| {
                d.child(
                    Checkbox::new(SharedString::from(format!("remember-{}", row.id)))
                        .label("Remember this answer in profile.yaml")
                        .checked(remember.contains(&row.id))
                        .on_click(cx.listener(move |this, _: &bool, _, cx| this.toggle_remember(app_id, &id, cx))),
                )
            })
            .into_any_element()
    }

    fn render_done(&mut self, app: &Application, proof: Option<PathBuf>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(theme.success.opacity(0.12))
                    .child(div().font_semibold().child("Application submitted"))
                    .child(div().text_sm().child(format!("{} confirmed the application for {}. Replies will show up from your email.", host(app.apply_url.as_deref().unwrap_or("the site")), app.role))),
            )
            .when_some(proof, |d, p| d.child(div().text_sm().font_semibold().child("Proof")).child(preview(&p, cx)))
            .into_any_element()
    }

    fn render_handoff(&mut self, app: &Application, h: &Handoff, headed: bool, cx: &mut Context<Self>) -> AnyElement {
        let app_id = app.id;
        let busy = self.view(app_id).busy;
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        div()
            .v_flex()
            .gap_3()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .p_3()
                    .rounded_md()
                    .bg(theme.danger.opacity(0.1))
                    .child(div().font_semibold().child(if h.captcha { "A CAPTCHA is in the way" } else { "The form needs you" }))
                    .child(div().text_sm().child(h.reason.clone())),
            )
            .child(div().text_sm().text_color(muted).child(if headed {
                "The visible browser is open at the form (your earlier answers were filled in again where possible). Finish and submit there, then mark the result below."
            } else {
                "Open a visible browser on the same profile to finish by hand, then mark the result."
            }))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("open-visible")
                            .primary()
                            .label(if headed { "Reopen visible browser" } else { "Open visible browser" })
                            .loading(busy == Some("Opening the browser..."))
                            .disabled(busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| this.open_visible(app_id, cx))),
                    )
                    .child(
                        Button::new("mark-applied")
                            .outline()
                            .label("Mark as applied")
                            .disabled(busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| this.mark(app_id, true, cx))),
                    )
                    .child(
                        Button::new("mark-failed")
                            .outline()
                            .label("Mark failed")
                            .disabled(busy.is_some())
                            .on_click(cx.listener(move |this, _, _, cx| this.mark(app_id, false, cx))),
                    )
                    .children(busy.map(|b| div().text_sm().text_color(muted).child(b))),
            )
            .children(self.error_line(app_id, cx))
            .when_some(h.screenshot.clone(), |d, p| d.child(div().text_sm().font_semibold().child("What the page looked like")).child(preview(&p, cx)))
            .into_any_element()
    }
}
