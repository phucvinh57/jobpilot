//! Settings > Profile & preferences (P9, folded into Settings): your CV and a small form for the
//! job-search preferences. There is no chat here.
//!
//! - Job type is one click; locations, domains and skills are tag fields (chips with a remove
//!   button, a small "Add" box, and suggestion chips from the core); "Anything else" is free text.
//! - Every change is saved at once through `Core::update_preferences` (so "Undo" works) and shows a
//!   "Saved" label that fades. Saving never cancels anything: searches and CV jobs read the
//!   preferences again on their next step.
//! - After the notes are saved, the model reads them once in the background
//!   (`Core::read_preference_notes`, no chat shown) so it can fill what the form hides: roles,
//!   seniority, salary, visa. Anything but a clean read is a muted line, never a blocker. The
//!   user's own words stay in the notes. Saving the same notes again after a failed read reads
//!   them again.
//!
//! Views only: every read and write runs through `runtime::spawn`. Every change goes through
//! `Core::edit_preferences` (a read, change and write of the whole document under the core's one
//! lock, which a notes read holds too), so a click never overwrites a field that changed elsewhere
//! (a Trends "add", a dismissed company, the model's merge). The edit hands back the document as it
//! is after it, which is what the card shows; an answer is dropped when a newer edit has started
//! (`AppState`'s latest-read rule, `seq`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui_kit::component::button::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::input::*;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::health::{classify, Problem};
use jobpilot_core::prefs::chat::MAX_LIST_ITEMS;
use jobpilot_core::prefs::{clean_notes, cv_upload, NotesRead, PrefSuggestions, Preferences, Remote, UploadedCv};
use jobpilot_core::{runtime, Core};

use super::section_with;
use crate::app::{is_latest_read, AppState, Screen};

/// Longest list entry typed by hand, in characters.
const MAX_ITEM_CHARS: usize = 80;
/// Suggestion chips shown under a field.
const SUGGESTION_CHIPS: usize = 8;
/// How long the "Saved" label stays before it is gone, in milliseconds.
const FLASH_MS: u64 = 2200;
/// Share of that time the label stays fully visible before it fades.
const FLASH_HOLD: f32 = 0.55;

// ---------------------------------------------------------------- pure logic

/// A list of the preferences that is edited as a tag field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Field {
    Locations,
    Domains,
    Skills,
}

impl Field {
    const ALL: [Field; 3] = [Field::Locations, Field::Domains, Field::Skills];

    /// Used in element ids.
    fn key(self) -> &'static str {
        match self {
            Field::Locations => "locations",
            Field::Domains => "domains",
            Field::Skills => "skills",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Field::Locations => "Work locations",
            Field::Domains => "Interested domains",
            Field::Skills => "Skills",
        }
    }

    fn placeholder(self) -> &'static str {
        match self {
            Field::Locations => "Add a city or country",
            Field::Domains => "Add a domain, e.g. fintech",
            Field::Skills => "Add a skill",
        }
    }

    fn list(self, prefs: &Preferences) -> &Vec<String> {
        match self {
            Field::Locations => &prefs.locations,
            Field::Domains => &prefs.domains,
            Field::Skills => &prefs.must_skills,
        }
    }

    fn list_mut(self, prefs: &mut Preferences) -> &mut Vec<String> {
        match self {
            Field::Locations => &mut prefs.locations,
            Field::Domains => &mut prefs.domains,
            Field::Skills => &mut prefs.must_skills,
        }
    }

    fn suggestions(self, all: &PrefSuggestions) -> &Vec<String> {
        match self {
            Field::Locations => &all.locations,
            Field::Domains => &all.domains,
            Field::Skills => &all.skills,
        }
    }

    /// Add `text` to this list of `prefs`; `false` when there was nothing to add (see [`with_item`])
    /// or it is in one of the related lists already (see [`presence`](Self::presence)).
    pub(crate) fn add(self, prefs: &mut Preferences, text: &str) -> bool {
        if self.presence(prefs, text) != Presence::Absent {
            return false;
        }
        match with_item(self.list(prefs), text) {
            Some(list) => {
                *self.list_mut(prefs) = list;
                true
            }
            None => false,
        }
    }

    /// Where `text` stands in `prefs` (ignoring case, with Unicode-aware lowercase like
    /// `Preferences::normalized`): skills count in the must, nice and avoid lists, domains in the
    /// interested and avoided ones.
    pub(crate) fn presence(self, prefs: &Preferences, text: &str) -> Presence {
        let key = item_key(text);
        if key.is_empty() {
            return Presence::Absent;
        }
        let has = |list: &[String]| list.iter().any(|i| i.trim().to_lowercase() == key);
        let (wanted, avoided): (&[&[String]], &[&[String]]) = match self {
            Field::Locations => (&[&prefs.locations], &[]),
            Field::Domains => (&[&prefs.domains], &[&prefs.avoid_domains]),
            Field::Skills => (&[&prefs.must_skills, &prefs.nice_skills], &[&prefs.avoid_skills]),
        };
        if wanted.iter().any(|l| has(l)) {
            Presence::Added
        } else if avoided.iter().any(|l| has(l)) {
            Presence::Avoided
        } else {
            Presence::Absent
        }
    }

    /// Why `text` is not in `prefs` after an attempt to add it: it is in the avoid list, the list
    /// really is full, or something else went wrong.
    pub(crate) fn why_not_added(self, prefs: &Preferences, text: &str) -> String {
        let text = item_text(text);
        if self.presence(prefs, &text) == Presence::Avoided {
            format!("\"{text}\" is in your avoid list, so it was not added.")
        } else if self.list(prefs).len() >= MAX_LIST_ITEMS {
            format!("Could not add \"{text}\": a list of preferences holds up to {MAX_LIST_ITEMS} entries.")
        } else {
            format!("Could not add \"{text}\".")
        }
    }

    /// `text` could be added now: in none of the related lists, and this one is not full.
    fn can_add(self, prefs: &Preferences, text: &str) -> bool {
        self.presence(prefs, text) == Presence::Absent && with_item(self.list(prefs), text).is_some()
    }
}

