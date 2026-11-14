//! The source list (add, enable, remove) and the search API keys of the Trends screen.

use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::store::Source;
use jobpilot_core::trends::{SearchProvider, SourceKind};

use super::{Colors, TrendsView};
use crate::settings::section;

/// What the source's last run says, in words.
fn status_text(source: &Source) -> String {
    match source.last_status.as_deref() {
        None => "not collected yet".into(),
        Some("needs_human") => "needs you: see the banner above".into(),
        Some(status) => format!("last run: {status}"),
    }
}

/// `Hacker News` or `RSS / Atom feed · blog.example.com`: the kind, and the host of its address
/// (never the whole address: a feed address may carry a token).
fn describe(source: &Source) -> String {
    let label = SourceKind::parse(&source.kind).map_or_else(|| source.kind.clone(), |k| k.label().to_string());
    match url::Url::parse(&source.url).ok().and_then(|u| u.host_str().map(str::to_string)) {
        Some(host) => format!("{label} \u{b7} {host}"),
        None => label,
    }
}

impl TrendsView {
    fn has_source(&self, kind: SourceKind) -> bool {
        self.snap.sources.iter().any(|s| s.kind == kind.as_str())
    }

    /// Add `kind`: Hacker News and YC need nothing more, a feed or page uses the form's address.
    fn add_source(&mut self, kind: SourceKind, window: &mut Window, cx: &mut Context<Self>) {
        let (name, url) = if kind.needs_url() {
            (self.form.name.read(cx).value().trim().to_string(), self.form.url.read(cx).value().trim().to_string())
        } else {
            (String::new(), String::new())
        };
        self.sources_error = None;
        let core = self.core(cx);
        self.task(window, cx, async move { core.add_trend_source(kind, &name, &url) }, |this, result, window, cx| match result {
            Ok(_) => {
                this.adding = None;
                this.form.name.update(cx, |i, cx| i.set_value("", window, cx));
                this.form.url.update(cx, |i, cx| i.set_value("", window, cx));
                this.reload(cx);
            }
            Err(e) => this.sources_error = Some(format!("{e:#}")),
        });
    }

    fn remove_source(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        self.sources_error = None;
        let core = self.core(cx);
        self.task(window, cx, async move { core.remove_trend_source(id) }, |this, result, _, cx| {
            match result {
                Ok(()) => this.reload(cx),
                Err(e) => this.sources_error = Some(format!("{e:#}")),
            }
        });
    }

    fn set_source_enabled(&mut self, id: i64, enabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.sources_error = None;
        let core = self.core(cx);
        self.task(window, cx, async move { core.set_trend_source_enabled(id, enabled) }, |this, result, _, cx| {
            match result {
                Ok(()) => this.reload(cx),
                Err(e) => this.sources_error = Some(format!("{e:#}")),
            }
        });
    }

    fn key_input(&self, provider: SearchProvider) -> &Entity<InputState> {
        match provider {
            SearchProvider::Brave => &self.form.brave,
            SearchProvider::Tavily => &self.form.tavily,
        }
    }

    /// Save the typed key in the keyring (through the core only); the field is cleared afterwards.
    fn save_key(&mut self, provider: SearchProvider, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key_input(provider).read(cx).value().trim().to_string();
        if key.is_empty() {
            self.sources_error = Some(format!("Enter the {} API key first.", provider.label()));
            cx.notify();
            return;
        }
        self.sources_error = None;
        let core = self.core(cx);
        self.task(window, cx, async move { core.set_search_key(provider, &key) }, move |this, result, window, cx| match result {
            Ok(()) => {
                this.key_input(provider).clone().update(cx, |i, cx| i.set_value("", window, cx));
                this.reload(cx);
            }
            Err(e) => this.sources_error = Some(format!("{e:#}")),
        });
    }

    fn clear_key(&mut self, provider: SearchProvider, window: &mut Window, cx: &mut Context<Self>) {
        self.sources_error = None;
        let core = self.core(cx);
        self.task(window, cx, async move { core.clear_search_key(provider) }, |this, result, _, cx| match result {
            Ok(()) => this.reload(cx),
            Err(e) => this.sources_error = Some(format!("{e:#}")),
        });
    }

