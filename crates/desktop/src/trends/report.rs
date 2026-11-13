//! The latest digest with clickable citations, the skill-frequency chart and the history of reports.

use gpui_kit::component::button::*;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Local, Utc};
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

/// Documents a month needs before its share of them says anything.
const MIN_DOCUMENTS: u32 = 10;
/// Two months are compared only when neither holds more than this many times the documents of the
/// other.
const MAX_SIZE_RATIO: u32 = 2;
/// Shares closer than this (percentage points) are the same.
const SAME_POINTS: f64 = 0.5;

/// Is the latest month of the counts not over at the time the digest was made? The documents of
/// that month are then the first ones only. (`created` and `month` are `YYYY-MM`.)
fn latest_month_is_partial(months: &[String], created: &str) -> bool {
    months.last().is_some_and(|last| created <= last.as_str())
}

/// One line per skill: the count and, when it is fair, how a month's share of documents compares
/// with the month before. A raw count would grow with the number of documents read, so the share
/// (count / documents of that month) is compared, and only when both months are of a similar size
/// and enough documents; otherwise the line says no more than the count. A month that is not over
/// is shown as "so far" and left out of the comparison: the last two whole months are compared.
fn describe(skill: &SkillCount, documents: &[u32], latest_partial: bool) -> String {
    let n = skill.by_month.len();
    let last = skill.by_month.last().copied().unwrap_or(0);
    let skipped = usize::from(latest_partial);
    let (this_month, prior) = if latest_partial { (format!("{last} so far this month"), "the month before") } else { (format!("{last} this month"), "last month") };
    let line = |tail: &str| format!("{} in total \u{b7} {this_month}{tail}", skill.total);
    if documents.len() != n || n < 2 + skipped {
        return line("");
    }
    let (before_at, now_at) = (n - 2 - skipped, n - 1 - skipped);
    let (docs_before, docs_now) = (documents[before_at], documents[now_at]);
    if docs_before < MIN_DOCUMENTS || docs_now < MIN_DOCUMENTS {
        return line(", too few documents to compare");
    }
    if docs_now.max(docs_before) > docs_now.min(docs_before).saturating_mul(MAX_SIZE_RATIO) {
        return line(", months too different in size to compare");
    }
    let share = |count: u32, docs: u32| f64::from(count) * 100.0 / f64::from(docs);
    let (now, before) = (share(skill.by_month[now_at], docs_now), share(skill.by_month[before_at], docs_before));
    let change = if (now - before).abs() < SAME_POINTS {
        format!("about the same as {prior} ({before:.1}%)")
    } else if now > before {
        format!("up from {before:.1}% {prior}")
    } else {
        format!("down from {before:.1}% {prior}")
    };
    if latest_partial {
        line(&format!(" \u{b7} last month {now:.1}% of documents, {change}"))
    } else {
        format!("{} in total \u{b7} {now:.1}% of this month's documents, {change}", skill.total)
    }
}

