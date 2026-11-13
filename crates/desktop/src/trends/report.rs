//! The latest digest with clickable citations, the skill-frequency chart and the history of reports.

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::Local;
use jobpilot_core::store::Report;
use jobpilot_core::trends::{Digest, DigestData, Point, Section, SkillCount, SkillCounts};

use super::{Colors, TrendsView};
use crate::settings::section;

/// Skills drawn in the chart.
const CHART_ROWS: usize = 10;
/// Reports listed in the history.
const HISTORY_ROWS: usize = 30;

/// The message the Preferences screen gets for a skill or domain of the digest.
fn suggestion(section: Section, topic: &str) -> String {
    match section {
        Section::Domains => format!("Add the domain \"{topic}\" to my preferences: the market looks active there."),
        _ => format!("Add the skill \"{topic}\" to my preferences: the market asks for it more."),
    }
}

/// One line per skill: the count and how the latest month compares with the one before. Words, not
/// only a bar length: the numbers are the chart's labels.
fn describe(skill: &SkillCount) -> String {
    let n = skill.by_month.len();
    let last = skill.by_month.last().copied().unwrap_or(0);
    let before = if n >= 2 { skill.by_month[n - 2] } else { 0 };
    let change = match last.cmp(&before) {
        std::cmp::Ordering::Greater => format!("up {} on last month", last - before),
        std::cmp::Ordering::Less => format!("down {} on last month", before - last),
        std::cmp::Ordering::Equal => "same as last month".to_string(),
    };
    format!("{} in total \u{b7} {} this month, {}", skill.total, last, change)
}

/// Only web addresses are opened, whatever the saved data says.
fn is_web_url(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("http://")
}

fn when(report: &Report) -> String {
    report.created_at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()
}

impl TrendsView {
    pub(super) fn render_digest(&self, c: &Colors, cx: &Context<Self>) -> Stateful<Div> {
        let Some(digest) = self.snap.shown.as_ref() else {
            return section("Digest", "Nothing yet.", cx)
                .id("trends-digest")
                .role(Role::Group)
                .aria_label("Digest")
                .child(div().text_sm().text_color(c.muted).child(
                    "Add sources below, then press \"Collect & build digest\". You need an AI provider (Settings > AI providers).",
                ));
        };
        let report = &digest.report;
        let hint = match &digest.data {
            Some(data) => format!("{} \u{b7} {} item(s) read", when(report), data.items_read),
            None => when(report),
        };
        let body = match &digest.data {
            Some(data) => self.render_data(data, c, cx),
            None => self.render_plain(digest, c),
        };
        section(&report.title, &hint, cx).id("trends-digest").role(Role::Group).aria_label(report.title.clone()).child(body)
    }

    /// A report this version cannot read back: its text as saved.
    fn render_plain(&self, digest: &Digest, c: &Colors) -> Div {
        div()
            .v_flex()
            .gap_1()
            .text_sm()
            .text_color(c.muted)
            .children(digest.report.body_md.lines().map(|l| div().child(l.to_string())))
    }