    pub(super) fn render_sources(&self, c: &Colors, cx: &Context<Self>) -> Stateful<Div> {
        let rows: Vec<Div> = self.snap.sources.iter().map(|s| self.source_row(s, c, cx)).collect();
        let kind_button = |id: &'static str, kind: SourceKind| {
            let button = Button::new(id).outline().compact().icon(IconName::Plus).label(kind.label());
            if kind.needs_url() {
                button.selected(self.adding == Some(kind)).on_click(cx.listener(move |this, _, _, cx| {
                    this.adding = if this.adding == Some(kind) { None } else { Some(kind) };
                    this.sources_error = None;
                    cx.notify();
                }))
            } else {
                button.disabled(self.has_source(kind)).on_click(cx.listener(move |this, _, window, cx| this.add_source(kind, window, cx)))
            }
        };
        let form = self.adding.map(|kind| {
            div()
                .v_flex()
                .gap_2()
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(200.)).child(Input::new(&self.form.name)))
                        .child(div().flex_1().min_w_0().child(Input::new(&self.form.url))),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .child(
                            Button::new("trends-add-source")
                                .outline()
                                .label(format!("Add {}", kind.label()))
                                .on_click(cx.listener(move |this, _, window, cx| this.add_source(kind, window, cx))),
                        )
                        .child(Button::new("trends-add-cancel").ghost().label("Cancel").on_click(cx.listener(|this, _, _, cx| {
                            this.adding = None;
                            this.sources_error = None;
                            cx.notify();
                        }))),
                )
        });
        section(
            "Sources",
            "Where the digest looks. Feeds and pages are read-only; a site that wants a login or captcha pauses the run and waits for you.",
            cx,
        )
        .id("trends-sources")
        .role(Role::Group)
        .aria_label("Sources")
        .when(rows.is_empty(), |d| {
            d.child(div().text_sm().text_color(c.muted).child("No sources yet: add Hacker News, the YC companies, a feed or a page."))
        })
        .children(rows)
        .child(
            div()
                .h_flex()
                .flex_wrap()
                .gap_2()
                .child(kind_button("trends-add-hn", SourceKind::HackerNews))
                .child(kind_button("trends-add-yc", SourceKind::Yc))
                .child(kind_button("trends-add-rss", SourceKind::Rss))
                .child(kind_button("trends-add-page", SourceKind::Page)),
        )
        .children(form)
        .when_some(self.sources_error.clone(), |d, e| d.child(div().text_sm().min_w_0().text_color(c.danger).child(e)))
    }

    fn source_row(&self, source: &Source, c: &Colors, cx: &Context<Self>) -> Div {
        let id = source.id;
        div()
            .h_flex()
            .items_center()
            .gap_3()
            .child(
                Switch::new(("trend-source-enabled", id as usize))
                    .checked(source.enabled)
                    .accessibility_label(format!("Collect from {}", source.name))
                    .on_click(cx.listener(move |this, checked: &bool, window, cx| this.set_source_enabled(id, *checked, window, cx))),
            )
            .child(
                div()
                    .v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().font_semibold().child(source.name.clone()))
                    .child(div().text_xs().text_color(c.muted).truncate().child(describe(source)))
                    .child(div().text_xs().text_color(c.muted).truncate().child(status_text(source))),
            )
            .child(
                Button::new(("trends-remove-source", id as usize))
                    .outline()
                    .compact()
                    .label("Remove")
                    .accessibility_label(format!("Remove {}", source.name))
                    .on_click(cx.listener(move |this, _, window, cx| this.remove_source(id, window, cx))),
            )
    }

    pub(super) fn render_search(&self, c: &Colors, cx: &Context<Self>) -> Stateful<Div> {
        let rows: Vec<Div> = SearchProvider::ALL
            .into_iter()
            .map(|provider| {
                let saved = self.snap.keys.contains(&provider);
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .v_flex()
                            .w(px(150.))
                            .flex_shrink_0()
                            .child(div().font_semibold().child(provider.label()))
                            .child(div().text_xs().text_color(c.muted).child(if saved { "key saved" } else { "no key" })),
                    )
                    .child(div().flex_1().min_w_0().child(Input::new(self.key_input(provider)).mask_toggle()))
                    .child(
                        Button::new(("trends-key-save", provider as usize))
                            .outline()
                            .label(if saved { "Replace key" } else { "Save key" })
                            .accessibility_label(format!("Save the {} API key", provider.label()))
                            .on_click(cx.listener(move |this, _, window, cx| this.save_key(provider, window, cx))),
                    )
                    .when(saved, |d| {
                        d.child(
                            Button::new(("trends-key-remove", provider as usize))
                                .outline()
                                .label("Remove key")
                                .accessibility_label(format!("Remove the {} API key", provider.label()))
                                .on_click(cx.listener(move |this, _, window, cx| this.clear_key(provider, window, cx))),
                        )
                    })
            })
            .collect();
        let on = !self.snap.keys.is_empty();
        section(
            "Report search",
            "With a Brave Search or Tavily API key, the digest also reads annual and quarterly reports found for your domains and roles. The key is kept in the OS keyring, never in a file.",
            cx,
        )
        .id("trends-search")
        .role(Role::Group)
        .aria_label("Report search")
        .child(div().text_sm().text_color(c.muted).child(if on { "Report search is on." } else { "Report search is off: no key saved." }))
        .children(rows)
    }
}