/// Only web addresses are opened, whatever the saved data says.
pub(super) fn is_web_url(url: &str) -> bool {
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
            Some(data) => self.render_data(data, &report.created_at, c, cx),
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

    fn render_data(&self, data: &DigestData, created: &DateTime<Utc>, c: &Colors, cx: &Context<Self>) -> Div {
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
        if let Some(chart) = self.render_chart(&data.counts, created, c, cx) {
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
    fn render_chart(&self, counts: &SkillCounts, created: &DateTime<Utc>, c: &Colors, cx: &Context<Self>) -> Option<Stateful<Div>> {
        let max = counts.skills.first()?.total.max(1) as f32;
        let documents: u32 = counts.documents.iter().sum();
        let partial = latest_month_is_partial(&counts.months, &created.format("%Y-%m").to_string());
        let rows = counts.skills.iter().take(CHART_ROWS).enumerate().map(|(i, skill)| {
            let share = (skill.total as f32 / max).clamp(0.02, 1.0);
            // Any skill of the chart, rising or not: the message makes no claim about the trend.
            let message = format!("Add the skill \"{}\" to my preferences.", skill.skill);
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
                .child(div().w(px(430.)).flex_shrink_0().truncate().text_xs().text_color(c.muted).child(describe(skill, &counts.documents, partial)))
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
    use super::{describe, is_web_url, latest_month_is_partial, suggestion};
    use jobpilot_core::trends::{Section, SkillCount};

    fn skill(by_month: &[u32]) -> SkillCount {
        SkillCount { skill: "Rust".into(), by_month: by_month.to_vec(), total: by_month.iter().sum() }
    }

    const DOT: &str = "\u{b7}";

    #[test]
    fn a_skill_is_described_by_its_share_of_the_documents() {
        // 20 of 100 documents last month, 30 of 100 this month: up.
        assert_eq!(describe(&skill(&[10, 20, 30]), &[100, 100, 100], false), format!("60 in total {DOT} 30.0% of this month's documents, up from 20.0% last month"));
        assert_eq!(describe(&skill(&[30, 10]), &[100, 100], false), format!("40 in total {DOT} 10.0% of this month's documents, down from 30.0% last month"));
        assert_eq!(describe(&skill(&[20, 20]), &[100, 100], false), format!("40 in total {DOT} 20.0% of this month's documents, about the same as last month (20.0%)"));
    }

    #[test]
    fn more_documents_alone_are_not_a_rise() {
        // The count doubles, but so do the documents read: the share is the same.
        let line = describe(&skill(&[10, 20]), &[50, 100], false);
        assert!(line.contains("about the same as last month (20.0%)"), "{line}");
        // The count falls, but the share rises.
        let line = describe(&skill(&[20, 15]), &[100, 60], false);
        assert!(line.contains("up from 20.0% last month") && line.contains("25.0% of this month's"), "{line}");
    }

    #[test]
    fn a_month_that_is_not_over_is_left_out_of_the_comparison() {
        // The last two whole months (10 -> 20 of 100 documents) are compared; this month is "so far".
        let line = describe(&skill(&[10, 20, 30]), &[100, 100, 100], true);
        assert_eq!(line, format!("60 in total {DOT} 30 so far this month {DOT} last month 20.0% of documents, up from 10.0% the month before"));
        assert_eq!(describe(&skill(&[30, 10, 5]), &[100, 100, 100], true), format!("45 in total {DOT} 5 so far this month {DOT} last month 10.0% of documents, down from 30.0% the month before"));
        assert!(describe(&skill(&[20, 20, 1]), &[100, 100, 100], true).contains("about the same as the month before (20.0%)"));
        // The size of the month that is not over does not matter: only the whole months are compared.
        assert!(describe(&skill(&[10, 20, 3]), &[100, 100, 4], true).contains("up from 10.0% the month before"));
        // The same counts with the month over: the last two months are compared instead.
        assert!(describe(&skill(&[10, 20, 3]), &[100, 100, 4], false).contains("too few documents"));
    }

    #[test]
    fn nothing_is_said_about_the_trend_when_it_would_not_be_fair() {
        let rising = skill(&[1, 2, 5]);
        // Months of very different size (more than twice as many documents).
        let line = describe(&rising, &[100, 40, 90], false);
        assert!(line.ends_with("months too different in size to compare"), "{line}");
        assert!(!line.contains("up from") && !line.contains('%'), "{line}");
        let line = describe(&rising, &[40, 100, 90], true);
        assert_eq!(line, format!("8 in total {DOT} 5 so far this month, months too different in size to compare"));
        // Too few documents for a share to mean anything.
        let line = describe(&rising, &[100, 100, 9], false);
        assert!(line.ends_with("too few documents to compare"), "{line}");
        let line = describe(&rising, &[100, 9, 100], true);
        assert!(line.ends_with("so far this month, too few documents to compare"), "{line}");
        // No document counts, too few months (a month that is not over needs two whole ones before it).
        assert_eq!(describe(&rising, &[], false), format!("8 in total {DOT} 5 this month"));
        assert_eq!(describe(&skill(&[3]), &[50], false), format!("3 in total {DOT} 3 this month"));
        assert_eq!(describe(&skill(&[]), &[], false), format!("0 in total {DOT} 0 this month"));
        assert_eq!(describe(&skill(&[3, 4]), &[50, 50], true), format!("7 in total {DOT} 4 so far this month"));
        assert_eq!(describe(&skill(&[4]), &[50], true), format!("4 in total {DOT} 4 so far this month"));
    }

    #[test]
    fn the_latest_month_is_partial_while_it_is_not_over() {
        let months = vec!["2026-10".to_string(), "2026-11".to_string()];
        assert!(latest_month_is_partial(&months, "2026-11"), "made in the month");
        assert!(latest_month_is_partial(&months, "2026-10"), "made before the month");
        assert!(!latest_month_is_partial(&months, "2026-12"), "made after it");
        assert!(!latest_month_is_partial(&[], "2026-12"));
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
