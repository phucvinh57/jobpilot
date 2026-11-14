//! The open lead's brief on the Today screen: the score breakdown, the skills the posting asks for
//! (matched against the profile), the company and the interview process.
//!
//! The helpers on top are pure and tested; the builders below them only draw. Nothing here reads
//! the database: the data is the lead's stored score and insight, and the profile's skill names
//! that the view loads once per refresh (see `Core::profile_skills`).

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::discover::queue::{criteria_rows, QueueItem};
use jobpilot_core::pipeline::insight::{CompanyFacts, Interview, Skills};
use jobpilot_core::pipeline::Insight;

use super::logic::is_web_link;
use crate::ui::{self, ChipKind, Tone};

/// `skills` -> `Skills`.
pub(super) fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}

/// `text` cut to `max` characters, with an ellipsis when something was cut (a long link in a banner).
pub(super) fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{head}\u{2026}")
}

// ------------------------------------------------------------ skills

/// The skills of a posting, split by whether the profile has them.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct SkillSplit {
    /// Asked for and in the profile.
    pub matched: Vec<String>,
    /// Asked for and not in the profile.
    pub gaps: Vec<String>,
}


/// Lower case, split at spaces and the usual list separators; `+`, `#` and `.` stay (`c++`, `c#`, `node.js`).
fn tokens(skill: &str) -> Vec<String> {
    skill
        .to_lowercase()
        .split(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '/' | '(' | ')' | '&'))
        .map(|t| t.trim_matches('.').to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// The same skill: equal names, or one name is a whole word of the other (`Kubernetes` and
/// `Kubernetes (EKS)`). `Java` is not `JavaScript`, `C` is not `C++`.
fn same_skill(a: &str, b: &str) -> bool {
    let (ta, tb) = (tokens(a), tokens(b));
    if ta.is_empty() || tb.is_empty() {
        return false;
    }
    ta == tb || (ta.len() == 1 && tb.contains(&ta[0])) || (tb.len() == 1 && ta.contains(&tb[0]))
}

/// The posting's skills (required first, then preferred, each once) split by the profile's.
pub(super) fn split_skills(wanted: &Skills, profile: &[String]) -> SkillSplit {
    let mut seen: Vec<String> = Vec::new();
    let mut split = SkillSplit::default();
    for skill in wanted.required.iter().chain(&wanted.preferred) {
        let key = skill.trim().to_lowercase();
        if key.is_empty() || seen.contains(&key) {
            continue;
        }
        seen.push(key);
        let name = skill.trim().to_string();
        if profile.iter().any(|have| same_skill(&name, have)) {
            split.matched.push(name);
        } else {
            split.gaps.push(name);
        }
    }
    split
}

// ----------------------------------------------------------- company

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

const NOT_FOUND: &str = "Not found in the posting or the company pages.";

/// A small upper-case heading of a brief section.
pub(super) fn section_label(text: &str, cx: &App) -> Div {
    div().text_size(px(ui::tokens::TEXT_SMALL)).font_semibold().text_color(ui::palette(cx).muted).child(text.to_uppercase())
}

/// "Why it fits": one bar per criterion with the reason under it. `None` when the lead was not scored.
pub(super) fn breakdown(item: &QueueItem, cx: &App) -> Option<Stateful<Div>> {
    let scored = item.scored.as_ref()?;
    let p = ui::palette(cx);
    let rows = criteria_rows(scored).into_iter().map(|r| {
        let label = format!("{} {}%", capitalize(r.criterion), r.weight);
        let (bar, reason) = if r.detail.known {
            (ui::fit_bar(&label, f64::from(r.detail.score), cx), r.detail.reason.clone())
        } else {
            let row = div()
                .h_flex()
                .items_center()
                .gap_3()
                .child(div().w(px(120.0)).flex_shrink_0().truncate().text_size(px(ui::tokens::TEXT_SMALL)).text_color(p.muted).child(label))
                .child(div().text_size(px(ui::tokens::TEXT_SMALL)).text_color(p.subtle).child("Not judged"));
            (row, "The posting or your preferences say nothing about it.".to_string())
        };
        div().v_flex().gap_0p5().child(bar).when(!reason.trim().is_empty(), |d| {
            d.child(div().text_size(px(ui::tokens::TEXT_SMALL)).text_color(p.muted).child(reason))
        })
    });
    Some(
        div()
            .id("queue-breakdown")
            .role(Role::Group)
            .aria_label("Score breakdown")
            .v_flex()
            .gap_2p5()
            .child(section_label("Why it fits", cx))
            .children(rows)
            .child(ui::caption("Weighted by your preferences.", cx)),
    )
}

/// The skills the posting asks for: chips for those the profile has, dashed chips for the others.
/// Without a readable profile they are plain tags. `None` when the brief lists no skills.
pub(super) fn skills_block(insight: &Insight, profile: &[String], cx: &App) -> Option<Stateful<Div>> {
    let wanted = &insight.skills;
    if wanted.required.is_empty() && wanted.preferred.is_empty() {
        return None;
    }
    let judged = !profile.is_empty();
    let split = split_skills(wanted, profile);
    let summary = if judged {
        format!("Skills the posting asks for. In your profile: {}. Not in your profile: {}.", list_or_none(&split.matched), list_or_none(&split.gaps))
    } else {
        format!("Skills the posting asks for: {}.", list_or_none(&split.gaps))
    };
    let chips: Vec<AnyElement> = if judged {
        split
            .matched
            .iter()
            .map(|s| ui::chip(s.clone(), ChipKind::Match, cx).into_any_element())
            .chain(split.gaps.iter().map(|s| ui::chip(s.clone(), ChipKind::Gap, cx).into_any_element()))
            .collect()
    } else {
        split.gaps.iter().map(|s| ui::tag(s.clone(), Tone::Neutral, cx).into_any_element()).collect()
    };
    Some(
        div()
            .id("queue-skills")
            .role(Role::Group)
            .aria_label(summary)
            .v_flex()
            .gap_2p5()
            .child(section_label("Skills", cx))
            .child(div().h_flex().flex_wrap().gap_1p5().children(chips))
            .child(ui::caption(
                if judged { "Blue: in your profile. Dashed: asked for, not in your profile." } else { "What the posting asks for." },
                cx,
            )),
    )
}

fn list_or_none(items: &[String]) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        items.join(", ")
    }
}

