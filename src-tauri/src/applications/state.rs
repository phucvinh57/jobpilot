//! What this context keeps in memory per application while the app runs: the pipeline job that is
//! running (its progress line), the last failure, what to run again on Retry, a counter that says
//! "the CV files changed", and the Apply tab's session (phase, review, typed answers).
//!
//! The web app never holds this: it asks for it (`applications_get`, `applications_apply_state`)
//! and is told when it moved (`shared:data-changed`, `applications:progress`).
//! One pipeline job per application at a time.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use crate::engine::apply::{Edit, Handoff, Live, Prepared, Review, Submitted};
use crate::engine::pipeline::{JdInput, Progress, Stage};

use super::rules::{edits_for, missing_count, progress_label, ApplyUi};

/// What to run again when a job failed and the user presses Retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Job {
    Generate,
    Revise(String),
    /// "Approve CV": not a pipeline job, but its failure is retried as itself (never as Generate,
    /// which would rewrite the CV).
    Approve,
}

// ---------------------------------------------------------------- the Apply tab

#[derive(Clone)]
pub enum Phase {
    Idle,
    Preparing,
    Review,
    Submitting,
    Done(Option<PathBuf>),
    Handoff { handoff: Handoff, headed: bool },
}

/// What the Apply tab is doing for one application.
#[derive(Clone)]
pub struct ApplyView {
    pub phase: Phase,
    pub review: Option<Review>,
    /// What the user typed (field id -> value); rows without an entry keep the value they were filled with.
    pub typed: HashMap<String, String>,
    /// Rows whose answer is remembered for the next applications.
    pub remember: HashSet<String>,
    /// Bumped when a new review arrives, so the web app starts its editors from it once.
    pub generation: u64,
    pub error: Option<String>,
    pub progress: Vec<String>,
    /// A short action that is running (closing the browser, saving...).
    pub busy: Option<&'static str>,
}

/// What pressing Submit / Continue does.
#[derive(Debug)]
pub enum Confirm {
    /// Nothing to confirm (nothing is open, a dry run, work is running).
    Ignore,
    /// Required fields are empty: nothing was started; the error is set.
    Blocked,
    /// The form is complete: send it with these edits.
    Submit(Vec<Edit>),
    /// The form has more steps: fill the next one with these edits.
    Advance(Vec<Edit>),
}

impl ApplyView {
    pub fn new() -> Self {
        Self {
            phase: Phase::Idle,
            review: None,
            typed: HashMap::new(),
            remember: HashSet::new(),
            generation: 0,
            error: None,
            progress: Vec::new(),
            busy: None,
        }
    }

    /// Forget everything about the run (the generation keeps counting).
    pub fn reset(&mut self) {
        *self = Self {
            generation: self.generation,
            ..Self::new()
        };
    }

    pub fn set_review(&mut self, review: Review) {
        self.review = Some(review);
        self.generation += 1;
        self.typed.clear();
        self.remember.clear();
        self.phase = Phase::Review;
    }

    /// Required rows that are empty right now.
    pub fn missing(&self) -> usize {
        self.review
            .as_ref()
            .map_or(0, |r| missing_count(r, &self.typed))
    }

    /// The state the primary button needs; `live` is asked when this view has nothing open and the core may.
    pub fn ui(&self, live: &dyn Fn() -> ApplyUi) -> ApplyUi {
        if self.busy.is_some() {
            return ApplyUi::Working { submitting: false };
        }
        match &self.phase {
            Phase::Idle => live(),
            Phase::Preparing => ApplyUi::Working { submitting: false },
            Phase::Submitting => ApplyUi::Working { submitting: true },
            Phase::Review => match &self.review {
                Some(r) => ApplyUi::Review {
                    complete: r.complete,
                    dry_run: r.dry_run,
                    missing: self.missing(),
                },
                None => ApplyUi::Idle,
            },
            Phase::Done(_) => ApplyUi::Done,
            Phase::Handoff { .. } => ApplyUi::Handoff,
        }
    }

