//! Pipeline screen: every application as a card on a kanban board (CV review, Ready to apply,
//! Submitted, Interview, Closed), and the latest events across all of them.
//!
//! A card says what the user does next (`next_action`), and opens the application when pressed. Views
//! only: applications come from `AppState`, events from `Store::list_events` on the core runtime.
//! There is no primary button here: the board is for looking around, the action is in the detail.

use std::collections::HashSet;

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Local, NaiveDate, NaiveDateTime, Utc};
use jobpilot_core::runtime;
use jobpilot_core::store::{Application, Event, Status};
use jobpilot_core::Core;

use crate::app::{AppState, Screen};
use crate::ui::tokens::{RADIUS_CARD, TEXT_SMALL};
use crate::ui::{self, Level, Tone};

/// How many events "Recent activity" shows.
const ACTIVITY_ROWS: usize = 6;
/// Events are read for this many applications (the most recently updated ones): enough for the
/// latest few lines, and a bound on the reads.
const ACTIVITY_APPS: usize = 20;
/// Narrowest a column gets before the board scrolls sideways.
const COLUMN_MIN_WIDTH: f32 = 200.0;
/// Gap between columns.
const COLUMN_GAP: f32 = 14.0;

// ---------------------------------------------------------------- columns

/// The five stages of the board, left to right.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Column {
    CvReview,
    ReadyToApply,
    Submitted,
    Interview,
    Closed,
}

impl Column {
    pub const ALL: [Column; 5] =
        [Column::CvReview, Column::ReadyToApply, Column::Submitted, Column::Interview, Column::Closed];

    pub fn label(self) -> &'static str {
        match self {
            Self::CvReview => "CV review",
            Self::ReadyToApply => "Ready to apply",
            Self::Submitted => "Submitted",
            Self::Interview => "Interview",
            Self::Closed => "Closed",
        }
    }
}

/// The column a status belongs in. Every status has one: a failed submit waits in "Ready to apply"
/// (the primary button there is "Review & Submit" again), an offer stays with the interviews.
pub fn column_for(status: Status) -> Column {
    match status {
        Status::Draft => Column::CvReview,
        Status::CvReady | Status::Submitting | Status::Failed => Column::ReadyToApply,
        Status::Applied => Column::Submitted,
        Status::Interview | Status::Offer => Column::Interview,
        Status::Rejected => Column::Closed,
    }
}

/// All five columns in order (empty ones too), each with its applications in the order of `apps`
/// (the store lists the most recently updated first).
pub fn board(apps: &[Application]) -> Vec<(Column, Vec<&Application>)> {
    Column::ALL.into_iter().map(|c| (c, apps.iter().filter(|a| column_for(a.status) == c).collect())).collect()
}

/// What the user does next with an application, as the card's tag says it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NextAction {
    pub label: &'static str,
    pub tone: Tone,
}

const fn next(label: &'static str, tone: Tone) -> NextAction {
    NextAction { label, tone }
}

/// The tag of a card. It follows the detail's one primary button (Generate -> Approve CV -> Review &
/// Submit) while the application is open, and says where it stands once it is not: `busy` = a
/// generate / revise job is running for it, `has_cv` = its `cv.yaml` exists.
pub fn next_action(status: Status, has_cv: bool, busy: bool) -> NextAction {
    if busy {
        return next(if has_cv { "Working" } else { "Generating" }, Tone::Neutral);
    }
    match status {
        Status::Draft if !has_cv => next("Generate CV", Tone::Accent),
        Status::Draft => next("Approve CV", Tone::Accent),
        Status::CvReady | Status::Submitting => next("Review & submit", Tone::Accent),
        Status::Failed => next("Submit failed", Tone::Danger),
        Status::Applied => next("No reply yet", Tone::Neutral),
        Status::Interview => next("Check your email", Tone::Warn),
        Status::Offer => next("Offer", Tone::Ok),
        Status::Rejected => next("Rejected", Tone::Neutral),
    }
}

// ---------------------------------------------------------------- times

/// How long ago a card last changed: `today`, `1 d`, `12 d`.
pub fn age_label(today: NaiveDate, then: NaiveDate) -> String {
    match (today - then).num_days() {
        d if d <= 0 => "today".to_string(),
        d => format!("{d} d"),
    }
}

