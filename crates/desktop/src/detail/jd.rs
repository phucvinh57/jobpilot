//! JD tab: the job description text, its source link and the key terms found in it.

use gpui_kit::component::tag::Tag;
use gpui_kit::component::text::TextView;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::{DateTime, Utc};
use jobpilot_core::pipeline::{read_jd_text, Analysis};
use jobpilot_core::store::Application;
use jobpilot_core::runtime;

use crate::app::AppState;

/// What the tab shows for one application.
#[derive(Default)]
struct JdView {
    /// `None` = no JD stored (yet).
    text: Option<String>,
    analysis: Option<Analysis>,
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

impl Render for JdTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let app = self.state.read(cx).selected_app().cloned();
        if let Some(app) = &app {
            if self.loaded != Some((app.id, app.updated_at)) {
                self.load(app.clone(), cx);
            }
        }
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let source = app.as_ref().and_then(|a| a.source.clone());

        let tags = |label: &'static str, items: &[String], must: bool| {
            div()
                .h_flex()
                .flex_wrap()
                .items_center()
                .gap_1()
                .child(div().text_xs().text_color(muted).pr_1().child(label))
                .children(items.iter().map(|item| {
                    let tag = if must { Tag::info() } else { Tag::secondary() };
                    tag.small().child(item.clone())
                }))
        };

        div()
            .v_flex()
            .size_full()
            .gap_2()
            .when_some(source, |d, src| {
                if is_link(&src) {
                    let url = src.clone();
                    d.child(
                        div()
                            .id("jd-source")
                            .text_sm()
                            .text_color(theme.link)
                            .underline()
                            .cursor_pointer()
                            .truncate()
                            .child(src)
                            .on_click(move |_, _, cx| cx.open_url(&url)),
                    )
                } else {
                    d.child(div().text_sm().text_color(muted).truncate().child(src))
                }
            })
            .when_some(self.view.analysis.as_ref(), |d, a| {
                d.when(!a.must_have.is_empty(), |d| d.child(tags("Must have", &a.must_have, true)))
                    .when(!a.nice_to_have.is_empty(), |d| d.child(tags("Nice to have", &a.nice_to_have, false)))
            })
            .child(match &self.view.text {
                Some(text) => div().flex_1().min_h_0().child(
                    TextView::markdown("jd-text", jd_markdown(text)).selectable(true).scrollable(true),
                ),
                None => div().text_color(muted).child("No JD text yet."),
            })
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
