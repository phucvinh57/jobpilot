//! JD tab: the company brief (the company, skills, interview process; drawn by `crate::brief`), the job
//! description text, its source link and the key terms found in it.

use gpui_kit::component::text::TextView;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::pipeline::{read_jd_text, Analysis, Insight};
use jobpilot_core::store::Application;
use jobpilot_core::runtime;

use crate::app::AppState;
use crate::brief::{company_block, interview_block, section_label, NOT_FOUND};
use crate::ui::{self, Level, Tone};

/// What the tab shows for one application.
#[derive(Default)]
struct JdView {
    /// `None` = no JD stored (yet).
    text: Option<String>,
    analysis: Option<Analysis>,
    /// The company brief; `None` = not researched (yet).
    insight: Option<Insight>,
}

pub struct JdTab {
    state: Entity<AppState>,
    /// The application version `view` was loaded for: id and `updated_at` (generate saves it).
    loaded: Option<(i64, DateTime<Utc>)>,
    view: JdView,
}

impl JdTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state, loaded: None, view: JdView::default() }
    }

    /// Read `jd.md` off the UI thread; the analysis is already in the row.
    fn load(&mut self, app: Application, cx: &mut Context<Self>) {
        self.loaded = Some((app.id, app.updated_at));
        self.view = JdView {
            text: None,
            analysis: app.analysis_json.as_deref().and_then(|j| serde_json::from_str(j).ok()),
            insight: app.insight_json.as_deref().and_then(Insight::from_json),
        };
        let key = (app.id, app.updated_at);
        let rx = runtime::spawn(async move { read_jd_text(&app).ok() });
        cx.spawn(async move |this, cx| {
            let text = rx.await.ok().flatten();
            this.update(cx, |this, cx| {
                // Ignore a late answer for an application that is no longer shown.
                if this.loaded == Some(key) {
                    this.view.text = text;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// `jd.md` text keeps its line structure when rendered as markdown: a single newline becomes a
/// hard break instead of collapsing into a space.
pub fn jd_markdown(text: &str) -> String {
    text.lines()
        .map(|line| if line.trim().is_empty() { String::new() } else { format!("{}  ", line.trim_end()) })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_link(source: &str) -> bool {
    let lower = source.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// A row of tags with a label in front: skills. `must` = required (accent), else neutral.
fn tag_row(label: &'static str, items: &[String], must: bool, cx: &App) -> Div {
    let tone = if must { Tone::Accent } else { Tone::Neutral };
    div()
        .h_flex()
        .flex_wrap()
        .items_center()
        .gap_1p5()
        .child(div().w(px(84.)).flex_shrink_0().child(ui::caption(label, cx)))
        .children(items.iter().map(|item| ui::tag(item.clone(), tone, cx)))
}

/// The company brief: the company and the interview process (the blocks Today shows too), and the
/// skills the posting asks for, with "not found" for what the texts did not say.
fn brief(insight: &Insight, cx: &App) -> Div {
    let skills = &insight.skills;
    let skills = div()
        .v_flex()
        .gap_1p5()
        .child(section_label("Skills", cx))
        .when(skills.required.is_empty() && skills.preferred.is_empty(), |d| d.child(ui::caption(NOT_FOUND, cx)))
        .when(!skills.required.is_empty(), |d| d.child(tag_row("Required", &skills.required, true, cx)))
        .when(!skills.preferred.is_empty(), |d| d.child(tag_row("Preferred", &skills.preferred, false, cx)));
    ui::card(cx)
        .gap_3()
        .child(ui::heading("Company brief", Level::H2, cx))
        .child(company_block(insight, cx))
        .child(skills)
        .child(interview_block(insight, cx))
}

impl Render for JdTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.state.read(cx).selected_app().cloned();
        if let Some(app) = &app {
            if self.loaded != Some((app.id, app.updated_at)) {
                self.load(app.clone(), cx);
            }
        }
        let p = ui::palette(cx);
        let source = app.as_ref().and_then(|a| a.source.clone());

        let brief_view = self.view.insight.as_ref().map(|i| brief(i, cx));

        // The posting text, in a card of its own: it scrolls inside the card.
        let posting = ui::card(cx)
            .flex_1()
            .min_h(px(260.))
            .gap_2()
            .child(ui::heading("Job description", Level::H2, cx))
            .child(match &self.view.text {
                Some(text) => div()
                    .flex_1()
                    .min_h_0()
                    .child(TextView::markdown("jd-text", jd_markdown(text)).selectable(true).scrollable(true))
                    .into_any_element(),
                None => div().text_color(p.muted).child("No JD text yet.").into_any_element(),
            });

        div()
            .id("jd-tab")
            .v_flex()
            .size_full()
            .gap_3()
            .overflow_y_scroll()
            .when_some(source, |d, src| {
                if is_link(&src) {
                    let url = src.clone();
                    d.child(
                        div()
                            .id("jd-source")
                            .text_size(px(ui::tokens::TEXT_BODY))
                            .text_color(p.accent)
                            .underline()
                            .cursor_pointer()
                            .truncate()
                            .child(src)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                } else {
                    d.child(div().text_size(px(ui::tokens::TEXT_BODY)).text_color(p.muted).truncate().child(src))
                }
            })
            // The brief lists the skills too, so the plain tags are for jobs without one.
            .when_some(brief_view, |d, brief| d.child(brief))
            .when_some(
                self.view
                    .analysis
                    .as_ref()
                    .filter(|a| self.view.insight.is_none() && !(a.must_have.is_empty() && a.nice_to_have.is_empty())),
                |d, a| {
                    d.child(
                        ui::card(cx)
                            .gap_2()
                            .child(ui::heading("Key requirements", Level::H2, cx))
                            .when(!a.must_have.is_empty(), |d| d.child(tag_row("Must have", &a.must_have, true, cx)))
                            .when(!a.nice_to_have.is_empty(), |d| {
                                d.child(tag_row("Nice to have", &a.nice_to_have, false, cx))
                            }),
                    )
                },
            )
            .child(posting)
    }
}

#[cfg(test)]
mod tests {
    use super::{is_link, jd_markdown};

    #[test]
    fn line_breaks_survive_markdown() {
        assert_eq!(jd_markdown("Title\n* one\n* two\n\nBody  \n"), "Title  \n* one  \n* two  \n\nBody  ");
        assert_eq!(jd_markdown(""), "");
    }

    #[test]
    fn only_web_sources_are_links() {
        assert!(is_link("https://acme.test/jobs/1"));
        assert!(is_link("HTTP://acme.test"));
        assert!(!is_link("/home/me/jd.pdf"));
    }
}
