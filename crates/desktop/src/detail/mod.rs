//! Right pane: header, tabs (JD · CV · Apply · Timeline) and the one primary action.

mod apply;
mod cv;
mod jd;
mod timeline;

use gpui_kit::component::button::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::apply as core_apply;
use jobpilot_core::health;
use jobpilot_core::store::{Application, Status};
use jobpilot_core::Core;

use crate::app::{status_color, AppState};
use crate::problem::{self, RetryFn};
use apply::{ApplyTab, ApplyUi};
use cv::CvTab;
use jd::JdTab;
use timeline::TimelineTab;

/// The company brief card, shared with the Job list's lead view.
pub(crate) use jd::brief as insight_brief;

#[derive(Clone, Copy, PartialEq, Eq)]
enum DetailTab {
    Jd,
    Cv,
    Apply,
    Timeline,
}

impl DetailTab {
    const ALL: [DetailTab; 4] = [DetailTab::Jd, DetailTab::Cv, DetailTab::Apply, DetailTab::Timeline];

    fn label(self) -> &'static str {
        match self {
            DetailTab::Jd => "JD",
            DetailTab::Cv => "CV",
            DetailTab::Apply => "Apply",
            DetailTab::Timeline => "Timeline",
        }
    }
}

/// The single primary button: its label follows the status and the live apply state
/// (Generate -> Approve CV -> Review & Submit -> Submit).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimaryAction {
    Generate,
    ApproveCv,
    ReviewAndSubmit,
    /// The form of the next step is open (multi-step forms): fill it, then continue. Nothing is sent yet.
    Continue,
    Submit,
}

impl PrimaryAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Generate => "Generate",
            Self::ApproveCv => "Approve CV",
            Self::ReviewAndSubmit => "Review & Submit",
            Self::Continue => "Continue",
            Self::Submit => "Submit",
        }
    }
}

/// What the primary button looks like right now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Primary {
    pub action: PrimaryAction,
    pub enabled: bool,
    /// Work is running: spinner on the button.
    pub loading: bool,
    /// Why the button is disabled, or what is left to do.
    pub hint: Option<String>,
    /// The Settings section that fixes what blocks the button, if there is one.
    pub settings: Option<&'static str>,
}

impl Primary {
    fn ready(action: PrimaryAction) -> Self {
        Self { action, enabled: true, loading: false, hint: None, settings: None }
    }

    fn blocked(action: PrimaryAction, hint: impl Into<String>) -> Self {
        Self { action, enabled: false, loading: false, hint: Some(hint.into()), settings: None }
    }

    fn working(action: PrimaryAction) -> Self {
        Self { action, enabled: false, loading: true, hint: None, settings: None }
    }
}

/// The one primary action for an application, from its status and the live state of the Apply tab.
/// `None` = read-only (already applied, or answered by the company).
///
/// An open or running apply flow wins over the status, so the button always matches what the Apply tab
/// shows. `Submit` stays disabled until every required field has a value; every button is disabled while
/// work runs. `blocker` is only asked when the form could be opened (no apply URL, no session, no Chrome).
pub fn primary_for(
    status: Status,
    has_cv: bool,
    cv_broken: bool,
    job_running: bool,
    apply: &ApplyUi,
    blocker: &dyn Fn() -> Option<Blocker>,
) -> Option<Primary> {
    match apply {
        ApplyUi::Working { submitting } => {
            let action = if *submitting { PrimaryAction::Submit } else { PrimaryAction::ReviewAndSubmit };
            return Some(Primary::working(action));
        }
        ApplyUi::Review { complete, dry_run, missing } => {
            let action = if *complete { PrimaryAction::Submit } else { PrimaryAction::Continue };
            return Some(if *complete && *dry_run {
                Primary::blocked(action, "Dry run: the form is filled but nothing is sent. Close the dry run to start over.")
            } else if *missing > 0 {
                Primary::blocked(action, format!("{missing} required field(s) still empty"))
            } else {
                Primary::ready(action)
            });
        }
        ApplyUi::Idle | ApplyUi::Handoff | ApplyUi::Done => {}
    }
    match status {
        Status::Applied | Status::Interview | Status::Rejected | Status::Offer => None,
        Status::Draft if !has_cv => Some(if job_running { Primary::working(PrimaryAction::Generate) } else { Primary::ready(PrimaryAction::Generate) }),
        Status::Draft => Some(if job_running {
            Primary::working(PrimaryAction::ApproveCv)
        } else if cv_broken {
            Primary::blocked(PrimaryAction::ApproveCv, "The CV has an error: fix it before approving")
        } else {
            Primary::ready(PrimaryAction::ApproveCv)
        }),
        Status::CvReady | Status::Failed | Status::Submitting => Some(if job_running {
            Primary::working(PrimaryAction::ReviewAndSubmit)
        } else {
            match blocker() {
                Some(b) => Primary { settings: b.settings, ..Primary::blocked(PrimaryAction::ReviewAndSubmit, b.message) },
                None => Primary::ready(PrimaryAction::ReviewAndSubmit),
            }
        }),
    }
}

