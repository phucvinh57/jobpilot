//! Right pane: header, tabs (JD · CV · Apply · Timeline) and the one primary action.

mod apply;
mod cv;
mod jd;
mod timeline;

use gpui_kit::component::button::*;
use gpui_kit::component::tab::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::store::{Application, Status};

use crate::app::{status_color, AppState};
use apply::ApplyTab;
use cv::CvTab;
use jd::JdTab;
use timeline::TimelineTab;

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

/// The single primary button; its label follows the application's status.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimaryAction {
    Generate,
    ApproveCv,
    ReviewAndSubmit,
    Submit,
}

impl PrimaryAction {
    /// `None` = read-only (already applied, or answered by the company).
    pub fn for_application(app: &Application) -> Option<Self> {
        match app.status {
            Status::Draft if app.cv_yaml_path().is_file() => Some(Self::ApproveCv),
            Status::Draft => Some(Self::Generate),
            Status::CvReady | Status::Failed => Some(Self::ReviewAndSubmit),
            Status::Submitting => Some(Self::Submit),
            Status::Applied | Status::Interview | Status::Rejected | Status::Offer => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Generate => "Generate",
            Self::ApproveCv => "Approve CV",
            Self::ReviewAndSubmit => "Review & Submit",
            Self::Submit => "Submit",
        }
    }
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
        Self {
            tab: DetailTab::Cv,
            jd: cx.new(|cx| JdTab::new(state.clone(), cx)),
            cv: cx.new(|cx| CvTab::new(state.clone(), window, cx)),
            apply: cx.new(|cx| ApplyTab::new(state.clone(), cx)),
            timeline: cx.new(|cx| TimelineTab::new(state.clone(), cx)),
            state,
        }
    }
}

impl Detail {
    /// Generate / Approve CV are handled here (P3). Review & Submit / Submit belong to the
    /// Apply tab (P5): switch to it and delegate.
    fn on_primary(&mut self, action: PrimaryAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            PrimaryAction::Generate | PrimaryAction::ApproveCv => {} // P3
            PrimaryAction::ReviewAndSubmit | PrimaryAction::Submit => {
                self.tab = DetailTab::Apply;
                self.apply.update(cx, |tab, cx| tab.on_primary(action, window, cx));
                cx.notify();
            }
        }
    }
}

impl Render for Detail {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.state.read(cx).selected_app().cloned();
        let theme = cx.theme();
        let Some(app) = app else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.muted_foreground)
                .child("Select an application, or paste a JD link")
                .into_any_element();
        };

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
            .p_4()
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_semibold().child(format!("{} — {}", app.company, app.role)))
                    .child(
                        div()
                            .h_flex()
                            .items_center()
                            .gap_2()
                            .child(div().size_2().rounded_full().bg(status_color(app.status, theme)))
                            .child(div().text_sm().child(app.status.label())),
                    ),
            )
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
            .when_some(PrimaryAction::for_application(&app), |d, action| {
                d.child(
                    div().h_flex().justify_end().child(
                        Button::new("primary-action").primary().label(action.label()).on_click(cx.listener(
                            move |this, _, window, cx| this.on_primary(action, window, cx),
                        )),
                    ),
                )
            })
            .into_any_element()
    }
}
