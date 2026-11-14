//! JD tab: the company brief (domain, business, facts, skills, interview process, sources), the job
//! description text, its source link and the key terms found in it.

use gpui_kit::component::text::TextView;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};
use jobpilot_core::pipeline::{read_jd_text, Analysis, Insight};
use jobpilot_core::store::Application;
use jobpilot_core::runtime;

use crate::app::AppState;
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

/// What the brief shows where a text did not say.
const NOT_FOUND: &str = "not found";

/// The company row: each fact with its label, `None` for what no text stated.
fn company_facts(c: &CompanyFacts) -> [(&'static str, Option<&str>); 5] {
    [
        ("Size", non_empty(&c.size)),
        ("Stage", non_empty(&c.stage)),
        ("Funding", non_empty(&c.funding)),
        ("HQ", non_empty(&c.hq)),
        ("Founded", non_empty(&c.founded)),
    ]
}

fn non_empty(s: &str) -> Option<&str> {
    Some(s).filter(|s| !s.is_empty())
}

/// The interview stages in order, `None` when the process was not found.
fn interview_steps(i: &Interview) -> Option<String> {
    (i.found && !i.stages.is_empty()).then(|| i.stages.join(" \u{2192} "))
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

/// The company brief: domain, business, company facts, skills, interview process and sources, with
/// "not found" for what the texts did not say.
fn brief(insight: &Insight, cx: &App) -> Div {
    let p = ui::palette(cx);
    let (muted, link) = (p.muted, p.accent);
    let not_found = || div().text_color(muted).child(NOT_FOUND).into_any_element();
    let text = |value: &str| match non_empty(value) {
        Some(v) => div().child(v.to_string()).into_any_element(),
        None => not_found(),
    };
    let row = |label: &'static str, body: AnyElement| {
        div()
            .h_flex()
            .items_start()
            .gap_2()
            .text_size(px(ui::tokens::TEXT_BODY))
            .text_color(p.ink)
            .child(div().w(px(84.)).flex_shrink_0().pt_0p5().child(ui::caption(label, cx)))
            .child(div().flex_1().min_w_0().child(body))
    };

    let facts = div().h_flex().flex_wrap().gap_x_4().children(company_facts(&insight.company).into_iter().map(
        |(label, value)| {
            let value = match value {
                Some(v) => div().child(v.to_string()),
                None => div().text_color(muted).child(NOT_FOUND),
            };
            div().h_flex().gap_1().child(div().text_color(muted).child(label)).child(value)
        },
    ));

    let skills = &insight.skills;
    let skills = if skills.required.is_empty() && skills.preferred.is_empty() {
        not_found()
    } else {
        div()
            .v_flex()
            .gap_1p5()
            .when(!skills.required.is_empty(), |d| d.child(tag_row("Required", &skills.required, true, cx)))
            .when(!skills.preferred.is_empty(), |d| d.child(tag_row("Preferred", &skills.preferred, false, cx)))
            .into_any_element()
    };

    let interview = &insight.interview;
    let process = match (interview_steps(interview), non_empty(&interview.notes).filter(|_| interview.found)) {
        (None, None) => not_found(),
        (steps, notes) => div()
            .v_flex()
            .gap_0p5()
            .children(steps.map(|s| div().child(s)))
            .children(notes.map(|n| div().text_color(muted).child(n.to_string())))
            .into_any_element(),
    };

    let sources = insight.sources.iter().filter(|u| is_link(u)).enumerate().map(|(i, url)| {
        let target = url.clone();
        div()
            .id(SharedString::from(format!("insight-source-{i}")))
            .text_color(link)
            .underline()
            .cursor_pointer()
            .truncate()
            .child(url.clone())
            .on_click(move |_, _, cx| cx.open_url(&target))
    });
    let sources: Vec<_> = sources.collect();

    ui::card(cx)
        .gap_2()
        .child(ui::heading("Company brief", Level::H2, cx))
        .child(row("Domain", text(&insight.domain)))
        .child(row("Business", text(&insight.business_summary)))
        .child(row("Company", facts.into_any_element()))
        .child(row("Skills", skills))
        .child(row("Interview", process))
        .when(!sources.is_empty(), |d| {
            d.child(row("Sources", div().v_flex().gap_0p5().children(sources).into_any_element()))
        })
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
    use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};

    use super::{company_facts, interview_steps, is_link, jd_markdown};

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

    #[test]
    fn unknown_company_facts_are_none_so_the_view_says_not_found() {
        let facts = CompanyFacts { hq: "Singapore".into(), founded: "2015".into(), ..CompanyFacts::default() };
        let shown: Vec<_> = company_facts(&facts).into_iter().collect();
        assert_eq!(
            shown,
            [("Size", None), ("Stage", None), ("Funding", None), ("HQ", Some("Singapore")), ("Founded", Some("2015"))]
        );
        assert!(company_facts(&CompanyFacts::default()).iter().all(|(_, v)| v.is_none()));
    }

    #[test]
    fn interview_stages_read_in_order_and_only_when_found() {
        let found = Interview { stages: vec!["Call".into(), "Onsite".into()], notes: String::new(), found: true };
        assert_eq!(interview_steps(&found).as_deref(), Some("Call \u{2192} Onsite"));
        assert_eq!(interview_steps(&Interview { found: false, ..found.clone() }), None);
        assert_eq!(interview_steps(&Interview { stages: vec![], notes: "Three rounds.".into(), found: true }), None);
    }
}