    /// A run that is already open in the core (switching tabs, selecting another application).
    pub fn adopt_live(&mut self, live: Live) {
        if !matches!(self.phase, Phase::Idle) {
            return;
        }
        match live {
            Live::Idle => {}
            Live::Review(r) => self.set_review(r),
            Live::Handoff { handoff, headed } => self.phase = Phase::Handoff { handoff, headed },
        }
    }

    pub fn push_progress(&mut self, line: String) {
        self.progress.push(line);
    }

    /// The answers the user typed since the last sync (a full snapshot).
    pub fn sync(&mut self, typed: HashMap<String, String>, remember: HashSet<String>) {
        if self.review.is_some() {
            self.typed = typed;
            self.remember = remember;
        }
    }

    /// Open and fill the form: `false` when that is already going on.
    pub fn begin_prepare(&mut self) -> bool {
        if matches!(self.phase, Phase::Preparing | Phase::Submitting) {
            return false;
        }
        self.phase = Phase::Preparing;
        self.error = None;
        self.review = None;
        self.typed.clear();
        self.remember.clear();
        self.progress.clear();
        true
    }

    /// The result of filling the form (first step or a next one).
    pub fn finish_prepare(&mut self, result: Result<Prepared, String>) {
        match result {
            Ok(Prepared::Review(r)) => self.set_review(r),
            Ok(Prepared::Handoff(h)) => {
                self.phase = Phase::Handoff {
                    handoff: h,
                    headed: false,
                }
            }
            Err(e) => {
                self.phase = if self.review.is_some() {
                    Phase::Review
                } else {
                    Phase::Idle
                };
                self.error = Some(e);
            }
        }
    }

    /// The explicit confirmation: submit, or continue to the next step when the form has more.
    pub fn begin_confirm(&mut self) -> Confirm {
        let Some(review) = self.review.clone() else {
            return Confirm::Ignore;
        };
        if !matches!(self.phase, Phase::Review) || (review.dry_run && review.complete) {
            return Confirm::Ignore;
        }
        if self.missing() > 0 {
            self.error = Some("Fill the required fields first.".into());
            return Confirm::Blocked;
        }
        let edits = edits_for(&review, &self.typed, &self.remember);
        self.error = None;
        self.progress.clear();
        if review.complete {
            self.phase = Phase::Submitting;
            Confirm::Submit(edits)
        } else {
            self.phase = Phase::Preparing;
            Confirm::Advance(edits)
        }
    }

    pub fn finish_submit(&mut self, result: Result<Submitted, String>) {
        match result {
            Ok(Submitted::Applied { proof }) => {
                self.phase = Phase::Done(Some(proof).filter(|p| !p.as_os_str().is_empty()));
                self.review = None;
            }
            Ok(Submitted::DryRun) => self.phase = Phase::Review,
            Ok(Submitted::Handoff(h)) => {
                self.phase = Phase::Handoff {
                    handoff: h,
                    headed: false,
                }
            }
            Err(e) => {
                self.phase = Phase::Review;
                self.error = Some(e);
            }
        }
    }

    /// Closing the browser and dropping the review.
    pub fn finish_discard(&mut self, result: Result<(), String>) {
        self.busy = None;
        match result {
            Ok(()) => self.reset(),
            Err(e) => self.error = Some(e),
        }
    }

    pub fn finish_open_visible(&mut self, result: Result<(), String>) {
        self.busy = None;
        match result {
            Ok(()) => {
                if let Phase::Handoff { headed, .. } = &mut self.phase {
                    *headed = true;
                }
            }
            Err(e) => self.error = Some(e),
        }
    }

    /// "Mark as applied / failed" after finishing by hand.
    pub fn finish_mark(&mut self, result: Result<(), String>) {
        self.busy = None;
        match result {
            Ok(()) => self.reset(),
            Err(e) => self.error = Some(e),
        }
    }
}