/// Whether a topic is in the preferences already, and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    /// In none of the lists.
    Absent,
    /// In a list of wanted things (must or nice skills, domains).
    Added,
    /// In a list of things to avoid.
    Avoided,
}

/// What to tell the user who typed `text` into a field when it is in a related list already
/// (`None` when it is not).
fn already_there_message(presence: Presence, text: &str) -> Option<String> {
    let text = item_text(text);
    match presence {
        Presence::Absent => None,
        Presence::Added => Some(format!("\"{text}\" is already in your preferences.")),
        Presence::Avoided => Some(format!("\"{text}\" is in your avoid list, so it was not added.")),
    }
}

/// A list entry as it is stored: trimmed and cut to [`MAX_ITEM_CHARS`].
fn item_text(text: &str) -> String {
    text.trim().chars().take(MAX_ITEM_CHARS).collect()
}

/// What two entries are compared by: [`item_text`] in Unicode lowercase (the same as
/// `Preferences::normalized`, so what the screen calls a duplicate is one there too).
pub(crate) fn item_key(text: &str) -> String {
    item_text(text).to_lowercase()
}

/// `list` plus `text`; `None` when there is nothing to add (blank, already there ignoring case, full).
pub(crate) fn with_item(list: &[String], text: &str) -> Option<Vec<String>> {
    let text = item_text(text);
    let key = text.to_lowercase();
    if text.is_empty() || list.len() >= MAX_LIST_ITEMS || list.iter().any(|i| i.trim().to_lowercase() == key) {
        return None;
    }
    let mut out = list.to_vec();
    out.push(text);
    Some(out)
}

/// `list` without the entries equal to `item` (ignoring case): by value, so a stale click or a
/// list that changed meanwhile never removes another entry.
pub(crate) fn without_item(list: &[String], item: &str) -> Vec<String> {
    let key = item.trim().to_lowercase();
    list.iter().filter(|i| i.trim().to_lowercase() != key).cloned().collect()
}

/// The suggestions to show: the ones that could be added to `field` (not in it or its related
/// lists yet), at most [`SUGGESTION_CHIPS`].
fn visible_suggestions(field: Field, prefs: &Preferences, all: &[String]) -> Vec<String> {
    all.iter().filter(|s| field.can_add(prefs, s)).take(SUGGESTION_CHIPS).cloned().collect()
}

/// Opacity of the "Saved" label at `delta` (0 to 1 over its life): fully visible, then gone.
fn fade(delta: f32) -> f32 {
    if delta <= FLASH_HOLD {
        1.0
    } else {
        (1.0 - (delta - FLASH_HOLD) / (1.0 - FLASH_HOLD)).clamp(0.0, 1.0)
    }
}

/// What to tell the user when the notes were saved but could not be read by the model.
fn unread_notes_message(error: &str) -> String {
    match classify(error) {
        Problem::NoProvider => {
            "Your notes are saved. Set up an AI provider below and the app can also read them for roles, seniority and salary."
                .to_string()
        }
        Problem::Network => "Your notes are saved, but the AI could not be reached to read them. Save them again to retry.".to_string(),
        _ => {
            let short: String = error.chars().take(160).collect();
            format!("Your notes are saved. They could not be read for more details: {short}")
        }
    }
}

/// Said under the notes when the model could not read all of them (it asked a question instead).
const PARTLY_READ: &str = "Your notes are saved; I could not read everything from them.";

/// What the line under the notes says once the model is done with them.
fn notes_status(read: NotesRead) -> NotesStatus {
    match read {
        NotesRead::Read => NotesStatus::Idle,
        NotesRead::PartlyRead => NotesStatus::Failed(PARTLY_READ.to_string()),
        // The failure may be one the user can act on (no provider, no network).
        NotesRead::Failed(message) => NotesStatus::Failed(unread_notes_message(&message)),
        // The user undid a change meanwhile: the read was dropped and there is nothing to say.
        NotesRead::Stale => NotesStatus::Idle,
    }
}

/// What Save does with the notes.
#[derive(Debug, PartialEq, Eq)]
enum SaveNotes {
    /// Nothing changed and nothing failed.
    Nothing,
    /// The text is saved already but the model could not read it last time: read it again.
    ReadAgain,
    /// New text: save it, then let the model read it.
    SaveAndRead,
}

/// `text` is what the box holds (cleaned), `saved` what is stored.
fn save_plan(text: &str, saved: &str, last_read_failed: bool) -> SaveNotes {
    if text != saved {
        SaveNotes::SaveAndRead
    } else if last_read_failed && !text.is_empty() {
        SaveNotes::ReadAgain
    } else {
        SaveNotes::Nothing
    }
}

/// An answer that shows `new` is worth a "Saved" label only when it differs from what is shown
/// (`old`): an edit that changed nothing saved nothing.
fn changed(old: Option<&Preferences>, new: &Preferences) -> bool {
    old != Some(new)
}