/// When an event happened: the time today, `Yesterday`, the weekday within the last week, else the date.
pub fn activity_when(now: NaiveDateTime, then: NaiveDateTime) -> String {
    match (now.date() - then.date()).num_days() {
        d if d <= 0 => then.format("%H:%M").to_string(),
        1 => "Yesterday".to_string(),
        2..=6 => then.format("%a").to_string(),
        _ => then.format("%d %b").to_string(),
    }
}

fn local(at: DateTime<Utc>) -> NaiveDateTime {
    at.with_timezone(&Local).naive_local()
}

// ---------------------------------------------------------------- activity

/// The colour of an event's dot: errors red, mail and warnings amber, status moves blue.
pub fn activity_tone(kind: &str) -> Tone {
    match kind {
        "error" => Tone::Danger,
        "email" | "email_review" | "warning" => Tone::Warn,
        "status" => Tone::Accent,
        _ => Tone::Neutral,
    }
}

/// An event's text for people: a stored status move (`cv_ready -> applied: note`) reads
/// "moved to Applied: note"; anything else is shown as stored.
pub fn activity_text(kind: &str, message: &str) -> String {
    if kind != "status" {
        return message.to_string();
    }
    let Some((_, rest)) = message.split_once(" -> ") else {
        return message.to_string();
    };
    let (to, note) = rest.split_once(": ").map_or((rest, None), |(t, n)| (t, Some(n)));
    match (Status::parse(to.trim()), note) {
        (Some(st), Some(n)) => format!("moved to {}: {n}", st.label()),
        (Some(st), None) => format!("moved to {}", st.label()),
        (None, _) => message.to_string(),
    }
}

/// The newest `n` events, newest first.
pub fn latest_events(mut events: Vec<Event>, n: usize) -> Vec<Event> {
    events.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    events.truncate(n);
    events
}

/// What the board needs besides the applications, read off the UI thread.
#[derive(Default)]
struct Loaded {
    /// Applications whose `cv.yaml` exists.
    has_cv: HashSet<i64>,
    activity: Vec<Event>,
}

fn load(core: &Core, apps: &[Application]) -> Loaded {
    let has_cv = apps.iter().filter(|a| a.cv_yaml_path().is_file()).map(|a| a.id).collect();
    let mut events = Vec::new();
    for app in apps.iter().take(ACTIVITY_APPS) {
        match core.store.list_events(app.id) {
            Ok(list) => events.extend(list),
            Err(e) => tracing::warn!("reading the events of application {} failed: {e:#}", app.id),
        }
    }
    Loaded { has_cv, activity: latest_events(events, ACTIVITY_ROWS) }
}

/// What a read of [`Loaded`] was made from: when it differs, the board reads again.
#[derive(Clone, PartialEq, Eq)]
struct Key {
    epoch: u64,
    apps: Vec<(i64, Status, DateTime<Utc>, u64)>,
}

// ---------------------------------------------------------------- view

pub struct PipelineView {
    state: Entity<AppState>,
    loaded: Loaded,
    /// What the data in `loaded` is being read for.
    requested: Option<Key>,
    /// The latest read that may still apply (an older answer is dropped).
    load_seq: u64,
}