impl Default for ApplyView {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------- the registry

struct JobRun {
    label: String,
    stage: Option<Stage>,
}

#[derive(Default)]
struct AppRuntime {
    job: Option<JobRun>,
    /// The job slot is held by a change that must not run beside a job ("Dismiss"): no job starts.
    held: bool,
    /// Last failure of a job, shown until the next job starts or the user dismisses it.
    error: Option<String>,
    last_job: Option<Job>,
    /// What this session fed to `generate` for a Draft, so Generate can run it again.
    input: Option<JdInput>,
    /// Bumped when a job rewrote the CV files, so the CV tab reads them again.
    cv_epoch: u64,
    /// The CV does not parse or compile (as of the last render), so it must not be approved.
    cv_problem: Option<String>,
    apply: ApplyView,
}

#[derive(Default)]
struct Inner {
    apps: HashMap<i64, AppRuntime>,
    /// The "Dry run" box of the Apply tab (one for the window, not per application).
    dry_run: bool,
}

/// A progress step as the web app is told about it.
pub struct ProgressNote {
    pub label: String,
    /// The stage differs from the one before: stored data (the analysis renames the row) may have changed.
    pub stage_changed: bool,
}

/// What the Application screen needs of the running state.
pub struct JobView {
    pub label: Option<String>,
    pub error: Option<String>,
    pub cv_epoch: u64,
    pub cv_problem: Option<String>,
}

/// The job slot of one application, held until dropped ([`Applications::hold_slot`]).
pub struct HeldSlot {
    apps: Applications,
    id: i64,
}

impl Drop for HeldSlot {
    fn drop(&mut self) {
        if let Some(rt) = self.apps.lock().apps.get_mut(&self.id) {
            rt.held = false;
        }
    }
}

/// Shared handle on the in-memory state; cheap to clone into a task.
#[derive(Clone, Default)]
pub struct Applications(Arc<Mutex<Inner>>);

impl Applications {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    // ------------------------------------------------------------ pipeline jobs

    /// Start `job` for `id`: `false` when one is already running (nothing changes).
    pub fn begin_job(&self, id: i64, job: Job, input: Option<JdInput>) -> bool {
        let mut inner = self.lock();
        let rt = inner.apps.entry(id).or_default();
        if rt.job.is_some() || rt.held {
            return false;
        }
        rt.last_job = Some(job);
        rt.error = None;
        rt.job = Some(JobRun {
            label: "Starting".into(),
            stage: None,
        });
        if let Some(input) = input {
            rt.input = Some(input);
        }
        true
    }

    /// Hold the job slot of `id` while a change runs that must not run beside a generate / revise
    /// job ("Dismiss"): `None` when a job is running or the slot is held already. No job starts
    /// until the returned guard is dropped.
    pub fn hold_slot(&self, id: i64) -> Option<HeldSlot> {
        let mut inner = self.lock();
        let rt = inner.apps.entry(id).or_default();
        if rt.job.is_some() || rt.held {
            return None;
        }
        rt.held = true;
        Some(HeldSlot { apps: self.clone(), id })
    }

    /// Applications with a job running.
    pub fn busy_ids(&self) -> HashSet<i64> {
        self.lock()
            .apps
            .iter()
            .filter(|(_, rt)| rt.job.is_some())
            .map(|(id, _)| *id)
            .collect()
    }

    /// Keep what a Draft was generated from, so a failed Generate can run again.
    pub fn remember_input(&self, id: i64, input: JdInput) {
        self.lock().apps.entry(id).or_default().input = Some(input);
    }

    pub fn cached_input(&self, id: i64) -> Option<JdInput> {
        self.lock().apps.get(&id).and_then(|rt| rt.input.clone())
    }

    pub fn last_job(&self, id: i64) -> Option<Job> {
        self.lock().apps.get(&id).and_then(|rt| rt.last_job.clone())
    }

    /// A progress step of the running job.
    pub fn on_progress(&self, p: &Progress) -> ProgressNote {
        let label = progress_label(p);
        let mut inner = self.lock();
        let rt = inner.apps.entry(p.app_id).or_default();
        rt.error = None;
        let stage_changed = match &mut rt.job {
            Some(run) => {
                let changed = run.stage != Some(p.stage);
                run.label = label.clone();
                run.stage = Some(p.stage);
                changed
            }
            None => false,
        };
        ProgressNote {
            label,
            stage_changed,
        }
    }

    /// The job ended: the CV files may have changed, and a failure is kept for the header.
    pub fn finish_job(&self, id: i64, result: Result<(), String>) {
        let mut inner = self.lock();
        let rt = inner.apps.entry(id).or_default();
        rt.job = None;
        rt.cv_epoch += 1;
        rt.cv_problem = None;
        match result {
            Ok(()) => {
                rt.error = None;
                rt.input = None;
            }
            Err(message) => {
                rt.last_job.get_or_insert(Job::Generate);
                rt.error = Some(message);
            }
        }
    }