/// "The company": domain, what it does, the facts, and the pages the brief comes from.
pub(super) fn company_block(insight: &Insight, cx: &App) -> Div {
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
pub(super) fn interview_block(insight: &Insight, cx: &App) -> Div {
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
    use jobpilot_core::pipeline::insight::{CompanyFacts, Interview, Skills};

    use super::{capitalize, facts_line, interview_steps, shorten, split_skills, SkillSplit};

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn skills_are_split_by_the_profile() {
        let wanted = Skills { required: names(&["Rust", "PostgreSQL", "Kafka"]), preferred: names(&["kubernetes", "rust"]) };
        let profile = names(&["Rust", "PostgreSQL", "Kubernetes (EKS)"]);
        assert_eq!(
            split_skills(&wanted, &profile),
            SkillSplit { matched: names(&["Rust", "PostgreSQL", "kubernetes"]), gaps: names(&["Kafka"]) }
        );
    }

    #[test]
    fn a_skill_counts_once_and_blanks_are_ignored() {
        let wanted = Skills { required: names(&["Go", " go ", "", "  "]), preferred: names(&["GO"]) };
        let split = split_skills(&wanted, &[]);
        assert_eq!(split.gaps, names(&["Go"]));
        assert!(split.matched.is_empty());
    }

    #[test]
    fn similar_names_are_not_the_same_skill() {
        let wanted = Skills { required: names(&["Java", "C", "SQL"]), preferred: vec![] };
        let profile = names(&["JavaScript", "C++", "PostgreSQL"]);
        let split = split_skills(&wanted, &profile);
        assert!(split.matched.is_empty());
        assert_eq!(split.gaps, names(&["Java", "C", "SQL"]));
    }

    #[test]
    fn a_whole_word_of_a_longer_name_matches() {
        let wanted = Skills { required: names(&["REST", "Terraform"]), preferred: vec![] };
        let profile = names(&["REST APIs", "Terraform / OpenTofu"]);
        assert_eq!(split_skills(&wanted, &profile).matched, names(&["REST", "Terraform"]));
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

    #[test]
    fn long_text_is_cut_with_an_ellipsis() {
        assert_eq!(shorten("short", 10), "short");
        assert_eq!(shorten("abcdefghij", 5), "abcd\u{2026}");
        assert_eq!(shorten("abcde", 5), "abcde");
    }

    #[test]
    fn criteria_are_capitalised() {
        assert_eq!(capitalize("skills"), "Skills");
        assert_eq!(capitalize(""), "");
    }
}