/// What the card shows.
struct Snapshot {
    prefs: Preferences,
    suggestions: PrefSuggestions,
    cv: Option<UploadedCv>,
    can_undo: bool,
}

/// The card around `prefs`: the document an edit just returned, so what is shown is that edit's
/// own result.
fn snapshot_of(core: &Core, prefs: Preferences) -> Result<Snapshot> {
    Ok(Snapshot {
        prefs,
        // Suggestions are a nicety: without them the form still works.
        suggestions: core.preference_suggestions().unwrap_or_else(|e| {
            tracing::warn!("preference suggestions: {e:#}");
            PrefSuggestions::default()
        }),
        cv: cv_upload::stored(core)?.map(|(cv, _)| cv),
        can_undo: core.can_undo_preferences()?,
    })
}

fn read_snapshot(core: &Core) -> Result<Snapshot> {
    snapshot_of(core, core.preferences()?)
}

// ---------------------------------------------------------------- view

/// Input widgets; created on the first render because they need a `Window`.
struct Form {
    notes: Entity<TextareaState>,
    /// One "add" box per entry of [`Field::ALL`].
    adds: Vec<Entity<InputState>>,
}

/// What the background reading of the notes is doing.
enum NotesStatus {
    Idle,
    Reading,
    Failed(String),
}

