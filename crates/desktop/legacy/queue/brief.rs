//! The open lead's brief on the Today screen: the score breakdown and the skills the posting asks
//! for (matched against the profile). The company and the interview process are drawn by the shared
//! [`crate::brief`] module.
//!
//! The helpers on top are pure and tested; the builders below them only draw. Nothing here reads
//! the database: the data is the lead's stored score and insight, and the profile's skill names
//! that the view caches (see `Core::profile_skills`).

use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::discover::queue::{criteria_rows, QueueItem};
use jobpilot_core::pipeline::insight::Skills;
use jobpilot_core::pipeline::Insight;

use crate::brief::section_label;
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

/// A skill name for comparing: lower case, spaces collapsed, a trailing parenthetical dropped
/// (`Kubernetes (EKS)` -> `kubernetes`). `/` and `&` stay: `CI/CD` is one skill, not `CI` and `CD`.
fn normalize(skill: &str) -> String {
    let lower = skill.to_lowercase();
    let name = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    without_trailing_parenthetical(&name).to_string()
}

/// `name` without a closing `( ... )` group at its end (nested groups included), unless that would
/// leave nothing.
fn without_trailing_parenthetical(name: &str) -> &str {
    let Some(inner) = name.strip_suffix(')') else { return name };
    let mut depth = 1usize;
    for (at, c) in inner.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    let head = inner[..at].trim_end();
                    return if head.is_empty() { name } else { head };
                }
            }
            _ => {}
        }
    }
    name
}

/// The same skill: equal names once normalised ([`normalize`]). Nothing looser: `Java` is not
/// `Java Spring` or `JavaScript`, `Cloud` is not `Google Cloud`, `C` is not `C++`, `CI/CD` is not
/// `CI`. A false "not in your profile" costs a glance; a false match would be a claim.
fn same_skill(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    !a.is_empty() && a == b
}

/// The posting's skills (required first, then preferred, each once) split by the profile's.
pub(super) fn split_skills(wanted: &Skills, profile: &[String]) -> SkillSplit {
    let mut seen: Vec<String> = Vec::new();
    let mut split = SkillSplit::default();
    for skill in wanted.required.iter().chain(&wanted.preferred) {
        let key = normalize(skill);
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

// ----------------------------------------------------------- drawing

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

#[cfg(test)]
mod tests {
    use jobpilot_core::pipeline::insight::Skills;

    use super::{capitalize, same_skill, shorten, split_skills, SkillSplit};

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
        let wanted = Skills { required: names(&["Java", "C", "SQL", "Cloud", "CI/CD", "REST"]), preferred: vec![] };
        let profile = names(&["JavaScript", "C++", "PostgreSQL", "Google Cloud", "CI", "REST APIs", "Java Spring"]);
        let split = split_skills(&wanted, &profile);
        assert!(split.matched.is_empty(), "{:?}", split.matched);
        assert_eq!(split.gaps, names(&["Java", "C", "SQL", "Cloud", "CI/CD", "REST"]));
    }

    #[test]
    fn a_longer_name_does_not_match_a_part_of_it() {
        // Both ways round: what the posting asks for, and what the profile has.
        assert!(!same_skill("Java", "Java Spring"));
        assert!(!same_skill("Java Spring", "Java"));
        assert!(!same_skill("Cloud", "Google Cloud"));
        assert!(!same_skill("CI/CD", "CI"));
        assert!(!same_skill("CI", "CI/CD"));
        assert!(!same_skill("Terraform", "Terraform / OpenTofu"), "'/' and '&' do not split a name");
        assert!(!same_skill("R&D", "R"));
    }

    #[test]
    fn case_spacing_and_a_trailing_parenthetical_are_ignored() {
        assert!(same_skill("rust", "Rust"));
        assert!(same_skill("  Node.js ", "node.js"));
        assert!(same_skill("Apache   Kafka", "apache kafka"));
        assert!(same_skill("C++", "c++"));
        assert!(same_skill("CI/CD", "ci/cd"));
        assert!(same_skill("Kubernetes (EKS)", "Kubernetes"));
        assert!(same_skill("Kubernetes", "kubernetes  (EKS)"));
        assert!(same_skill("Postgres (RDS (Aurora))", "Postgres"), "a nested group goes with the rest");
        assert!(!same_skill("C (embedded) and Rust", "C"), "only a parenthetical at the very end goes");
    }

    #[test]
    fn nothing_matches_a_blank_or_a_bare_parenthetical() {
        assert!(!same_skill("", ""));
        assert!(!same_skill("  ", " "));
        assert!(!same_skill("(EKS)", "(GKE)"), "a name that is only a parenthetical is kept whole");
        assert!(same_skill("(EKS)", "(eks)"));
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