/// After the application was sent (or while its confirmation shows), a plain "Next job" button
/// keeps the user moving: it opens the best lead left in the Job list.
pub fn next_job_offered(status: Status, apply: &ApplyUi) -> bool {
    matches!(apply, ApplyUi::Done)
        || matches!(status, Status::Applied | Status::Interview | Status::Rejected | Status::Offer)
}

/// Why the form cannot be opened, and which Settings section helps.
pub struct Blocker {
    pub message: String,
    pub settings: Option<&'static str>,
}

fn apply_blocker(core: &Core, app: &Application) -> Option<Blocker> {
    let info = core_apply::apply_info(core, app);
    Some(Blocker { message: info.blocker()?, settings: info.blocker_settings() })
}

pub struct Detail {
    state: Entity<AppState>,
    tab: DetailTab,
    jd: Entity<JdTab>,
    cv: Entity<CvTab>,
    apply: Entity<ApplyTab>,
    timeline: Entity<TimelineTab>,
}

impl Detail {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let cv = cx.new(|cx| CvTab::new(state.clone(), window, cx));
        // The Approve button depends on whether the CV compiles.
        cx.observe(&cv, |_, _, cx| cx.notify()).detach();
        Self {
            tab: DetailTab::Cv,
            jd: cx.new(|cx| JdTab::new(state.clone(), cx)),
            cv,
            apply: cx.new(|cx| ApplyTab::new(state.clone(), cx)),
            timeline: cx.new(|cx| TimelineTab::new(state.clone(), cx)),
            state,
        }
    }
}

impl Detail {
    /// Generate / Approve CV are handled here (P3). Review & Submit / Continue / Submit belong to the
    /// Apply tab (P5): switch to it and delegate.
    fn on_primary(&mut self, action: PrimaryAction, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.state.read(cx).selected else { return };
        match action {
            PrimaryAction::Generate => self.state.update(cx, |state, cx| state.run_generate(id, cx)),
            PrimaryAction::ApproveCv => self.state.update(cx, |state, cx| state.approve_cv(id, cx)),
            PrimaryAction::ReviewAndSubmit | PrimaryAction::Continue | PrimaryAction::Submit => {
                let was_on_apply = self.tab == DetailTab::Apply;
                self.tab = DetailTab::Apply;
                cx.notify();
                // Submitting is an explicit click on what the user is looking at: from another tab the
                // click only brings the open review into view.
                if action != PrimaryAction::ReviewAndSubmit && !was_on_apply {
                    return;
                }
                self.apply.update(cx, |tab, cx| tab.on_primary(action, window, cx));
            }
        }
    }
}

