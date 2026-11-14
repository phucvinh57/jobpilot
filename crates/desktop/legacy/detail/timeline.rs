//! Timeline tab: the application's events, newest first, and the emails that need a review.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::chrono::Local;
use jobpilot_core::store::{Email, Event};
use jobpilot_core::{runtime, Core};

use crate::app::AppState;
use crate::ui::{self, Level, Tone};

/// Longest event text shown on its row; the full text is in the tooltip.
const ROW_CHARS: usize = 110;

/// Look of one timeline line, derived from the event's free-form `kind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventStyle {
    Created,
    Status,
    Cv,
    Mail,
    Review,
    Warning,
    Error,
    Note,
}

impl EventStyle {
    fn of(kind: &str) -> Self {
        match kind {
            "created" => Self::Created,
            "status" => Self::Status,
            "cv" => Self::Cv,
            "email" => Self::Mail,
            "email_review" => Self::Review,
            "warning" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Note,
        }
    }

    /// The colour of the row's dot.
    fn tone(self) -> Tone {
        match self {
            Self::Created | Self::Note => Tone::Neutral,
            Self::Status | Self::Cv | Self::Mail => Tone::Accent,
            Self::Review | Self::Warning => Tone::Warn,
            Self::Error => Tone::Danger,
        }
    }
}

/// What a row shows of an event text: its first non-empty line, cut at [`ROW_CHARS`]; `true` when something was left out.
fn row_text(message: &str) -> (String, bool) {
    let first = message.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if first.chars().count() > ROW_CHARS {
        let cut: String = first.chars().take(ROW_CHARS).collect();
        return (format!("{}...", cut.trim_end()), true);
    }
    (first.to_string(), first.len() != message.trim().len())
}

/// Newest first (the store returns oldest first).
fn newest_first(mut events: Vec<Event>) -> Vec<Event> {
    events.reverse();
    events
}

/// One line for a pending email review: `Rejection (60% sure): Subject`.
fn review_title(email: &Email) -> String {
    let kind = email.kind.as_deref().unwrap_or("other");
    let kind = kind.chars().next().map(|c| c.to_uppercase().collect::<String>() + &kind[c.len_utf8()..]).unwrap_or_default();
    match email.confidence {
        Some(c) => format!("{kind} ({:.0}% sure): {}", c * 100.0, email.subject),
        None => format!("{kind}: {}", email.subject),
    }
}

#[derive(Default)]
struct TimelineData {
    app_id: Option<i64>,
    events: Vec<Event>,
    reviews: Vec<Email>,
}

pub struct TimelineTab {
    state: Entity<AppState>,
    data: TimelineData,
    /// Latest load that may still apply (older answers are dropped).
    load_seq: u64,
    /// The last accept/dismiss failure.
    error: Option<String>,
}