    /// How "Approve CV" ended: a failure is shown, and Retry approves again.
    pub fn approval_ended(&self, id: i64, error: Option<String>) {
        let mut inner = self.lock();
        let rt = inner.apps.entry(id).or_default();
        if error.is_some() {
            rt.last_job = Some(Job::Approve);
        }
        rt.error = error;
    }

    /// Hide the failure; `true` when there was one.
    pub fn dismiss_error(&self, id: i64) -> bool {
        self.lock()
            .apps
            .get_mut(&id)
            .is_some_and(|rt| rt.error.take().is_some())
    }

    pub fn set_cv_problem(&self, id: i64, problem: Option<String>) {
        self.lock().apps.entry(id).or_default().cv_problem = problem;
    }

    pub fn job_view(&self, id: i64) -> JobView {
        let inner = self.lock();
        match inner.apps.get(&id) {
            Some(rt) => JobView {
                label: rt.job.as_ref().map(|j| j.label.clone()),
                error: rt.error.clone(),
                cv_epoch: rt.cv_epoch,
                cv_problem: rt.cv_problem.clone(),
            },
            None => JobView {
                label: None,
                error: None,
                cv_epoch: 0,
                cv_problem: None,
            },
        }
    }

    // ------------------------------------------------------------ Apply tab

    /// Read or change the Apply tab's session of `id`.
    pub fn with_apply<R>(&self, id: i64, f: impl FnOnce(&mut ApplyView) -> R) -> R {
        let mut inner = self.lock();
        f(&mut inner.apps.entry(id).or_default().apply)
    }

    pub fn dry_run(&self) -> bool {
        self.lock().dry_run
    }