impl PipelineView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // The state notifies on every status change, mail sync and job progress; the board reads
        // again only when an application changed (or the screen was shown again).
        cx.observe(&state, |this, _, cx| {
            this.refresh(cx);
            cx.notify();
        })
        .detach();
        let mut view = Self { state, loaded: Loaded::default(), requested: None, load_seq: 0 };
        view.refresh(cx);
        view
    }

    fn key(&self, cx: &Context<Self>) -> Key {
        let s = self.state.read(cx);
        Key {
            epoch: s.pipeline_epoch,
            apps: s.applications.iter().map(|a| (a.id, a.status, a.updated_at, s.cv_epoch(a.id))).collect(),
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let key = self.key(cx);
        if self.requested.as_ref() == Some(&key) {
            return;
        }
        self.requested = Some(key);
        self.load_seq += 1;
        let seq = self.load_seq;
        let (core, apps) = {
            let s = self.state.read(cx);
            (s.core.clone(), s.applications.clone())
        };
        let rx = runtime::spawn(async move { load(&core, &apps) });
        cx.spawn(async move |this, cx| {
            let Ok(loaded) = rx.await else { return };
            this.update(cx, |this, cx| {
                if this.load_seq == seq {
                    this.loaded = loaded;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open the application, as the sidebar's list used to.
    fn open(&self, id: i64, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.select(id, cx));
    }

    fn card(&self, app: &Application, now: NaiveDateTime, cx: &Context<Self>) -> impl IntoElement {
        let p = ui::palette(cx);
        let busy = self.state.read(cx).is_busy(app.id);
        let action = next_action(app.status, self.loaded.has_cv.contains(&app.id), busy);
        let (title, subtitle) = match (app.role.trim(), app.company.trim()) {
            ("", "") => ("Untitled".to_string(), String::new()),
            ("", company) => (company.to_string(), String::new()),
            (role, company) => (role.to_string(), company.to_string()),
        };
        let id = app.id;
        let aria = format!("{subtitle}, {title}: {}, {}", action.label, app.status.label());
        ui::kanban_card(("card", id as usize), &title, &subtitle, None, cx)
            .aria_label(aria)
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pt_1()
                    .child(ui::tag(action.label, action.tone, cx))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(TEXT_SMALL))
                            .text_color(p.subtle)
                            .child(age_label(now.date(), local(app.updated_at).date())),
                    ),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.open(id, cx)))
    }

    fn column(&self, index: usize, column: Column, apps: Vec<&Application>, now: NaiveDateTime, cx: &Context<Self>) -> impl IntoElement {
        let p = ui::palette(cx);
        let count = apps.len();
        div()
            .id(("column", index))
            .role(Role::Group)
            .aria_label(format!("{}, {count}", column.label()))
            .v_flex()
            .flex_1()
            .min_w(px(COLUMN_MIN_WIDTH))
            .gap_2p5()
            .p_3()
            .rounded(px(RADIUS_CARD))
            .bg(p.track)
            .child(
                div()
                    .h_flex()
                    .justify_between()
                    .px_1()
                    .py_0p5()
                    .text_size(px(13.0))
                    .font_semibold()
                    .text_color(p.ink)
                    .child(column.label())
                    .child(div().font_medium().text_color(p.subtle).child(count.to_string())),
            )
            .children(apps.into_iter().map(|app| self.card(app, now, cx)))
            .when(count == 0, |d| d.child(div().px_1().text_size(px(TEXT_SMALL)).text_color(p.subtle).child("Nothing here")))
    }

    fn activity(&self, apps: &[Application], now: NaiveDateTime, cx: &Context<Self>) -> impl IntoElement {
        let rows: Vec<AnyElement> = self
            .loaded
            .activity
            .iter()
            .filter_map(|e| {
                // An event of an application that is gone has nothing to open.
                let app = apps.iter().find(|a| a.id == e.app_id)?;
                let text = format!("{}: {}", app.company, activity_text(&e.kind, &e.message));
                let when = activity_when(now, local(e.at));
                let (id, app_id) = (e.id, app.id);
                Some(
                    div()
                        .id(("activity", id as usize))
                        .role(Role::Button)
                        .aria_label(format!("{text}, {when}"))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.open(app_id, cx)))
                        .child(ui::activity_row(activity_tone(&e.kind), &text, &when, cx))
                        .into_any_element(),
                )
            })
            .collect();
        let empty = rows.is_empty();
        ui::card(cx)
            .gap_3()
            .child(ui::heading("Recent activity", Level::H2, cx))
            .child(div().v_flex().children(rows))
            .when(empty, |d| d.child(ui::caption("Nothing has happened yet.", cx)))
    }
}