impl TimelineTab {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // The state notifies on selection changes, pipeline progress, status changes and mail syncs:
        // all of them can add an event, so reload each time.
        cx.observe(&state, |this, _, cx| this.reload(cx)).detach();
        let mut tab = Self { state, data: TimelineData::default(), load_seq: 0, error: None };
        tab.reload(cx);
        tab
    }

    /// Read the events and pending reviews off the UI thread.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let (core, selected) = {
            let state = self.state.read(cx);
            (state.core.clone(), state.selected)
        };
        let Some(id) = selected else {
            self.data = TimelineData::default();
            return;
        };
        if self.data.app_id != Some(id) {
            self.data = TimelineData { app_id: Some(id), ..Default::default() };
            self.error = None;
        }
        self.load_seq += 1;
        let seq = self.load_seq;
        let rx = runtime::spawn(async move {
            (core.store.list_events(id).unwrap_or_default(), core.emails_needing_review(Some(id)).unwrap_or_default())
        });
        cx.spawn(async move |this, cx| {
            let Ok((events, reviews)) = rx.await else { return };
            this.update(cx, |this, cx| {
                if this.load_seq == seq {
                    this.data = TimelineData { app_id: Some(id), events: newest_first(events), reviews };
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Accept (apply the suggested status) or dismiss a low-confidence email.
    fn resolve(&mut self, email_id: String, accept: bool, cx: &mut Context<Self>) {
        let core: std::sync::Arc<Core> = self.state.read(cx).core.clone();
        let rx = runtime::spawn(async move {
            if accept {
                core.accept_email(&email_id).map(|_| ())
            } else {
                core.dismiss_email(&email_id)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("{e:#}")),
                    Err(_) => Some("the task was cancelled".into()),
                };
                // A status change shows up in the list, the sidebar and the header.
                this.state.update(cx, |state, cx| state.reload(cx));
            })
            .ok();
        })
        .detach();
    }
}

impl Render for TimelineTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = ui::palette(cx);

        let reviews = self.data.reviews.iter().map(|email| {
            let accept_id = email.id.clone();
            let dismiss_id = email.id.clone();
            let ix = gpui_kit::ElementId::from(SharedString::from(format!("review-{}", email.id)));
            let body = format!(
                "From {}{}",
                email.from_addr,
                email.summary.as_deref().filter(|s| !s.is_empty()).map(|s| format!(" - {s}")).unwrap_or_default()
            );
            let actions = div()
                .h_flex()
                .flex_shrink_0()
                .gap_2()
                .child(
                    ui::secondary_button(SharedString::from(format!("accept-{}", email.id)), "Accept")
                        .on_click(cx.listener(move |this, _, _, cx| this.resolve(accept_id.clone(), true, cx))),
                )
                .child(
                    ui::ghost_button(SharedString::from(format!("dismiss-{}", email.id)), "Dismiss")
                        .on_click(cx.listener(move |this, _, _, cx| this.resolve(dismiss_id.clone(), false, cx))),
                );
            div().id(ix).child(ui::banner(Tone::Warn, &review_title(email), &body, actions, cx))
        });

        let events = self.data.events.iter().map(|e| {
            let style = EventStyle::of(&e.kind);
            let when = e.at.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string();
            let (text, cut) = row_text(&e.message);
            let full = e.message.clone();
            div()
                .id(("event", usize::try_from(e.id).unwrap_or_default()))
                .child(ui::activity_row(style.tone(), &text, &when, cx))
                .when(cut, |d| d.tooltip(move |window, cx| Tooltip::new(full.clone()).build(window, cx)))
        });

        let history = if self.data.events.is_empty() {
            div().text_size(px(ui::tokens::TEXT_BODY)).text_color(p.muted).child("Nothing has happened yet.").into_any_element()
        } else {
            div().v_flex().children(events).into_any_element()
        };

        div()
            .id("timeline")
            .v_flex()
            .size_full()
            .gap_3()
            .overflow_y_scroll()
            .when_some(self.error.clone(), |d, message| {
                d.child(div().text_size(px(ui::tokens::TEXT_BODY)).text_color(p.danger).child(message))
            })
            .children(reviews)
            .child(ui::card(cx).child(ui::heading("History", Level::H2, cx)).child(history))
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::chrono::Utc;
    use jobpilot_core::store::{Email, Event};

    use super::{newest_first, review_title, row_text, EventStyle, ROW_CHARS};
    use crate::ui::Tone;

    fn event(id: i64, kind: &str) -> Event {
        Event { id, app_id: 1, at: Utc::now(), kind: kind.into(), message: format!("m{id}") }
    }

    #[test]
    fn kinds_map_to_styles() {
        assert_eq!(EventStyle::of("error"), EventStyle::Error);
        assert_eq!(EventStyle::of("email_review"), EventStyle::Review);
        assert_eq!(EventStyle::of("status"), EventStyle::Status);
        assert_eq!(EventStyle::of("whatever"), EventStyle::Note);
    }

    #[test]
    fn kinds_have_a_dot_colour() {
        assert_eq!(EventStyle::of("error").tone(), Tone::Danger);
        assert_eq!(EventStyle::of("warning").tone(), Tone::Warn);
        assert_eq!(EventStyle::of("email_review").tone(), Tone::Warn);
        assert_eq!(EventStyle::of("status").tone(), Tone::Accent);
        assert_eq!(EventStyle::of("created").tone(), Tone::Neutral);
    }

    #[test]
    fn a_row_shows_one_short_line() {
        assert_eq!(row_text("CV generated"), ("CV generated".to_string(), false));
        let (first, cut) = row_text("Gaps: Haskell\nsecond line");
        assert_eq!((first.as_str(), cut), ("Gaps: Haskell", true));
        let (long, cut) = row_text(&"x".repeat(ROW_CHARS + 20));
        assert!(cut && long.ends_with("...") && long.chars().count() == ROW_CHARS + 3);
    }

    #[test]
    fn newest_event_comes_first() {
        let ids: Vec<i64> = newest_first(vec![event(1, "created"), event(2, "cv"), event(3, "error")])
            .iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, [3, 2, 1]);
    }

    #[test]
    fn review_title_shows_kind_and_confidence() {
        let mut email = Email {
            id: "m1".into(),
            app_id: Some(1),
            from_addr: "hr@acme.test".into(),
            subject: "Your application".into(),
            snippet: String::new(),
            received_at: Utc::now(),
            kind: Some("rejection".into()),
            confidence: Some(0.6),
            summary: None,
            applied: false,
        };
        assert_eq!(review_title(&email), "Rejection (60% sure): Your application");
        email.confidence = None;
        assert_eq!(review_title(&email), "Rejection: Your application");
    }
}