    pub fn set_dry_run(&self, on: bool) {
        self.lock().dry_run = on;
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::apply::{Ats, FieldKind, Row, Source};

    use super::*;

    fn progress(id: i64, stage: Stage, detail: &str) -> Progress {
        Progress {
            app_id: id,
            stage,
            detail: detail.into(),
        }
    }

    #[test]
    fn a_failed_approval_is_retried_as_an_approval() {
        let apps = Applications::default();
        apps.begin_job(1, Job::Generate, None);
        apps.finish_job(1, Ok(()));
        apps.approval_ended(1, Some("database is locked".into()));
        assert_eq!(
            apps.last_job(1),
            Some(Job::Approve),
            "Retry never regenerates the CV"
        );
        assert!(apps.dismiss_error(1));
        apps.approval_ended(1, None);
        assert!(
            !apps.dismiss_error(1),
            "a successful approval leaves no error"
        );
    }

    #[test]
    fn one_job_runs_per_application() {
        let apps = Applications::default();
        assert!(apps.begin_job(1, Job::Generate, None));
        assert!(
            !apps.begin_job(1, Job::Revise("shorter".into()), None),
            "a second job waits"
        );
        assert!(
            apps.begin_job(2, Job::Generate, None),
            "another application is free"
        );
        assert!(apps.busy_ids().contains(&1));
        assert_eq!(apps.busy_ids(), HashSet::from([1, 2]));
        assert_eq!(
            apps.last_job(1),
            Some(Job::Generate),
            "the refused job is not remembered"
        );

        apps.finish_job(1, Ok(()));
        assert!(!apps.busy_ids().contains(&1));
        assert!(apps.begin_job(1, Job::Revise("shorter".into()), None));
        assert_eq!(apps.last_job(1), Some(Job::Revise("shorter".into())));
    }

    #[test]
    fn a_held_slot_keeps_jobs_out_until_it_is_dropped() {
        let apps = Applications::default();
        assert!(apps.begin_job(1, Job::Generate, None));
        assert!(apps.hold_slot(1).is_none(), "not while a job runs");
        apps.finish_job(1, Ok(()));

        let held = apps.hold_slot(1).expect("a free slot");
        assert!(apps.hold_slot(1).is_none(), "held once");
        assert!(!apps.begin_job(1, Job::Generate, None), "no job starts while it is held");
        assert!(apps.begin_job(2, Job::Generate, None), "another application is free");
        assert!(apps.job_view(1).label.is_none(), "holding is not a job");
        drop(held);
        assert!(apps.begin_job(1, Job::Generate, None));
    }

    #[test]
    fn progress_moves_the_label_and_reports_a_new_stage() {
        let apps = Applications::default();
        apps.begin_job(1, Job::Generate, None);
        assert_eq!(apps.job_view(1).label.as_deref(), Some("Starting"));
        let first = apps.on_progress(&progress(1, Stage::Analyzing, ""));
        assert!(first.stage_changed);
        assert_eq!(first.label, "Analysing the job description");
        let again = apps.on_progress(&progress(1, Stage::Analyzing, "retrying"));
        assert!(!again.stage_changed, "a detail is not a new stage");
        assert_eq!(
            apps.job_view(1).label.as_deref(),
            Some("Analysing the job description: retrying")
        );
        // A step that arrives after the job ended does not make the application busy again.
        apps.finish_job(1, Ok(()));
        apps.on_progress(&progress(1, Stage::Done, ""));
        assert!(!apps.busy_ids().contains(&1));
    }

    #[test]
    fn a_failure_is_kept_until_the_next_job_or_a_dismissal() {
        let apps = Applications::default();
        apps.begin_job(1, Job::Generate, Some(JdInput::Text("pasted".into())));
        apps.finish_job(1, Err("boom".into()));
        let view = apps.job_view(1);
        assert_eq!(
            (view.label, view.error.as_deref(), view.cv_epoch),
            (None, Some("boom"), 1)
        );
        assert!(
            matches!(apps.cached_input(1), Some(JdInput::Text(t)) if t == "pasted"),
            "kept for Retry"
        );

        assert!(apps.dismiss_error(1));
        assert!(!apps.dismiss_error(1), "nothing left to dismiss");

        apps.begin_job(1, Job::Generate, None);
        apps.finish_job(1, Ok(()));
        assert!(
            apps.cached_input(1).is_none(),
            "a finished job needs no retry input"
        );
        assert_eq!(
            apps.job_view(1).cv_epoch,
            2,
            "every finished job says the CV files may have changed"
        );

        apps.begin_job(1, Job::Generate, None);
        apps.finish_job(1, Err("again".into()));
        apps.begin_job(1, Job::Generate, None);
        assert!(
            apps.job_view(1).error.is_none(),
            "a new job clears the failure"
        );
    }

    #[test]
    fn a_finished_job_clears_the_cv_problem() {
        let apps = Applications::default();
        apps.set_cv_problem(1, Some("typst failed".into()));
        assert_eq!(apps.job_view(1).cv_problem.as_deref(), Some("typst failed"));
        apps.begin_job(1, Job::Revise("shorter".into()), None);
        apps.finish_job(1, Ok(()));
        assert_eq!(apps.job_view(1).cv_problem, None, "the CV was rewritten");
    }

    // ------------------------------------------------------------ Apply tab

    fn row(id: &str, required: bool, value: &str) -> Row {
        Row {
            id: id.into(),
            step: 0,
            label: id.into(),
            kind: FieldKind::Text,
            required,
            options: Vec::new(),
            value: value.into(),
            source: Source::Profile,
            needs_input: false,
            warning: None,
            editable: true,
        }
    }

    fn review(complete: bool, dry_run: bool, rows: Vec<Row>) -> Review {
        Review {
            app_id: 1,
            ats: Ats::Greenhouse,
            url: "https://boards.greenhouse.io/acme/jobs/1".into(),
            dry_run,
            complete,
            rows,
            attachment: None,
            screenshots: Vec::new(),
            notes: Vec::new(),
            session: None,
        }
    }

    fn handoff() -> Handoff {
        Handoff {
            reason: "CAPTCHA".into(),
            captcha: true,
            screenshot: None,
        }
    }

    fn idle() -> ApplyUi {
        ApplyUi::Idle
    }

    #[test]
    fn the_primary_button_sees_the_apply_tab() {
        let mut v = ApplyView::new();
        assert_eq!(v.ui(&idle), ApplyUi::Idle);
        assert_eq!(
            v.ui(&|| ApplyUi::Handoff),
            ApplyUi::Handoff,
            "an idle view asks the core"
        );

        assert!(v.begin_prepare());
        assert!(!v.begin_prepare(), "the form is already being filled");
        assert_eq!(v.ui(&idle), ApplyUi::Working { submitting: false });

        v.finish_prepare(Ok(Prepared::Review(review(
            true,
            false,
            vec![row("a", true, "")],
        ))));
        assert_eq!(
            v.ui(&idle),
            ApplyUi::Review {
                complete: true,
                dry_run: false,
                missing: 1
            }
        );
        v.sync(
            HashMap::from([("a".to_string(), "x".to_string())]),
            HashSet::new(),
        );
        assert_eq!(
            v.ui(&idle),
            ApplyUi::Review {
                complete: true,
                dry_run: false,
                missing: 0
            }
        );

        v.busy = Some("Saving...");
        assert_eq!(v.ui(&idle), ApplyUi::Working { submitting: false });
        v.busy = None;
        v.phase = Phase::Submitting;
        assert_eq!(v.ui(&idle), ApplyUi::Working { submitting: true });
        v.phase = Phase::Done(None);
        assert_eq!(v.ui(&idle), ApplyUi::Done);
    }

    #[test]
    fn nothing_is_sent_while_required_fields_are_empty() {
        let mut v = ApplyView::new();
        v.begin_prepare();
        v.finish_prepare(Ok(Prepared::Review(review(
            true,
            false,
            vec![row("a", true, "")],
        ))));
        assert!(matches!(v.begin_confirm(), Confirm::Blocked));
        assert_eq!(v.error.as_deref(), Some("Fill the required fields first."));
        assert!(matches!(v.phase, Phase::Review), "the review stays open");

        v.sync(
            HashMap::from([("a".to_string(), "answer".to_string())]),
            HashSet::from(["a".to_string()]),
        );
        match v.begin_confirm() {
            Confirm::Submit(edits) => {
                assert_eq!(edits.len(), 1);
                assert_eq!(
                    (
                        edits[0].id.as_str(),
                        edits[0].value.as_str(),
                        edits[0].remember
                    ),
                    ("a", "answer", true)
                );
            }
            other => panic!("expected a submit, got {other:?}"),
        }
        assert!(matches!(v.phase, Phase::Submitting));
        assert!(v.error.is_none());
        assert!(
            matches!(v.begin_confirm(), Confirm::Ignore),
            "a second click while submitting does nothing"
        );
    }

    #[test]
    fn a_dry_run_is_never_submitted_and_more_steps_continue() {
        let mut v = ApplyView::new();
        v.begin_prepare();
        v.finish_prepare(Ok(Prepared::Review(review(
            true,
            true,
            vec![row("a", false, "x")],
        ))));
        assert!(matches!(v.begin_confirm(), Confirm::Ignore));
        assert!(matches!(v.phase, Phase::Review));

        v.finish_prepare(Ok(Prepared::Review(review(
            false,
            false,
            vec![row("a", false, "x")],
        ))));
        assert!(matches!(v.begin_confirm(), Confirm::Advance(_)));
        assert!(
            matches!(v.phase, Phase::Preparing),
            "the next step is being filled"
        );
    }

    #[test]
    fn the_outcome_of_a_submit_decides_what_the_tab_shows() {
        let open = || {
            let mut v = ApplyView::new();
            v.set_review(review(true, false, vec![row("a", false, "x")]));
            v.phase = Phase::Submitting;
            v
        };
        let mut v = open();
        v.finish_submit(Ok(Submitted::Applied {
            proof: PathBuf::from("/tmp/proof.png"),
        }));
        assert!(matches!(&v.phase, Phase::Done(Some(p)) if p.ends_with("proof.png")));
        assert!(v.review.is_none());

        let mut v = open();
        v.finish_submit(Ok(Submitted::Applied {
            proof: PathBuf::new(),
        }));
        assert!(
            matches!(v.phase, Phase::Done(None)),
            "an empty proof path is no proof"
        );

        let mut v = open();
        v.finish_submit(Ok(Submitted::DryRun));
        assert!(matches!(v.phase, Phase::Review));

        let mut v = open();
        v.finish_submit(Ok(Submitted::Handoff(handoff())));
        assert!(matches!(v.phase, Phase::Handoff { headed: false, .. }));

        let mut v = open();
        v.finish_submit(Err("the page changed".into()));
        assert!(
            matches!(v.phase, Phase::Review),
            "a failed submit goes back to the review"
        );
        assert_eq!(v.error.as_deref(), Some("the page changed"));
    }

    #[test]
    fn a_failure_while_filling_goes_back_to_where_the_tab_was() {
        let mut v = ApplyView::new();
        v.begin_prepare();
        v.finish_prepare(Err("No Chrome found".into()));
        assert!(matches!(v.phase, Phase::Idle));
        assert_eq!(v.error.as_deref(), Some("No Chrome found"));

        v.set_review(review(false, false, vec![row("a", false, "x")]));
        v.phase = Phase::Preparing;
        v.finish_prepare(Err("the next step failed".into()));
        assert!(
            matches!(v.phase, Phase::Review),
            "the review of the earlier step is still there"
        );
    }

    #[test]
    fn a_review_in_the_core_is_taken_over_once() {
        let mut v = ApplyView::new();
        v.adopt_live(Live::Idle);
        assert!(matches!(v.phase, Phase::Idle));
        v.adopt_live(Live::Review(review(
            true,
            false,
            vec![row("a", false, "x")],
        )));
        assert!(matches!(v.phase, Phase::Review));
        let generation = v.generation;
        v.adopt_live(Live::Handoff {
            handoff: handoff(),
            headed: true,
        });
        assert!(
            matches!(v.phase, Phase::Review),
            "an open view is not replaced"
        );
        assert_eq!(v.generation, generation);

        let mut v = ApplyView::new();
        v.adopt_live(Live::Handoff {
            handoff: handoff(),
            headed: true,
        });
        assert!(matches!(v.phase, Phase::Handoff { headed: true, .. }));
    }

    #[test]
    fn closing_or_finishing_starts_over_but_keeps_counting_reviews() {
        let mut v = ApplyView::new();
        v.set_review(review(true, false, vec![row("a", false, "x")]));
        let generation = v.generation;
        v.busy = Some("Closing the browser...");
        v.finish_discard(Ok(()));
        assert!(matches!(v.phase, Phase::Idle) && v.review.is_none() && v.busy.is_none());
        v.set_review(review(true, false, vec![row("a", false, "x")]));
        assert!(
            v.generation > generation,
            "a new review is never mistaken for the old one"
        );

        v.finish_discard(Err("could not close".into()));
        assert_eq!(v.error.as_deref(), Some("could not close"));
        assert!(
            matches!(v.phase, Phase::Review),
            "a failed close leaves the review open"
        );
    }

    #[test]
    fn the_visible_browser_is_remembered_in_the_handoff() {
        let mut v = ApplyView::new();
        v.phase = Phase::Handoff {
            handoff: handoff(),
            headed: false,
        };
        v.busy = Some("Opening the browser...");
        v.finish_open_visible(Ok(()));
        assert!(matches!(v.phase, Phase::Handoff { headed: true, .. }) && v.busy.is_none());
        v.finish_open_visible(Err("no display".into()));
        assert_eq!(v.error.as_deref(), Some("no display"));
    }

    #[test]
    fn typed_answers_are_kept_only_for_an_open_review() {
        let mut v = ApplyView::new();
        v.sync(
            HashMap::from([("a".to_string(), "x".to_string())]),
            HashSet::new(),
        );
        assert!(
            v.typed.is_empty(),
            "a late sync after the review closed is dropped"
        );
        v.set_review(review(true, false, vec![row("a", false, "")]));
        v.sync(
            HashMap::from([("a".to_string(), "x".to_string())]),
            HashSet::new(),
        );
        assert_eq!(v.typed.get("a").map(String::as_str), Some("x"));
    }

    #[test]
    fn the_dry_run_box_is_one_for_the_window() {
        let apps = Applications::default();
        assert!(!apps.dry_run());
        apps.set_dry_run(true);
        assert!(apps.dry_run());
    }
}