    fn render_data(&self, data: &DigestData, c: &Colors, cx: &Context<Self>) -> Div {
        let mut body = div().v_flex().gap_4();
        for sec in Section::ALL {
            let points: Vec<(usize, &Point)> = data.points.iter().enumerate().filter(|(_, p)| p.section == sec).collect();
            if points.is_empty() {
                continue;
            }
            body = body.child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(div().font_semibold().child(sec.title()))
                    .children(points.into_iter().map(|(idx, p)| self.render_point(idx, p, data, c, cx))),
            );
        }
        if data.points.is_empty() {
            body = body.child(div().text_sm().text_color(c.muted).child("No point could be tied to a source this time."));
        }
        if let Some(chart) = self.render_chart(&data.counts, c, cx) {
            body = body.child(chart);
        }
        body.when(data.dropped > 0, |d| {
            d.child(div().text_xs().text_color(c.muted).child(format!(
                "{} point(s) were left out: no valid source, or a quote that is not in the source.",
                data.dropped
            )))
        })
    }

    fn render_point(&self, idx: usize, point: &Point, data: &DigestData, c: &Colors, cx: &Context<Self>) -> Div {
        let chips: Vec<Stateful<Div>> = point
            .citations
            .iter()
            .enumerate()
            .filter_map(|(j, id)| {
                let ordinal = data.items.iter().position(|i| i.id == *id)?;
                let item = &data.items[ordinal];
                let url = item.url.clone();
                let chip = div()
                    .id(("trends-cite", idx * 16 + j))
                    .role(Role::Link)
                    .aria_label(format!("Open source: {}", item.title))
                    .text_xs()
                    .text_color(c.link)
                    .max_w(px(280.))
                    .truncate()
                    .child(format!("[{}] {}", ordinal + 1, item.title));
                Some(if is_web_url(&url) {
                    chip.underline().cursor_pointer().on_click(move |_, _, cx| cx.open_url(&url))
                } else {
                    chip
                })
            })
            .collect();
        let suggest = (point.section.is_suggestible() && !point.topic.is_empty()).then(|| {
            let message = suggestion(point.section, &point.topic);
            Button::new(("trends-suggest-point", idx))
                .outline()
                .compact()
                .label("Add to preferences")
                .accessibility_label(format!("Add {} to preferences", point.topic))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let message = message.clone();
                    this.state.update(cx, |s, cx| s.suggest_preference(message, cx));
                }))
        });
        div()
            .v_flex()
            .gap_1()
            .child(
                div()
                    .h_flex()
                    .items_start()
                    .gap_1()
                    .text_sm()
                    .when(!point.topic.is_empty(), |d| d.child(div().flex_shrink_0().font_semibold().child(format!("{}:", point.topic))))
                    .child(div().flex_1().min_w_0().child(point.text.clone())),
            )
            .when(!point.quote.is_empty(), |d| {
                d.child(div().text_xs().text_color(c.muted).italic().child(format!("\u{201c}{}\u{201d}", point.quote)))
            })
            .child(div().h_flex().flex_wrap().items_center().gap_2().children(chips).children(suggest))
    }

    /// Simple bars: one hue, the numbers beside each bar.
    fn render_chart(&self, counts: &SkillCounts, c: &Colors, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let max = counts.skills.first()?.total.max(1) as f32;
        let documents: u32 = counts.documents.iter().sum();
        let rows = counts.skills.iter().take(CHART_ROWS).enumerate().map(|(i, skill)| {
            let share = (skill.total as f32 / max).clamp(0.02, 1.0);
            let message = suggestion(Section::RisingSkills, &skill.skill);
            div()
                .h_flex()
                .items_center()
                .gap_2()
                .child(div().w(px(120.)).flex_shrink_0().truncate().text_sm().child(skill.skill.clone()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(60.))
                        .h(px(8.))
                        .rounded_full()
                        .bg(c.track)
                        .child(div().h_full().w(relative(share)).rounded_full().bg(c.primary)),
                )
                .child(div().w(px(300.)).flex_shrink_0().truncate().text_xs().text_color(c.muted).child(describe(skill)))
                .child(
                    Button::new(("trends-suggest-skill", i))
                        .ghost()
                        .compact()
                        .icon(IconName::Plus)
                        .accessibility_label(format!("Add {} to preferences", skill.skill))
                        .tooltip("Add to preferences")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let message = message.clone();
                            this.state.update(cx, |s, cx| s.suggest_preference(message, cx));
                        })),
                )
        });
        Some(
            div()
                .id("trends-chart")
                .role(Role::Group)
                .aria_label("Skill frequency")
                .v_flex()
                .gap_2()
                .child(div().font_semibold().child("Skill frequency"))
                .child(div().text_xs().text_color(c.muted).child(format!(
                    "Job postings and leads naming each skill, {} to {} ({documents} read). Counted by the app, not by the AI.",
                    counts.months.first().map_or("", String::as_str),
                    counts.months.last().map_or("", String::as_str),
                )))
                .children(rows),
        )
    }

    pub(super) fn render_history(&self, c: &Colors, cx: &Context<Self>) -> Stateful<Div> {
        let shown = self.snap.shown.as_ref().map(|d| d.report.id);
        let rows: Vec<Stateful<Div>> = self
            .snap
            .reports
            .iter()
            .take(HISTORY_ROWS)
            .map(|r| {
                let id = r.id;
                let selected = shown == Some(id);
                div()
                    .id(("trends-report", id as usize))
                    .role(Role::ListItem)
                    .aria_label(format!("{}, {}", r.title, when(r)))
                    .h_flex()
                    .justify_between()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(selected, |d| d.bg(c.accent))
                    .hover(|d| d.bg(c.accent))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id, cx)))
                    .child(div().truncate().child(r.title.clone()))
                    .child(div().text_xs().text_color(c.muted).flex_shrink_0().child(when(r)))
            })
            .collect();
        section("History", "Past digests, newest first. Click one to read it again.", cx)
            .id("trends-history")
            .role(Role::Group)
            .aria_label("History")
            .when(rows.is_empty(), |d| d.child(div().text_sm().text_color(c.muted).child("No digests yet.")))
            .children(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::{describe, is_web_url, suggestion};
    use jobpilot_core::trends::{Section, SkillCount};

    fn skill(by_month: &[u32]) -> SkillCount {
        SkillCount { skill: "Rust".into(), by_month: by_month.to_vec(), total: by_month.iter().sum() }
    }

    #[test]
    fn a_skill_is_described_in_words() {
        assert_eq!(describe(&skill(&[1, 2, 5])), "8 in total \u{b7} 5 this month, up 3 on last month");
        assert_eq!(describe(&skill(&[4, 1])), "5 in total \u{b7} 1 this month, down 3 on last month");
        assert_eq!(describe(&skill(&[2, 2])), "4 in total \u{b7} 2 this month, same as last month");
        assert_eq!(describe(&skill(&[3])), "3 in total \u{b7} 3 this month, up 3 on last month");
        assert_eq!(describe(&skill(&[])), "0 in total \u{b7} 0 this month, same as last month");
    }

    #[test]
    fn suggestions_name_the_kind_of_preference() {
        assert!(suggestion(Section::RisingSkills, "Rust").contains("skill \"Rust\""));
        assert!(suggestion(Section::Domains, "fintech").contains("domain \"fintech\""));
    }

    #[test]
    fn only_web_addresses_are_opened() {
        assert!(is_web_url("https://a.test/x") && is_web_url("http://a.test"));
        for bad in ["file:///etc/passwd", "javascript:alert(1)", "mailto:a@b.c", "ftp://x", ""] {
            assert!(!is_web_url(bad), "{bad}");
        }
    }
}