impl Render for PipelineView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let apps = self.state.read(cx).applications.clone();
        let now = Local::now().naive_local();
        let count = apps.len();
        let summary = match count {
            0 => "No applications yet.".to_string(),
            1 => "1 application. Stages update from your Gmail replies.".to_string(),
            n => format!("{n} applications. Stages update from your Gmail replies."),
        };
        let columns: Vec<AnyElement> = board(&apps)
            .into_iter()
            .enumerate()
            .map(|(i, (column, rows))| self.column(i, column, rows, now, cx).into_any_element())
            .collect();
        let min_board_width = Column::ALL.len() as f32 * COLUMN_MIN_WIDTH + (Column::ALL.len() - 1) as f32 * COLUMN_GAP;

        let body = if count == 0 {
            let to_today = ui::secondary_button("pipeline-to-today", "Go to Today")
                .on_click(cx.listener(|this, _, _, cx| this.state.update(cx, |s, cx| s.show(Screen::Jobs, cx))))
                .into_any_element();
            ui::empty_state(
                IconName::LayoutDashboard,
                "No applications yet",
                "Prepare a CV for a job on Today and it shows up here.",
                Some(to_today),
                cx,
            )
            .into_any_element()
        } else {
            div()
                .v_flex()
                .gap(px(22.0))
                .child(
                    // The board scrolls sideways when the window is too narrow for five columns.
                    div().id("board").overflow_x_scroll().pb_2().child(
                        div().h_flex().items_stretch().gap(px(COLUMN_GAP)).w_full().min_w(px(min_board_width)).children(columns),
                    ),
                )
                .child(self.activity(&apps, now, cx))
                .into_any_element()
        };

        div()
            .id("pipeline")
            .role(Role::Main)
            .aria_label("Pipeline")
            .size_full()
            .overflow_y_scroll()
            .child(
                div()
                    .v_flex()
                    .gap(px(22.0))
                    .px_8()
                    .pt(px(28.0))
                    .pb_10()
                    .child(
                        div()
                            .v_flex()
                            .gap_0p5()
                            .child(ui::heading("Pipeline", Level::H1, cx))
                            .child(ui::caption(summary, cx)),
                    )
                    .child(body),
            )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the file glob-imports gpui_kit, whose `test` attribute would shadow the standard one.
    use jobpilot_core::chrono::{NaiveDate, NaiveDateTime, TimeZone, Utc};
    use jobpilot_core::store::{Application, Event, Status};

    use super::{
        activity_text,
        activity_tone, activity_when, age_label, board, column_for, latest_events, next_action, Column,
    };
    use crate::ui::Tone;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).expect("a valid date")
    }

    fn at(d: u32, h: u32, m: u32) -> NaiveDateTime {
        day(d).and_hms_opt(h, m, 0).expect("a valid time")
    }

    #[test]
    fn every_status_has_a_column() {
        for (status, column) in [
            (Status::Draft, Column::CvReview),
            (Status::CvReady, Column::ReadyToApply),
            (Status::Submitting, Column::ReadyToApply),
            (Status::Failed, Column::ReadyToApply),
            (Status::Applied, Column::Submitted),
            (Status::Interview, Column::Interview),
            (Status::Offer, Column::Interview),
            (Status::Rejected, Column::Closed),
        ] {
            assert_eq!(column_for(status), column, "{status:?}");
        }
        // The table above must list every status, so a new one cannot be forgotten.
        assert_eq!(Status::ALL.len(), 8);
        for status in Status::ALL {
            assert!(Column::ALL.contains(&column_for(status)), "{status:?}");
        }
    }

    #[test]
    fn columns_run_from_review_to_closed() {
        assert_eq!(
            Column::ALL.map(Column::label),
            ["CV review", "Ready to apply", "Submitted", "Interview", "Closed"]
        );
    }

    fn app(id: i64, status: Status) -> Application {
        let moment = Utc.with_ymd_and_hms(2026, 11, 1, 9, 0, 0).single().expect("a valid moment");
        Application {
            id,
            slug: format!("a{id}"),
            company: format!("Company {id}"),
            role: "Engineer".into(),
            source: None,
            apply_url: None,
            status,
            dir: Default::default(),
            analysis_json: None,
            gaps_json: None,
            created_at: moment,
            updated_at: moment,
            insight_json: None,
        }
    }

    #[test]
    fn the_board_lists_all_columns_and_keeps_the_order() {
        let apps = vec![
            app(1, Status::Applied),
            app(2, Status::Draft),
            app(3, Status::Applied),
            app(4, Status::Offer),
            app(5, Status::Rejected),
        ];
        let ids = |rows: &[&Application]| rows.iter().map(|a| a.id).collect::<Vec<_>>();
        let columns = board(&apps);
        assert_eq!(columns.iter().map(|(c, _)| *c).collect::<Vec<_>>(), Column::ALL);
        assert_eq!(ids(&columns[0].1), [2]);
        assert!(columns[1].1.is_empty(), "an empty column is still there");
        assert_eq!(ids(&columns[2].1), [1, 3], "the list's order is kept inside a column");
        assert_eq!(ids(&columns[3].1), [4]);
        assert_eq!(ids(&columns[4].1), [5]);
        assert!(board(&[]).iter().all(|(_, rows)| rows.is_empty()));
    }

    #[test]
    fn the_tag_is_the_next_action() {
        let label = |status, has_cv| next_action(status, has_cv, false).label;
        assert_eq!(label(Status::Draft, false), "Generate CV");
        assert_eq!(label(Status::Draft, true), "Approve CV");
        assert_eq!(label(Status::CvReady, true), "Review & submit");
        assert_eq!(label(Status::Submitting, true), "Review & submit");
        assert_eq!(label(Status::Failed, true), "Submit failed");
        assert_eq!(label(Status::Applied, true), "No reply yet");
        assert_eq!(label(Status::Interview, true), "Check your email");
        assert_eq!(label(Status::Offer, true), "Offer");
        assert_eq!(label(Status::Rejected, true), "Rejected");
        // Every status says something, with a tone that fits.
        for status in Status::ALL {
            assert!(!next_action(status, true, false).label.is_empty(), "{status:?}");
        }
        assert_eq!(next_action(Status::Failed, true, false).tone, Tone::Danger);
        assert_eq!(next_action(Status::Interview, true, false).tone, Tone::Warn);
        assert_eq!(next_action(Status::Offer, true, false).tone, Tone::Ok);
    }

    #[test]
    fn running_work_wins_over_the_status() {
        assert_eq!(next_action(Status::Draft, false, true).label, "Generating");
        assert_eq!(next_action(Status::Draft, true, true).label, "Working");
        assert_eq!(next_action(Status::CvReady, true, true).label, "Working");
        assert_eq!(next_action(Status::Draft, false, true).tone, Tone::Neutral);
    }

    #[test]
    fn ages_count_whole_days() {
        assert_eq!(age_label(day(14), day(14)), "today");
        assert_eq!(age_label(day(14), day(13)), "1 d");
        assert_eq!(age_label(day(14), day(2)), "12 d");
        assert_eq!(age_label(day(14), day(15)), "today", "a clock a little ahead is not negative");
    }

    #[test]
    fn events_are_dated_for_a_glance() {
        let now = at(14, 18, 0);
        assert_eq!(activity_when(now, at(14, 9, 5)), "09:05");
        assert_eq!(activity_when(now, at(13, 23, 59)), "Yesterday");
        // 2026-11-12 is a Thursday, 2026-11-09 a Monday.
        assert_eq!(activity_when(now, at(12, 8, 0)), "Thu");
        assert_eq!(activity_when(now, at(9, 8, 0)), "Mon");
        assert_eq!(activity_when(now, at(8, 8, 0)), "Sun", "six days ago still has a weekday");
        assert_eq!(activity_when(now, at(7, 8, 0)), "07 Nov");
    }

    fn event(id: i64, hour: u32, kind: &str) -> Event {
        let at = Utc.with_ymd_and_hms(2026, 11, 14, hour, 0, 0).single().expect("a valid moment");
        Event { id, app_id: 1, at, kind: kind.into(), message: format!("event {id}") }
    }

    #[test]
    fn the_latest_events_come_first_and_are_capped() {
        let events = vec![event(1, 9, "created"), event(2, 12, "status"), event(3, 12, "email"), event(4, 7, "note")];
        let ids = |list: &[Event]| list.iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(ids(&latest_events(events.clone(), 3)), [3, 2, 1], "the newest first, the later row first on a tie");
        assert_eq!(ids(&latest_events(events.clone(), 10)), [3, 2, 1, 4]);
        assert!(latest_events(events, 0).is_empty());
        assert!(latest_events(Vec::new(), 3).is_empty());
    }

    #[test]
    fn events_have_a_tone() {
        assert_eq!(activity_tone("error"), Tone::Danger);
        assert_eq!(activity_tone("email"), Tone::Warn);
        assert_eq!(activity_tone("email_review"), Tone::Warn);
        assert_eq!(activity_tone("status"), Tone::Accent);
        assert_eq!(activity_tone("created"), Tone::Neutral);
        assert_eq!(activity_tone("anything else"), Tone::Neutral);
    }

    #[test]
    fn a_status_move_reads_as_words() {
        assert_eq!(activity_text("status", "draft -> cv_ready: demo data"), "moved to CV ready: demo data");
        assert_eq!(activity_text("status", "applied -> interview"), "moved to Interview");
        assert_eq!(activity_text("status", "odd text"), "odd text");
        assert_eq!(activity_text("email", "a -> b"), "a -> b");
    }
}