pub struct ProfileSection {
    state: Entity<AppState>,
    form: Option<Form>,
    snap: Option<Snapshot>,
    /// Settings is on screen: cleared when it is left, so the next visit reads everything again.
    shown: bool,
    /// The notes box holds what was last put into it (loaded or saved), so a reload may replace it.
    synced_notes: String,
    /// A CV import or removal is running.
    cv_busy: bool,
    /// A save or undo is running (only the undo button waits for it).
    saving: u32,
    /// Edits started so far: the answer of an edit is shown only while it is the latest started,
    /// and an answer that only reads is dropped once a later edit has started (that edit's own
    /// answer is the newer document).
    seq: u64,
    /// Readings of the notes started so far: only the latest one sets the line under the notes.
    notes_seq: u64,
    /// The label of the last save and a counter that restarts its fade.
    flash: Option<(u64, &'static str)>,
    notes: NotesStatus,
    error: Option<String>,
}

impl ProfileSection {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, state, cx| {
            if state.read(cx).screen != Screen::Settings && this.shown {
                this.shown = false;
            }
        })
        .detach();
        Self {
            state,
            form: None,
            snap: None,
            shown: false,
            synced_notes: String::new(),
            cv_busy: false,
            saving: 0,
            seq: 0,
            notes_seq: 0,
            flash: None,
            notes: NotesStatus::Idle,
            error: None,
        }
    }

    fn core(&self, cx: &App) -> Arc<Core> {
        self.state.read(cx).core.clone()
    }

    fn ensure_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            return;
        }
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("Seniority, salary, visa, the kind of company: say it in your own words.")
        });
        cx.subscribe_in(&notes, window, |this, _, event: &InputEvent, window, cx| match event {
            // Enter adds a line; Ctrl/Cmd+Enter saves.
            InputEvent::PressEnter { secondary: true, .. } => this.save_notes(window, cx),
            InputEvent::Change => cx.notify(),
            _ => {}
        })
        .detach();
        let adds = Field::ALL
            .iter()
            .map(|&field| {
                let input = cx.new(|cx| InputState::new(window, cx).placeholder(field.placeholder()));
                cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, window, cx| {
                    if let InputEvent::PressEnter { .. } = event {
                        this.add_typed(field, window, cx);
                    }
                })
                .detach();
                input
            })
            .collect();
        self.form = Some(Form { notes, adds });
    }

    // ------------------------------------------------------------ reading

    /// Read everything the card shows, off the UI thread.
    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let rx = runtime::spawn(async move { read_snapshot(&core) });
        self.await_snapshot(rx, self.seq, None, window, cx);
    }

    /// Show the snapshot that arrives on `rx`; `flash` is the label to fade in once it is shown.
    /// `seq` is the edit the answer belongs to (see [`seq`](Self::seq)): the snapshot of an older
    /// one is dropped.
    fn await_snapshot(
        &mut self,
        rx: tokio::sync::oneshot::Receiver<Result<Snapshot>>,
        seq: u64,
        flash: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.saving += u32::from(flash.is_some());
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update_in(cx, |this, window, cx| {
                this.saving = this.saving.saturating_sub(u32::from(flash.is_some()));
                match result {
                    Ok(snap) => {
                        this.error = None;
                        this.show(snap, flash, is_latest_read(seq, this.seq), window, cx);
                    }
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Show `snap`, unless `latest` is false (a later edit has started: it will bring a newer
    /// document). `flash` is the label of a change that was really made, even when it is not shown.
    fn show(&mut self, snap: Snapshot, flash: Option<&'static str>, latest: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(label) = flash.filter(|_| changed(self.snap.as_ref().map(|s| &s.prefs), &snap.prefs)) {
            let n = self.flash.map_or(1, |(n, _)| n + 1);
            self.flash = Some((n, label));
            // The setup banner follows whether preferences were saved.
            self.state.update(cx, |s, cx| {
                if !s.prefs_saved {
                    s.prefs_saved = true;
                    cx.notify();
                }
            });
        }
        if !latest {
            return;
        }
        // The notes box follows the saved notes unless the user has typed since it was last synced.
        if let Some(form) = &self.form {
            let typed = form.notes.read(cx).value().to_string();
            if clean_notes(&typed) == clean_notes(&self.synced_notes) && clean_notes(&typed) != snap.prefs.notes {
                let saved = snap.prefs.notes.clone();
                form.notes.update(cx, |input, cx| input.set_value(saved, window, cx));
            }
        }
        self.synced_notes = snap.prefs.notes.clone();
        self.snap = Some(snap);
    }

    // ------------------------------------------------------------ edits

    /// Change the saved preferences with `change` (`true` when it changed something) and show the
    /// document as it is after it. Nothing waits for it and nothing running is cancelled; an edit
    /// made while the model reads the notes waits for that, in the core.
    fn edit(&mut self, change: impl FnOnce(&mut Preferences) -> bool + Send + 'static, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            let prefs = core.edit_preferences(change).await?;
            snapshot_of(&core, prefs)
        });
        self.seq += 1;
        self.await_snapshot(rx, self.seq, Some("Saved"), window, cx);
    }

    fn add_item(&mut self, field: Field, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(snap) = &self.snap else { return };
        let list = field.list(&snap.prefs);
        if list.len() >= MAX_LIST_ITEMS {
            self.error = Some(format!("{} can hold up to {MAX_LIST_ITEMS} entries.", field.label()));
            cx.notify();
            return;
        }
        if with_item(list, &text).is_none() {
            return;
        }
        // In a list the form does not show (nice or avoided skills, avoided domains): say so.
        if let Some(message) = already_there_message(field.presence(&snap.prefs, &text), &text) {
            self.error = Some(message);
            cx.notify();
            return;
        }
        self.error = None;
        self.edit(move |prefs| field.add(prefs, &text), window, cx);
    }

    /// The "Add" box of `field` (Enter or the Add button).
    fn add_typed(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.form.as_ref().and_then(|f| f.adds.get(field as usize)).cloned() else { return };
        let text = input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        input.update(cx, |input, cx| input.set_value("", window, cx));
        self.add_item(field, text, window, cx);
    }

    /// Remove `item` by value (not by position): the list may have changed since the chip was drawn.
    fn remove_item(&mut self, field: Field, item: String, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(
            move |prefs| {
                let kept = without_item(field.list(prefs), &item);
                let removed = kept.len() != field.list(prefs).len();
                *field.list_mut(prefs) = kept;
                removed
            },
            window,
            cx,
        );
    }

    /// Save the notes, then let the model read them in the background.
    fn save_notes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(form), Some(snap)) = (&self.form, &self.snap) else { return };
        let typed = form.notes.read(cx).value().to_string();
        let text = clean_notes(&typed);
        // Ctrl/Cmd+Enter also inserted a line break: drop it.
        if typed != text {
            form.notes.update(cx, |input, cx| input.set_value(text.clone(), window, cx));
        }
        let plan = save_plan(&text, &snap.prefs.notes, matches!(self.notes, NotesStatus::Failed(_)));
        if plan == SaveNotes::Nothing {
            return;
        }
        self.error = None;
        if plan == SaveNotes::SaveAndRead {
            self.synced_notes = text.clone();
            let core = self.core(cx);
            let saved = text.clone();
            let rx = runtime::spawn(async move {
                let prefs = core
                    .edit_preferences(move |prefs| {
                        let changed = prefs.notes != saved;
                        prefs.notes = saved;
                        changed
                    })
                    .await?;
                snapshot_of(&core, prefs)
            });
            self.seq += 1;
            self.await_snapshot(rx, self.seq, Some("Saved"), window, cx);
        }
        if text.is_empty() {
            self.notes_seq += 1;
            self.notes = NotesStatus::Idle;
        } else {
            self.read_notes(text, window, cx);
        }
    }

    /// The model reads `text` once, so it can fill what the form hides (`Core::read_preference_notes`,
    /// which saves the notes first when they changed). Its messages are not shown; anything but a
    /// clean read is a muted line, never a blocker. Reads and edits queue in the core.
    fn read_notes(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.notes_seq += 1;
        let (notes_seq, seq) = (self.notes_seq, self.seq);
        self.notes = NotesStatus::Reading;
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            let read = core.read_preference_notes(text).await?;
            Ok((read_snapshot(&core)?, read))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result: Result<(Snapshot, NotesRead)> = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update_in(cx, |this, window, cx| {
                let newest = is_latest_read(notes_seq, this.notes_seq);
                match result {
                    Ok((snap, read)) => {
                        if newest {
                            this.notes = notes_status(read);
                        }
                        // What the model merged shows, unless an edit started meanwhile (its answer comes later).
                        this.show(snap, None, is_latest_read(seq, this.seq), window, cx);
                    }
                    Err(e) if newest => this.notes = NotesStatus::Failed(unread_notes_message(&format!("{e:#}"))),
                    Err(_) => {}
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            let prefs = match core.undo_preference_edit().await? {
                Some(prev) => prev,
                None => core.preferences()?,
            };
            snapshot_of(&core, prefs)
        });
        // The notes box follows the restored notes.
        self.synced_notes = self.form.as_ref().map(|f| f.notes.read(cx).value().to_string()).unwrap_or_default();
        // A reading that is still going no longer says anything about what is shown.
        self.notes_seq += 1;
        self.notes = NotesStatus::Idle;
        self.seq += 1;
        self.await_snapshot(rx, self.seq, Some("Undone"), window, cx);
    }

    // ------------------------------------------------------------ CV

    fn pick_cv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cv_busy {
            return;
        }
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose your CV (.pdf, .md or .txt)".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let paths = match picked.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(e)) => {
                    this.update(cx, |this, cx| {
                        this.error = Some(format!("The file dialog did not open: {e:#}"));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            this.update_in(cx, |this, window, cx| this.import_cv(paths, window, cx)).ok();
        })
        .detach();
    }

    fn import_cv(&mut self, paths: Vec<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = paths.into_iter().find(|p| cv_upload::is_supported(p)) else {
            self.error = Some("Use a .pdf, .md or .txt file.".into());
            cx.notify();
            return;
        };
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            cv_upload::import(&core, &path).await?;
            read_snapshot(&core)
        });
        self.run_cv(rx, window, cx);
    }

    fn remove_cv(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.cv_busy {
            return;
        }
        let core = self.core(cx);
        let rx = runtime::spawn(async move {
            cv_upload::remove(&core)?;
            read_snapshot(&core)
        });
        self.run_cv(rx, window, cx);
    }

    fn run_cv(&mut self, rx: tokio::sync::oneshot::Receiver<Result<Snapshot>>, window: &mut Window, cx: &mut Context<Self>) {
        self.cv_busy = true;
        self.error = None;
        cx.notify();
        let seq = self.seq;
        cx.spawn_in(window, async move |this, cx| {
            let result = rx.await.unwrap_or_else(|_| Err(anyhow!("the task was cancelled")));
            this.update_in(cx, |this, window, cx| {
                this.cv_busy = false;
                match result {
                    Ok(snap) => this.show(snap, None, is_latest_read(seq, this.seq), window, cx),
                    Err(e) => this.error = Some(format!("{e:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ------------------------------------------------------------ rendering

    /// A labelled row of the card.
    fn row(&self, label: &'static str, hint: Option<&'static str>, body: impl IntoElement, cx: &App) -> Div {
        let muted = cx.theme().muted_foreground;
        div()
            .v_flex()
            .gap_1p5()
            .child(
                div()
                    .h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(div().text_sm().font_semibold().child(label))
                    .children(hint.map(|h| div().text_xs().text_color(muted).child(h))),
            )
            .child(body)
    }

    fn render_cv(&self, cv: Option<&UploadedCv>, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let button = |id: &'static str, label: &'static str| {
            Button::new(id)
                .outline()
                .compact()
                .icon(IconName::FileText)
                .label(label)
                .loading(self.cv_busy)
                .disabled(self.cv_busy)
                .on_click(cx.listener(|this, _, window, cx| this.pick_cv(window, cx)))
        };
        match cv {
            Some(cv) => {
                let cut = if cv.truncated { ", cut to the limit" } else { "" };
                div()
                    .h_flex()
                    .flex_wrap()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .id("profile-cv-name")
                            .role(Role::Paragraph)
                            .aria_label(format!("CV: {}", cv.name))
                            .text_sm()
                            .max_w(px(320.))
                            .truncate()
                            .child(cv.name.clone()),
                    )
                    .child(div().text_xs().text_color(muted).child(format!("{} characters read{cut}", cv.chars)))
                    .child(button("profile-cv-replace", "Replace"))
                    .child(
                        Button::new("profile-cv-remove")
                            .ghost()
                            .compact()
                            .label("Remove")
                            .disabled(self.cv_busy)
                            .on_click(cx.listener(|this, _, window, cx| this.remove_cv(window, cx))),
                    )
            }
            None => div()
                .h_flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(button("profile-cv-upload", "Upload CV"))
                .child(div().text_xs().text_color(muted).child("A .pdf, .md or .txt file.")),
        }
    }

    /// One removable chip.
    fn chip(&self, field: Field, index: usize, item: &str, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        div()
            .id(SharedString::from(format!("profile-{}-{index}", field.key())))
            .role(Role::ListItem)
            .aria_label(item.to_string())
            .h_flex()
            .items_center()
            .gap_0p5()
            .pl_2p5()
            .pr_0p5()
            .py_0p5()
            .rounded_full()
            .bg(theme.secondary)
            .text_color(theme.secondary_foreground)
            .text_sm()
            .child(item.to_string())
            .child(
                Button::new(SharedString::from(format!("profile-{}-remove-{index}", field.key())))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .tooltip(format!("Remove {item}"))
                    .accessibility_label(format!("Remove {item}"))
                    .on_click({
                        let item = item.to_string();
                        cx.listener(move |this, _, window, cx| this.remove_item(field, item.clone(), window, cx))
                    }),
            )
    }

    fn render_field(&self, field: Field, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let Some(input) = self.form.as_ref().and_then(|f| f.adds.get(field as usize)) else { return div() };
        let list = field.list(&snap.prefs);
        let chips: Vec<Stateful<Div>> = list.iter().enumerate().map(|(i, item)| self.chip(field, i, item, cx)).collect();
        let suggestions = visible_suggestions(field, &snap.prefs, field.suggestions(&snap.suggestions));
        let muted = cx.theme().muted_foreground;
        let body = div()
            .v_flex()
            .gap_2()
            .when(!chips.is_empty(), |d| d.child(div().h_flex().flex_wrap().gap_1p5().children(chips)))
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().w(px(260.)).child(
                            Input::new(input)
                                .small()
                                .accessibility_id(format!("profile-add-{}", field.key()))
                                .aria_label(format!("Add to {}", field.label())),
                        ),
                    )
                    .child(
                        Button::new(SharedString::from(format!("profile-{}-add", field.key())))
                            .outline()
                            .compact()
                            .label("Add")
                            .accessibility_label(format!("Add to {}", field.label()))
                            .on_click(cx.listener(move |this, _, window, cx| this.add_typed(field, window, cx))),
                    ),
            )
            .when(!suggestions.is_empty(), |d| {
                d.child(
                    div()
                        .h_flex()
                        .flex_wrap()
                        .items_center()
                        .gap_1()
                        .child(div().text_xs().text_color(muted).pr_1().child("Suggestions"))
                        .children(suggestions.into_iter().enumerate().map(|(i, s)| {
                            let add = s.clone();
                            Button::new(SharedString::from(format!("profile-{}-suggest-{i}", field.key())))
                                .ghost()
                                .compact()
                                .icon(IconName::Plus)
                                .label(s.clone())
                                .accessibility_label(format!("Add {s} to {}", field.label()))
                                .on_click(cx.listener(move |this, _, window, cx| this.add_item(field, add.clone(), window, cx)))
                        })),
                )
            });
        self.row(field.label(), None, body, cx)
    }

    fn render_notes(&self, snap: &Snapshot, cx: &mut Context<Self>) -> Div {
        let muted = cx.theme().muted_foreground;
        let Some(form) = &self.form else { return div() };
        let typed = form.notes.read(cx).value().to_string();
        // Also on when the saved notes could not be read: Save reads them again.
        let can_save = save_plan(&clean_notes(&typed), &snap.prefs.notes, matches!(self.notes, NotesStatus::Failed(_))) != SaveNotes::Nothing;
        let shortcut = if cfg!(target_os = "macos") { "Cmd+Enter" } else { "Ctrl+Enter" };
        let status = match &self.notes {
            NotesStatus::Idle => None,
            NotesStatus::Reading => Some(
                div()
                    .id("profile-notes-reading")
                    .role(Role::Status)
                    .aria_label("Reading your notes")
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .text_color(muted)
                    .child(Spinner::new().xsmall().color(muted))
                    .child("Reading your notes to fill in the rest..."),
            ),
            NotesStatus::Failed(message) => Some(
                div()
                    .id("profile-notes-unread")
                    .role(Role::Status)
                    .aria_label("Notes saved, not read")
                    .text_xs()
                    .text_color(muted)
                    .child(message.clone()),
            ),
        };
        let body = div()
            .v_flex()
            .gap_2()
            .child(
                Textarea::new(&form.notes)
                    .small()
                    .accessibility_id("profile-notes")
                    .aria_label("Anything else"),
            )
            .child(
                div()
                    .h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("profile-notes-save")
                            .outline()
                            .compact()
                            .label("Save")
                            .disabled(!can_save)
                            .on_click(cx.listener(|this, _, window, cx| this.save_notes(window, cx))),
                    )
                    .child(div().text_xs().text_color(muted).child(format!("or {shortcut}"))),
            )
            .children(status);
        self.row("Anything else", Some("Seniority, salary, visa, company type"), body, cx)
    }
}

impl Render for ProfileSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_form(window, cx);
        if !self.shown {
            self.shown = true;
            cx.defer_in(window, |this, window, cx| this.reload(window, cx));
        }
        let (muted, ok, danger) = {
            let t = cx.theme();
            (t.muted_foreground, t.success, t.danger)
        };
        let can_undo = self.snap.as_ref().is_some_and(|s| s.can_undo) && self.saving == 0;
        let flash = self.flash.map(|(n, label)| {
            div()
                .id("profile-saved")
                .role(Role::Status)
                .aria_label(label)
                .text_sm()
                .text_color(ok)
                .child(label)
                .with_animation(
                    ElementId::NamedInteger("profile-saved".into(), n),
                    Animation::new(Duration::from_millis(FLASH_MS)),
                    |label, delta| label.opacity(fade(delta)),
                )
        });
        let actions = div().h_flex().items_center().gap_2().children(flash).child(
            Button::new("profile-undo")
                .ghost()
                .compact()
                .icon(IconName::Undo2)
                .label("Undo")
                .disabled(!can_undo)
                .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))),
        );
        let card = section_with(
            "Profile & preferences",
            "What to look for. Every change is saved at once and used by the next search.",
            actions,
            cx,
        )
        .id("settings-profile")
        .role(Role::Group)
        .aria_label("Profile & preferences")
        .gap_4();

        let Some(snap) = self.snap.take() else {
            return card.child(div().text_sm().text_color(muted).child(match &self.error {
                Some(e) => format!("Could not read your preferences: {e}"),
                None => "Loading...".to_string(),
            }));
        };
        let body = card
            .child(self.row("Your CV", Some("Used only to suggest preferences, never copied into a CV"), self.render_cv(snap.cv.as_ref(), cx), cx))
            .child({
                let this = cx.entity();
                let choices = [Remote::Any, Remote::Remote, Remote::Hybrid, Remote::Onsite].map(|r| (r, r.label()));
                let control = super::segmented("profile-job-type", "Job type", &choices, snap.prefs.remote, move |remote, window, cx| {
                    this.update(cx, |this, cx| {
                        this.edit(
                            move |prefs| {
                                let changed = prefs.remote != remote;
                                prefs.remote = remote;
                                changed
                            },
                            window,
                            cx,
                        )
                    })
                }, cx);
                self.row("Job type", None, control, cx)
            })
            .children(Field::ALL.map(|field| self.render_field(field, &snap, cx)))
            .child(self.render_notes(&snap, cx))
            .children(self.error.clone().map(|e| {
                div().id("profile-error").role(Role::Alert).aria_label("Error").text_sm().text_color(danger).child(e)
            }));
        self.snap = Some(snap);
        body
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: it would pull in gpui's `test` attribute and shadow the standard one.
    use super::{
        already_there_message, changed, fade, item_key, notes_status, save_plan, unread_notes_message, visible_suggestions, with_item,
        without_item, Field, NotesStatus, Presence, SaveNotes, MAX_ITEM_CHARS, PARTLY_READ, SUGGESTION_CHIPS,
    };

    use jobpilot_core::prefs::chat::MAX_LIST_ITEMS;
    use jobpilot_core::prefs::{NotesRead, PrefSuggestions, Preferences, Remote};

    fn strings(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn items_are_added_without_blanks_duplicates_or_overflow() {
        let list = strings(&["Rust"]);
        assert_eq!(with_item(&list, "  Go "), Some(strings(&["Rust", "Go"])));
        assert_eq!(with_item(&list, "rust"), None, "already there, ignoring case");
        assert_eq!(with_item(&list, "   "), None);
        let full: Vec<String> = (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect();
        assert_eq!(with_item(&full, "one more"), None);
        assert_eq!(with_item(&[], &"x".repeat(500)).unwrap_or_default()[0].chars().count(), MAX_ITEM_CHARS);
    }

    #[test]
    fn items_are_compared_with_unicode_lowercase() {
        let list = strings(&["Z\u{fc}rich", "\u{c9}cole"]);
        assert_eq!(with_item(&list, "Z\u{dc}RICH"), None, "the same city in capitals");
        assert_eq!(with_item(&list, " \u{e9}cole "), None);
        assert!(with_item(&list, "Zurich").is_some(), "another spelling is another entry");
        assert_eq!(item_key("  \u{c9}COLE "), "\u{e9}cole");
    }

    #[test]
    fn an_item_is_removed_by_value_not_by_position() {
        assert_eq!(without_item(&strings(&["a", "b"]), "a"), strings(&["b"]));
        assert_eq!(without_item(&strings(&["Rust", "Go"]), " rust "), strings(&["Go"]), "ignoring case and spaces");
        assert_eq!(without_item(&strings(&["a"]), "zzz"), strings(&["a"]), "an item that is gone changes nothing");
        // A stale click: the list lost its first entry since the chip for "b" was drawn.
        let now = strings(&["b", "c"]);
        assert_eq!(without_item(&now, "c"), strings(&["b"]), "removes the entry that was clicked, whatever its place");
        assert_eq!(without_item(&[], "x"), Vec::<String>::new());
    }

    #[test]
    fn a_topic_counts_as_present_in_every_related_list() {
        let prefs = Preferences {
            must_skills: strings(&["Rust"]),
            nice_skills: strings(&["Kafka"]),
            avoid_skills: strings(&["PHP"]),
            domains: strings(&["Fintech"]),
            avoid_domains: strings(&["Gambling"]),
            ..Default::default()
        };
        let skills = |t: &str| Field::Skills.presence(&prefs, t);
        assert_eq!(skills("rust"), Presence::Added);
        assert_eq!(skills(" KAFKA "), Presence::Added, "a nice skill is present too");
        assert_eq!(skills("php"), Presence::Avoided);
        assert_eq!(skills("Go"), Presence::Absent);
        assert_eq!(skills("  "), Presence::Absent);
        let domains = |t: &str| Field::Domains.presence(&prefs, t);
        assert_eq!(domains("FINTECH"), Presence::Added);
        assert_eq!(domains("gambling"), Presence::Avoided);
        assert_eq!(domains("rust"), Presence::Absent, "skills and domains are separate");
        assert_eq!(Field::Locations.presence(&prefs, "Rust"), Presence::Absent);

        // Unicode-aware, like `Preferences::normalized`.
        let accented = Preferences { domains: strings(&["\u{c9}nergie"]), ..Default::default() };
        assert_eq!(Field::Domains.presence(&accented, "\u{e9}NERGIE"), Presence::Added);

        // Adding never duplicates into another list, and never adds what is avoided.
        let mut prefs = prefs;
        assert!(!Field::Skills.add(&mut prefs, "kafka"));
        assert!(!Field::Skills.add(&mut prefs, "PHP"));
        assert!(!Field::Domains.add(&mut prefs, "Gambling"));
        assert!(Field::Skills.add(&mut prefs, "Go"));
        assert_eq!(prefs.must_skills, strings(&["Rust", "Go"]));
        assert_eq!(prefs.nice_skills, strings(&["Kafka"]));
    }

    #[test]
    fn the_reason_a_topic_was_not_added_is_the_real_one() {
        let mut prefs = Preferences { avoid_skills: strings(&["PHP"]), ..Default::default() };
        assert!(Field::Skills.why_not_added(&prefs, "php").contains("avoid list"));
        let other = Field::Skills.why_not_added(&prefs, "Go");
        assert_eq!(other, "Could not add \"Go\".", "not full: no talk of a full list");
        assert!(!other.contains("holds up to"));
        prefs.must_skills = (0..MAX_LIST_ITEMS).map(|i| format!("s{i}")).collect();
        assert!(Field::Skills.why_not_added(&prefs, "Go").contains(&format!("holds up to {MAX_LIST_ITEMS}")));
        assert!(!Field::Domains.why_not_added(&prefs, "Go").contains("holds up to"), "only the list that is full");
        assert_eq!(already_there_message(Presence::Absent, "x"), None);
        assert!(already_there_message(Presence::Added, "x").is_some_and(|m| m.contains("already")));
        assert!(already_there_message(Presence::Avoided, "x").is_some_and(|m| m.contains("avoid list")));
    }

    #[test]
    fn every_field_reads_and_writes_its_own_list() {
        let mut prefs = Preferences::default();
        for field in Field::ALL {
            assert!(field.add(&mut prefs, &format!("item {}", field.key())));
        }
        assert_eq!(prefs.locations, strings(&["item locations"]));
        assert_eq!(prefs.domains, strings(&["item domains"]));
        assert_eq!(prefs.must_skills, strings(&["item skills"]), "skills are the must-have skills");
        assert!(prefs.nice_skills.is_empty() && prefs.avoid_skills.is_empty(), "the others are left alone");
        for field in Field::ALL {
            assert_eq!(field.presence(&prefs, &format!(" ITEM {} ", field.key())), Presence::Added);
            assert!(!field.add(&mut prefs, &format!("item {}", field.key())), "no duplicate");
        }
        let keys: std::collections::HashSet<_> = Field::ALL.iter().map(|f| f.key()).collect();
        assert_eq!(keys.len(), Field::ALL.len(), "element ids stay unique");
        // `add_typed` finds the box of a field by its position.
        for (ix, field) in Field::ALL.iter().enumerate() {
            assert_eq!(*field as usize, ix);
        }
    }

    #[test]
    fn suggestions_skip_what_is_saved_and_are_capped() {
        let all: Vec<String> = (0..20).map(|i| format!("s{i}")).collect();
        let mut prefs = Preferences { must_skills: strings(&["S0", "s3"]), ..Default::default() };
        let shown = visible_suggestions(Field::Skills, &prefs, &all);
        assert_eq!(shown.len(), SUGGESTION_CHIPS);
        assert!(!shown.contains(&"s0".to_string()) && !shown.contains(&"s3".to_string()));
        assert_eq!(shown[0], "s1");
        // A nice or avoided skill is not suggested again either.
        prefs.nice_skills = strings(&["s1"]);
        prefs.avoid_skills = strings(&["S2"]);
        assert_eq!(visible_suggestions(Field::Skills, &prefs, &all)[0], "s4");
        let full = Preferences { must_skills: all.clone(), ..Default::default() };
        assert!(visible_suggestions(Field::Skills, &full, &all).is_empty());
        let none = PrefSuggestions::default();
        assert!(Field::Skills.suggestions(&none).is_empty());
    }

    #[test]
    fn the_saved_label_stays_then_fades_out() {
        assert_eq!(fade(0.0), 1.0);
        assert_eq!(fade(0.5), 1.0);
        assert!(fade(0.8) < 1.0 && fade(0.8) > 0.0);
        assert_eq!(fade(1.0), 0.0);
        assert!((0.0..=1.0).contains(&fade(7.0)), "never outside 0..=1");
    }

    #[test]
    fn what_the_model_did_with_the_notes_becomes_the_line_under_them() {
        assert!(matches!(notes_status(NotesRead::Read), NotesStatus::Idle), "a clean read says nothing");
        match notes_status(NotesRead::PartlyRead) {
            NotesStatus::Failed(line) => assert_eq!(line, PARTLY_READ),
            _ => panic!("a question nobody can answer here is a muted line"),
        }
        match notes_status(NotesRead::Failed("no AI provider is set up".into())) {
            NotesStatus::Failed(line) => assert!(line.contains("AI provider"), "an actionable failure keeps its advice: {line}"),
            _ => panic!("a failure is a muted line"),
        }
        match notes_status(NotesRead::Failed("odd".into())) {
            NotesStatus::Failed(line) => assert!(line.starts_with("Your notes are saved"), "{line}"),
            _ => panic!("a failure is a muted line"),
        }
        assert!(matches!(notes_status(NotesRead::Stale), NotesStatus::Idle), "a read dropped by an undo says nothing");
        assert_eq!(PARTLY_READ, "Your notes are saved; I could not read everything from them.");
    }

    #[test]
    fn saving_unchanged_notes_reads_them_again_only_after_a_failed_read() {
        assert_eq!(save_plan("new", "old", false), SaveNotes::SaveAndRead);
        assert_eq!(save_plan("new", "old", true), SaveNotes::SaveAndRead, "new text is saved and read whatever happened before");
        assert_eq!(save_plan("same", "same", false), SaveNotes::Nothing, "nothing changed, nothing failed: Save does nothing");
        assert_eq!(save_plan("same", "same", true), SaveNotes::ReadAgain, "retry: the text is saved, the read runs again");
        assert_eq!(save_plan("", "", true), SaveNotes::Nothing, "empty notes have nothing to read");
        assert_eq!(save_plan("", "old", false), SaveNotes::SaveAndRead, "clearing the notes is a change");
    }

    #[test]
    fn an_edit_that_changed_nothing_does_not_say_saved() {
        let shown = Preferences { remote: Remote::Remote, ..Default::default() };
        assert!(!changed(Some(&shown), &shown.clone()));
        assert!(changed(Some(&shown), &Preferences::default()));
        assert!(changed(None, &shown), "the first answer is always news");
    }

    #[test]
    fn an_unread_note_says_what_to_do() {
        assert!(unread_notes_message("something odd").starts_with("Your notes are saved."));
        let long = unread_notes_message(&"e".repeat(500));
        assert!(long.chars().count() < 300, "the error is cut");
    }
}