impl Render for Detail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (app, progress, error) = {
            let state = self.state.read(cx);
            let app = state.selected_app().cloned();
            let id = app.as_ref().map(|a| a.id);
            (
                app,
                id.and_then(|id| state.job_label(id)).map(str::to_string),
                id.and_then(|id| state.error(id)).map(str::to_string),
            )
        };
        let busy = progress.is_some();
        // A CV that does not compile (or parse) must not be approved.
        let cv_broken = self.cv.read(cx).has_error();
        let none_yet = self.state.read(cx).applications.is_empty();
        let theme = cx.theme();
        let Some(app) = app else {
            return div()
                .size_full()
                .v_flex()
                .items_center()
                .justify_center()
                .gap_2()
                .p_6()
                .text_center()
                .child(div().text_lg().font_semibold().child(if none_yet { "No applications yet" } else { "Nothing selected" }))
                .child(div().text_color(theme.muted_foreground).max_w(px(420.)).child(if none_yet {
                    "Open the Job list and paste the link of a job posting (or drop a .md, .txt, .html or .pdf file on the window). Press Prepare CV on the job: JobPilot writes a CV tailored to it from your profile, then helps you apply."
                } else {
                    "Select an application in the list, or prepare a CV for a job from the Job list."
                }))
                .into_any_element();
        };
        let core = self.state.read(cx).core.clone();
        let apply_ui = self.apply.read(cx).ui_state(app.id, cx);
        let primary = primary_for(
            app.status,
            app.cv_yaml_path().is_file(),
            cv_broken,
            busy,
            &apply_ui,
            &|| apply_blocker(&core, &app),
        );

        // Read-only (sent): the one primary is gone, "Next job" is a plain button in its place.
        let show_next = primary.is_none() && next_job_offered(app.status, &apply_ui);

        let content = match self.tab {
            DetailTab::Jd => self.jd.clone().into_any_element(),
            DetailTab::Cv => self.cv.clone().into_any_element(),
            DetailTab::Apply => self.apply.clone().into_any_element(),
            DetailTab::Timeline => self.timeline.clone().into_any_element(),
        };
        let selected = DetailTab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);

        div()
            .v_flex()
            .size_full()
            .gap_3()
            .p_6()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_2xl().font_semibold().child(format!("{} — {}", app.company, app.role)))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .when_some(progress, |d, label| {
                                d.child(Spinner::new().small().color(theme.primary))
                                    .child(div().text_sm().text_color(theme.muted_foreground).child(label))
                            })
                            .child(div().size_2().rounded_full().bg(status_color(app.status, theme)))
                            .child(div().text_sm().child(app.status.label())),
                    ),
            )
            .when_some(error, |d, message| {
                let id = app.id;
                let problem = health::classify(&message);
                let state = self.state.clone();
                let retry: RetryFn = {
                    let state = state.clone();
                    std::rc::Rc::new(move |_, cx| state.update(cx, |s, cx| s.retry(id, cx)))
                };
                let help = problem::help(&format!("job-error-{id}"), problem, &state, Some(retry), cx);
                d.child(
                    div()
                        .h_flex()
                        .items_start()
                        .gap_2()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.danger)
                        .child(
                            div()
                                .v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .child(
                                    div()
                                        .id("job-error")
                                        .max_h(px(96.))
                                        .overflow_y_scroll()
                                        .font_family("monospace")
                                        .text_xs()
                                        .text_color(theme.danger)
                                        .child(message),
                                )
                                .children(help),
                        )
                        .child(
                            Button::new("dismiss-error").ghost().compact().icon(IconName::Close).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.state.update(cx, |state, cx| state.dismiss_error(id, cx))
                                }),
                            ),
                        ),
                )
            })
            .child(
                TabBar::new("detail-tabs")
                    .underline()
                    .selected_index(selected)
                    .on_click(cx.listener(|this, ix: &usize, _, cx| {
                        if let Some(tab) = DetailTab::ALL.get(*ix) {
                            this.tab = *tab;
                            cx.notify();
                        }
                    }))
                    .children(DetailTab::ALL.iter().map(|t| Tab::new().label(t.label()))),
            )
            .child(div().flex_1().min_h_0().child(content))
            .when(show_next, |d| {
                d.child(div().h_flex().justify_end().child(
                    Button::new("next-job").outline().icon(IconName::Inbox).label("Next job").on_click(
                        cx.listener(|this, _, _, cx| this.state.update(cx, |state, cx| state.next_job(cx))),
                    ),
                ))
            })
            .when_some(primary, |d, primary| {
                let action = primary.action;
                let settings_help = primary.settings.map(|section| {
                    let state = self.state.clone();
                    Button::new("primary-settings")
                        .outline()
                        .compact()
                        .icon(IconName::Settings)
                        .label(format!("Open Settings > {section}"))
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.open_settings(Some(section), cx)))
                });
                d.child(
                    div()
                        .h_flex()
                        .items_center()
                        .justify_end()
                        .gap_3()
                        .when_some(primary.hint.clone(), |d, hint| {
                            d.child(div().flex_1().min_w_0().text_sm().text_color(theme.muted_foreground).child(hint))
                        })
                        .children(settings_help)
                        .child(
                            Button::new("primary-action")
                                .primary()
                                .label(action.label())
                                .loading(primary.loading)
                                .disabled(!primary.enabled)
                                .on_click(cx.listener(move |this, _, window, cx| this.on_primary(action, window, cx))),
                        ),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Option<Blocker> {
        None
    }

    fn idle(status: Status, has_cv: bool) -> Option<Primary> {
        primary_for(status, has_cv, false, false, &ApplyUi::Idle, &none)
    }

    #[::core::prelude::v1::test]
    fn the_primary_action_follows_the_status() {
        assert_eq!(idle(Status::Draft, false).unwrap().action, PrimaryAction::Generate);
        assert_eq!(idle(Status::Draft, true).unwrap().action, PrimaryAction::ApproveCv);
        assert_eq!(idle(Status::CvReady, true).unwrap().action, PrimaryAction::ReviewAndSubmit);
        assert_eq!(idle(Status::Failed, true).unwrap().action, PrimaryAction::ReviewAndSubmit);
        for status in [Status::Applied, Status::Interview, Status::Rejected, Status::Offer] {
            assert_eq!(idle(status, true), None, "{status:?} is read-only");
        }
    }

    #[::core::prelude::v1::test]
    fn next_job_is_offered_once_the_application_was_sent() {
        for status in [Status::Applied, Status::Interview, Status::Rejected, Status::Offer] {
            assert!(next_job_offered(status, &ApplyUi::Idle), "{status:?}");
            assert_eq!(idle(status, true), None, "it replaces the primary button, never joins it");
        }
        // Right after Submit, while the confirmation shows.
        assert!(next_job_offered(Status::Submitting, &ApplyUi::Done));
        for status in [Status::Draft, Status::CvReady, Status::Failed, Status::Submitting] {
            assert!(!next_job_offered(status, &ApplyUi::Idle), "{status:?} still has work to do");
            assert!(!next_job_offered(status, &ApplyUi::Handoff));
        }
    }

    #[::core::prelude::v1::test]
    fn an_open_review_decides_the_button_whatever_the_status() {
        let review = |complete, dry_run, missing| ApplyUi::Review { complete, dry_run, missing };
        // The demo state: a Draft without CV with a review open must not offer Generate.
        let p = primary_for(Status::Draft, false, false, false, &review(true, false, 0), &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::Submit, true));
        for status in [Status::CvReady, Status::Submitting, Status::Failed] {
            let p = primary_for(status, true, false, false, &review(true, false, 0), &none).unwrap();
            assert_eq!((p.action, p.enabled), (PrimaryAction::Submit, true), "{status:?}");
        }
        // Required fields empty: Submit stays disabled and says how many.
        let p = primary_for(Status::Submitting, true, false, false, &review(true, false, 2), &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::Submit, false));
        assert!(p.hint.unwrap().contains('2'));
        // A dry run can be reviewed but never submitted.
        let p = primary_for(Status::CvReady, true, false, false, &review(true, true, 0), &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::Submit, false));
        // More steps to go: Continue, enabled once the highlighted fields are filled.
        let p = primary_for(Status::Submitting, true, false, false, &review(false, false, 0), &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::Continue, true));
    }

    #[::core::prelude::v1::test]
    fn the_button_is_disabled_while_work_runs() {
        let working = ApplyUi::Working { submitting: false };
        let p = primary_for(Status::CvReady, true, false, false, &working, &none).unwrap();
        assert_eq!((p.action, p.enabled, p.loading), (PrimaryAction::ReviewAndSubmit, false, true));
        let submitting = ApplyUi::Working { submitting: true };
        let p = primary_for(Status::Submitting, true, false, false, &submitting, &none).unwrap();
        assert_eq!((p.action, p.enabled, p.loading), (PrimaryAction::Submit, false, true));
        // A generate / revise job.
        let p = primary_for(Status::Draft, false, false, true, &ApplyUi::Idle, &none).unwrap();
        assert_eq!((p.action, p.enabled, p.loading), (PrimaryAction::Generate, false, true));
        let p = primary_for(Status::CvReady, true, false, true, &ApplyUi::Idle, &none).unwrap();
        assert!(!p.enabled);
    }

    #[::core::prelude::v1::test]
    fn approve_waits_for_a_compiling_cv_and_review_waits_for_its_prerequisites() {
        let p = primary_for(Status::Draft, true, true, false, &ApplyUi::Idle, &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::ApproveCv, false));

        let blocker = || Some(Blocker { message: "No Chrome found".into(), settings: Some("Browser sessions") });
        let p = primary_for(Status::CvReady, true, false, false, &ApplyUi::Idle, &blocker).unwrap();
        assert_eq!((p.action, p.enabled, p.settings), (PrimaryAction::ReviewAndSubmit, false, Some("Browser sessions")));
        // After a handoff the same button retries.
        let p = primary_for(Status::Failed, true, false, false, &ApplyUi::Handoff, &none).unwrap();
        assert_eq!((p.action, p.enabled), (PrimaryAction::ReviewAndSubmit, true));
    }
}
