//! The company brief, drawn the same way wherever it is shown: "The company" (domain, what it does,
//! the facts, the pages it comes from) and "Interview process" (the stages in order, then notes).
//! The Today screen shows them under a lead, the Application screen's JD tab under the posting.
//!
//! The helpers on top are pure and tested; the builders below them only draw. Nothing here reads
//! the database: the data is the stored [`Insight`].

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};
use jobpilot_core::pipeline::Insight;

use crate::queue::logic::is_web_link;
use crate::ui::{self, Tone};

// ----------------------------------------------------------- facts

/// The company facts the texts gave, as one line (`Series B · 80 people · HQ Berlin`); `None` when
/// they gave none.
pub(super) fn facts_line(c: &CompanyFacts) -> Option<String> {
    let part = |label: &str, value: &str| {
        let value = value.trim();
        (!value.is_empty()).then(|| if label.is_empty() { value.to_string() } else { format!("{label} {value}") })
    };
    let parts: Vec<String> = [
        part("", &c.stage),
        part("", &c.size),
        part("Funding", &c.funding),
        part("HQ", &c.hq),
        part("Founded", &c.founded),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
}

/// The interview stages in order; empty when the process was not found.
pub(super) fn interview_steps(i: &Interview) -> Vec<String> {
    if !i.found {
        return Vec::new();
    }
    i.stages.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

// ----------------------------------------------------------- drawing

/// What a brief section says where the texts did not.
pub(crate) const NOT_FOUND: &str = "Not found in the posting or the company pages.";

/// A small upper-case heading of a brief section.
pub(crate) fn section_label(text: &str, cx: &App) -> Div {
    div().text_size(px(ui::tokens::TEXT_SMALL)).font_semibold().text_color(ui::palette(cx).muted).child(text.to_uppercase())
}

/// "The company": domain, what it does, the facts, and the pages the brief comes from.
pub(crate) fn company_block(insight: &Insight, cx: &App) -> Div {
    let p = ui::palette(cx);
    let facts = facts_line(&insight.company);
    let domain = insight.domain.trim();
    let summary = insight.business_summary.trim();
    let known = !domain.is_empty() || !summary.is_empty() || facts.is_some();
    let links: Vec<Stateful<Div>> = insight
        .sources
        .iter()
        .filter(|u| is_web_link(u))
        .enumerate()
        .map(|(i, url)| {
            let target = url.clone();
            div()
                .id(SharedString::from(format!("insight-source-{i}")))
                .role(Role::Link)
                .aria_label(format!("Source: {url}"))
                .truncate()
                .text_size(px(ui::tokens::TEXT_SMALL))
                .text_color(p.accent)
                .underline()
                .cursor_pointer()
                .child(url.clone())
                .on_click(move |_, _, cx| cx.open_url(&target))
        })
        .collect();
    div()
        .v_flex()
        .gap_2()
        .child(section_label("The company", cx))
        .when(!domain.is_empty(), |d| d.child(div().h_flex().child(ui::tag(domain.to_string(), Tone::Accent, cx))))
        .when(!summary.is_empty(), |d| {
            d.child(div().text_size(px(ui::tokens::TEXT_BODY)).text_color(p.ink).child(summary.to_string()))
        })
        .children(facts.map(|f| ui::caption(f, cx)))
        .when(!known, |d| d.child(ui::caption(NOT_FOUND, cx)))
        .when(!links.is_empty(), |d| d.child(div().v_flex().gap_0p5().children(links)))
}

/// "Interview process": the stages in order, then any notes.
pub(crate) fn interview_block(insight: &Insight, cx: &App) -> Div {
    let p = ui::palette(cx);
    let steps = interview_steps(&insight.interview);
    let notes = Some(insight.interview.notes.trim()).filter(|n| insight.interview.found && !n.is_empty());
    div()
        .v_flex()
        .gap_2()
        .child(section_label("Interview process", cx))
        .when(!steps.is_empty(), |d| {
            d.child(
                div()
                    .id("queue-interview")
                    .role(Role::List)
                    .aria_label("Interview stages")
                    .v_flex()
                    .gap_1()
                    .children(steps.iter().enumerate().map(|(i, step)| {
                        div()
                            .id(("queue-interview-step", i))
                            .role(Role::ListItem)
                            .aria_label(format!("Step {}: {step}", i + 1))
                            .h_flex()
                            .items_start()
                            .gap_2()
                            .text_size(px(ui::tokens::TEXT_BODY))
                            .child(div().w(px(18.0)).flex_shrink_0().text_color(p.subtle).child(format!("{}.", i + 1)))
                            .child(div().flex_1().min_w_0().text_color(p.ink).child(step.clone()))
                    })),
            )
        })
        .children(notes.map(|n| ui::caption(n.to_string(), cx)))
        .when(steps.is_empty() && notes.is_none(), |d| d.child(ui::caption(NOT_FOUND, cx)))
}

#[cfg(test)]
mod tests {
    use jobpilot_core::pipeline::insight::{CompanyFacts, Interview};

    use super::{facts_line, interview_steps};

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn facts_read_as_one_line() {
        let facts = CompanyFacts { stage: "Series B".into(), size: "80 people".into(), hq: "Berlin".into(), ..CompanyFacts::default() };
        assert_eq!(facts_line(&facts).as_deref(), Some("Series B \u{b7} 80 people \u{b7} HQ Berlin"));
        assert_eq!(facts_line(&CompanyFacts::default()), None);
        let blank = CompanyFacts { size: "  ".into(), ..CompanyFacts::default() };
        assert_eq!(facts_line(&blank), None);
    }

    #[test]
    fn interview_steps_need_a_found_process() {
        let found = Interview { stages: names(&["Recruiter call", " ", "Take-home"]), notes: String::new(), found: true };
        assert_eq!(interview_steps(&found), names(&["Recruiter call", "Take-home"]));
        let not_found = Interview { stages: names(&["Stale"]), notes: String::new(), found: false };
        assert!(interview_steps(&not_found).is_empty());
    }
}
