//! CV tab: the requirement checklist and fact-check cards with the YAML editor on the left, the page
//! preview framed like paper on the right, and the "Ask AI" box below.
//!
//! Editing: every change restarts an 800 ms debounce; when it fires the YAML is written to `cv.yaml`
//! and `pipeline::rerender` runs (parse, fact guard, Typst). Typst/YAML errors show in red, guard
//! violations in amber, and the previous preview stays until a render succeeds.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui_kit::component::input::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::pipeline::{self, Analysis, Violation};
use jobpilot_core::{runtime, Core};

use crate::app::jobs::{load_cv_files, page_files, parse_gaps, CvFiles, Debounce, RenderQueue, EDIT_DEBOUNCE};
use crate::app::AppState;
use crate::ui::{self, Tone};

/// Share of the row taken by the left column (checklist, cards, editor).
const EDITOR_SHARE: f32 = 0.42;
/// Width of the sidebar (`sidebar.rs`) and the Detail screen's horizontal padding, for sizing the pages.
/// A page image needs a definite width (a percentage width collapses its aspect ratio), so the pane
/// width is computed from the window.
const CHROME_WIDTH: f32 = 224.0 + 1.0 + 2.0 * 32.0;
/// Gap between the two columns, padding of the preview pane and of a sheet (with its border).
const COLUMN_GAP: f32 = 16.0;
const PANE_PADDING: f32 = 2.0 * 16.0;
const SHEET_PADDING: f32 = 2.0 * 8.0 + 2.0;
/// Widest a page is drawn (px).
const PAGE_MAX_WIDTH: f32 = 620.0;

/// Result of saving and re-rendering the CV.
#[derive(Debug)]
struct Rendered {
    pages: Vec<Vec<u8>>,
    warnings: Vec<String>,
}

pub struct CvTab {
    state: Entity<AppState>,
    editor: Entity<EditorState>,
    ask_ai: Entity<InputState>,
    /// Application id and the `AppState` CV epoch whose files are loaded.
    loaded: Option<(i64, u64)>,
    load_seq: u64,
    has_cv: bool,
    pages: Vec<Arc<Image>>,
    /// Edited but not yet written to disk.
    dirty: bool,
    debounce: Debounce,
    queue: RenderQueue,
    /// Red: the CV does not parse or compile.
    problem: Option<String>,
    /// Amber: fact-guard findings (the edit is kept, the user decides).
    warnings: Vec<String>,
    /// The fact guard ran on the CV shown and found nothing (it is also false while unknown: no CV, no
    /// readable profile, or a render that failed).
    guard_clean: bool,
    /// Failure to open `profile.yaml`.
    open_error: Option<String>,
}

impl CvTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)).detach();

        let editor = cx.new(|cx| EditorState::new(window, cx).language("yaml").soft_wrap(true));
        cx.subscribe_in(&editor, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::Change) {
                this.on_edit(window, cx);
            }
        })
        .detach();

        let ask_ai = cx.new(|cx| InputState::new(window, cx).placeholder("Ask AI: \"shorter, stress Rust\" (Enter)"));
        cx.subscribe_in(&ask_ai, window, |this, input, event: &InputEvent, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                let instruction = input.read(cx).value().trim().to_string();
                if !instruction.is_empty() && this.ask(instruction, window, cx) {
                    input.update(cx, |input, cx| input.set_value("", window, cx));
                }
            }
        })
        .detach();

        let mut tab = Self {
            state,
            editor,
            ask_ai,
            loaded: None,
            load_seq: 0,
            has_cv: false,
            pages: Vec::new(),
            dirty: false,
            debounce: Debounce::default(),
            queue: RenderQueue::default(),
            problem: None,
            warnings: Vec::new(),
            guard_clean: false,
            open_error: None,
        };
        tab.sync(window, cx);
        tab
    }

    /// The CV does not parse or compile right now (the Approve button waits for a fix).
    pub fn has_error(&self) -> bool {
        self.problem.is_some()
    }

    /// Follow the selection and the pipeline: load the CV files of the selected application whenever
    /// the application or its CV epoch changes.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (app, epoch) = {
            let state = self.state.read(cx);
            let app = state.selected_app().cloned();
            let epoch = app.as_ref().map(|a| state.cv_epoch(a.id)).unwrap_or(0);
            (app, epoch)
        };
        let key = app.as_ref().map(|a| (a.id, epoch));
        if key == self.loaded {
            return;
        }
        // Do not lose an edit that is still waiting for its debounce when another application is picked.
        if self.dirty && self.loaded.map(|l| l.0) != key.map(|k| k.0) {
            self.flush_in_background(cx);
        }
        self.loaded = key;
        self.debounce.cancel();
        self.dirty = false;
        self.problem = None;
        self.warnings.clear();
        self.guard_clean = false;
        self.open_error = None;
        match app {
            Some(app) => self.load_files(app, window, cx),
            None => self.apply_files(CvFiles::default(), window, cx),
        }
        cx.notify();
    }

    /// Write the pending edit of the application that is being left (and refresh its preview).
    fn flush_in_background(&mut self, cx: &mut Context<Self>) {
        let Some((id, _)) = self.loaded else { return };
        let text = self.editor.read(cx).value().to_string();
        let core = self.state.read(cx).core.clone();
        drop(runtime::spawn(async move { save_and_render(&core, id, text).await }));
    }

    fn load_files(&mut self, app: jobpilot_core::store::Application, window: &mut Window, cx: &mut Context<Self>) {
        self.load_seq += 1;
        let seq = self.load_seq;
        let core = self.state.read(cx).core.clone();
        let rx = runtime::spawn(async move {
            let files = load_cv_files(&app);
            let clean = guard_clean(&core, files.yaml.as_deref());
            (files, clean)
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok((files, clean)) = rx.await else { return };
            this.update_in(cx, |this, window, cx| {
                if this.load_seq == seq {
                    this.guard_clean = clean;
                    this.apply_files(files, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn apply_files(&mut self, files: CvFiles, window: &mut Window, cx: &mut Context<Self>) {
        self.has_cv = files.yaml.is_some();
        let needs_render = files.yaml.is_some() && files.stale;
        let text = files.yaml.unwrap_or_default();
        self.editor.update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.set_pages(files.pages, cx);
        // A CV without (up to date) previews, e.g. copied in by hand or edited and never rendered: render it once.
        if needs_render {
            self.request_render(window, cx);
        }
        cx.notify();
    }

    /// Replace the preview. GPUI caches decoded images by their id (a hash of the bytes), so a new
    /// render is a new image, and the old ones are dropped from the cache.
    fn set_pages(&mut self, pages: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        for old in self.pages.drain(..) {
            old.remove_asset(cx);
        }
        self.pages = pages.into_iter().map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes))).collect();
    }

    // ------------------------------------------------------------ editing

    fn on_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dirty = true;
        let token = self.debounce.touch();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(EDIT_DEBOUNCE).await;
            this.update_in(cx, |this, window, cx| {
                if this.debounce.is_latest(token) {
                    this.request_render(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn request_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.queue.request() {
            self.start_render(window, cx);
        }
    }

    fn start_render(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((id, _)) = self.loaded else {
            self.queue = RenderQueue::default();
            return;
        };
        let text = self.editor.read(cx).value().to_string();
        let core = self.state.read(cx).core.clone();
        self.dirty = false;
        let rx = runtime::spawn(async move { save_and_render(&core, id, text).await });
        cx.spawn_in(window, async move |this, cx| {
            let outcome = match rx.await {
                Ok(outcome) => outcome,
                Err(_) => Err(anyhow::anyhow!("the render was cancelled")),
            };
            this.update_in(cx, |this, window, cx| this.render_done(id, outcome, window, cx)).ok();
        })
        .detach();
        cx.notify();
    }

    fn render_done(&mut self, id: i64, outcome: Result<Rendered>, window: &mut Window, cx: &mut Context<Self>) {
        let again = self.queue.finish();
        // The result belongs to the application that was being edited; drop it if another is shown.
        if self.loaded.map(|l| l.0) == Some(id) {
            match outcome {
                Ok(rendered) => {
                    self.problem = None;
                    self.guard_clean = rendered.warnings.is_empty();
                    self.warnings = rendered.warnings;
                    self.has_cv = true;
                    self.set_pages(rendered.pages, cx);
                }
                Err(e) => {
                    self.problem = Some(format!("{e:#}"));
                    self.warnings.clear();
                    self.guard_clean = false;
                }
            }
        }
        if again {
            self.start_render(window, cx);
        }
        cx.notify();
    }

    // ------------------------------------------------------------ Ask AI

    /// Save the edit that is waiting, then run the revision. `false` when nothing was started.
    fn ask(&mut self, instruction: String, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some((id, _)) = self.loaded else { return false };
        if self.state.read(cx).is_busy(id) || !self.has_cv {
            return false;
        }
        self.debounce.cancel();
        self.dirty = false;
        let text = self.editor.read(cx).value().to_string();
        let core = self.state.read(cx).core.clone();
        // `revise` starts from the CV on disk, so write the editor text first.
        let rx = runtime::spawn(async move {
            let app = core.store.get_application(id)?.context("the application no longer exists")?;
            tokio::fs::write(app.cv_yaml_path(), text).await.context("saving cv.yaml")
        });
        cx.spawn_in(window, async move |this, cx| {
            let saved = rx.await;
            this.update_in(cx, |this, _, cx| match saved {
                Ok(Ok(())) => this.state.update(cx, |state, cx| state.revise(id, instruction, cx)),
                Ok(Err(e)) => this.problem = Some(format!("{e:#}")),
                Err(_) => this.problem = Some("the task was cancelled".into()),
            })
            .ok();
        })
        .detach();
        true
    }

    fn open_profile(&mut self, cx: &mut Context<Self>) {
        let path = self.state.read(cx).core.profile_path();
        let rx = runtime::spawn(async move { open_path(path) });
        cx.spawn(async move |this, cx| {
            let result = rx.await;
            this.update(cx, |this, cx| {
                this.open_error = match result {
                    Ok(Ok(())) => None,
                    Ok(Err(e)) => Some(format!("{e:#}")),
                    Err(_) => Some("the task was cancelled".into()),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// `xdg-open <path>` (`open` on macOS): hand the file to the user's default editor.
fn open_path(path: PathBuf) -> Result<()> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    std::process::Command::new(opener)
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {opener} on {}", path.display()))?;
    Ok(())
}

/// Write `cv.yaml`, then `pipeline::rerender` (parse, fact guard, Typst) and read the new previews.
async fn save_and_render(core: &Core, id: i64, yaml: String) -> Result<Rendered> {
    let app = core.store.get_application(id)?.context("the application no longer exists")?;
    tokio::fs::create_dir_all(&app.dir).await.with_context(|| format!("creating {}", app.dir.display()))?;
    tokio::fs::write(app.cv_yaml_path(), yaml).await.context("saving cv.yaml")?;
    let outcome = pipeline::rerender(core, id).await?;
    let mut pages = Vec::new();
    for path in page_files(&app.dir) {
        pages.push(tokio::fs::read(&path).await.with_context(|| format!("reading {}", path.display()))?);
    }
    Ok(Rendered { pages, warnings: violation_lines(&outcome.violations) })
}

fn violation_lines(violations: &[Violation]) -> Vec<String> {
    violations.iter().map(|v| v.to_string()).collect()
}

/// Does the stored CV pass the fact guard against the profile? `false` when it cannot be told (no
/// CV, an unreadable or invalid profile, a CV that does not parse). Blocking: run it with `runtime::spawn`.
fn guard_clean(core: &Core, yaml: Option<&str>) -> bool {
    yaml.is_some_and(|yaml| core.cv_is_traced(yaml))
}

/// One line of the "What they ask for" checklist.
#[derive(Debug, PartialEq, Eq)]
struct Requirement {
    text: String,
    /// From the nice-to-have list (not required).
    nice: bool,
    /// The gap line that says the profile has no evidence for it; `None` = covered.
    gap: Option<String>,
}

/// The words of a text, lower case; `+` and `#` stay (C++, C#).
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '+' || c == '#'))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// What a gap line is about: the part before the first colon (`Kafka: nothing shows it` -> `Kafka`).
fn gap_subject(gap: &str) -> &str {
    gap.split(':').next().unwrap_or(gap)
}

/// Do a requirement and a gap line talk about the same thing? (all the words of one are in the other,
/// comparing the requirement with the line's subject)
fn same_topic(requirement: &str, gap: &str) -> bool {
    let (a, b) = (words(requirement), words(gap_subject(gap)));
    !a.is_empty() && !b.is_empty() && (a.iter().all(|w| b.contains(w)) || b.iter().all(|w| a.contains(w)))
}

/// The JD's requirements (must have, then nice to have) with the gaps the tailoring step reported:
/// a requirement is covered unless a gap line is about it. Gap lines that match no requirement are
/// listed too, so none is lost. Empty when there is nothing to list.
fn requirements(analysis: Option<&Analysis>, gaps: &[String]) -> Vec<Requirement> {
    let mut rows: Vec<Requirement> = Vec::new();
    if let Some(a) = analysis {
        let lists = [(&a.must_have, false), (&a.nice_to_have, true)];
        for (list, nice) in lists {
            for text in list.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
                if rows.iter().any(|r| r.text.eq_ignore_ascii_case(text)) {
                    continue;
                }
                let gap = gaps.iter().find(|g| same_topic(text, g)).cloned();
                rows.push(Requirement { text: text.to_string(), nice, gap });
            }
        }
    }
    for gap in gaps {
        let listed = rows.iter().any(|r| r.gap.as_deref() == Some(gap.as_str()) || same_topic(&r.text, gap));
        if !listed {
            rows.push(Requirement { text: gap.clone(), nice: false, gap: Some(gap.clone()) });
        }
    }
    rows
}

impl Render for CvTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (app, busy) = {
            let state = self.state.read(cx);
            let app = state.selected_app().cloned();
            let busy = app.as_ref().is_some_and(|a| state.is_busy(a.id));
            (app, busy)
        };
        let gaps = parse_gaps(app.as_ref().and_then(|a| a.gaps_json.as_deref()));
        let analysis = app
            .as_ref()
            .and_then(|a| a.analysis_json.as_deref())
            .and_then(|j| serde_json::from_str::<Analysis>(j).ok());
        // The checklist needs the tailoring step's gap list (written with the CV): without it nothing
        // can be called covered.
        let checked = app.as_ref().is_some_and(|a| a.gaps_json.is_some()) && self.has_cv;
        let rows = if checked { requirements(analysis.as_ref(), &gaps) } else { Vec::new() };
        let rendering = self.queue.is_running();
        let p = ui::palette(cx);
        let radius = px(ui::tokens::RADIUS_CARD - 2.0);

        let pane_title = |title: &'static str, note: Option<&'static str>| {
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .child(div().text_size(px(ui::tokens::TEXT_BODY)).font_semibold().text_color(p.ink).child(title))
                .when_some(note, |d, note| d.child(ui::caption(note, cx)))
        };

        let editor_note = if busy && self.has_cv {
            Some("locked while the AI works")
        } else if rendering {
            Some("rendering...")
        } else if self.dirty {
            Some("editing...")
        } else {
            None
        };

        // ---- left column: what the JD asks for, the fact check, then the YAML

        let mut aside: Vec<AnyElement> = Vec::new();

        if !rows.is_empty() {
            let checklist = ui::section(
                "What they ask for",
                "From the job description, checked against your profile.",
                cx,
            )
            .child(div().id("cv-gaps").v_flex().gap_2p5().children(rows.iter().enumerate().map(|(i, r)| {
                let (icon, color, label) = match r.gap {
                    None => (IconName::Check, p.ok, "Covered"),
                    Some(_) => (IconName::Minus, p.warn, "Not in your profile"),
                };
                let note = r.gap.as_ref().map(|g| {
                    if g.eq_ignore_ascii_case(&r.text) {
                        "Nothing in profile.yaml shows it".to_string()
                    } else {
                        g.clone()
                    }
                });
                div()
                    .h_flex()
                    .items_start()
                    .gap_2p5()
                    .child(
                        div()
                            .id(("requirement", i))
                            .role(Role::Image)
                            .aria_label(label)
                            .flex_shrink_0()
                            .pt_0p5()
                            .child(Icon::new(icon).size_4().text_color(color)),
                    )
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .h_flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .text_size(px(ui::tokens::TEXT_BODY))
                                    .text_color(p.ink)
                                    .child(r.text.clone())
                                    .when(r.nice, |d| d.child(ui::tag("Nice to have", Tone::Neutral, cx))),
                            )
                            .when_some(note, |d, note| d.child(ui::caption(note, cx))),
                    )
            })))
            .when(!gaps.is_empty(), |d| {
                d.child(
                    div().h_flex().child(
                        ui::ghost_button("open-profile", "Open profile.yaml")
                            .icon(IconName::ExternalLink)
                            .on_click(cx.listener(|this, _, _, cx| this.open_profile(cx))),
                    ),
                )
            })
            .when_some(self.open_error.clone(), |d, e| d.child(div().text_size(px(ui::tokens::TEXT_SMALL)).text_color(p.danger).child(e)));
            aside.push(checklist.into_any_element());
        }

        // Green only when the fact guard ran on this very CV and found nothing.
        if self.guard_clean && self.has_cv && self.problem.is_none() && self.warnings.is_empty() && !busy {
            aside.push(
                ui::banner(
                    Tone::Ok,
                    "Every fact traced to your profile",
                    "Employers, titles, dates and technologies all match profile.yaml.",
                    div(),
                    cx,
                )
                .into_any_element(),
            );
        }

        if let Some(problem) = self.problem.clone() {
            // A missing typst gets its install hint and a "Check again" button next to the error.
            let help = crate::problem::help("cv-problem", jobpilot_core::health::classify(&problem), &self.state, None, cx);
            let colors = p.tone(Tone::Danger);
            aside.push(
                div()
                    .v_flex()
                    .gap_2()
                    .p_3()
                    .rounded(radius)
                    .border_1()
                    .border_color(colors.dot.opacity(0.35))
                    .bg(colors.bg)
                    .child(
                        div()
                            .id("cv-problem")
                            .max_h(px(110.))
                            .overflow_y_scroll()
                            .font_family("monospace")
                            .text_size(px(ui::tokens::TEXT_SMALL))
                            .text_color(colors.fg)
                            .child(problem),
                    )
                    .children(help)
                    .into_any_element(),
            );
        }

        if !self.warnings.is_empty() {
            let colors = p.tone(Tone::Warn);
            aside.push(
                div()
                    .id("cv-warnings")
                    .v_flex()
                    .gap_1()
                    .max_h(px(120.))
                    .overflow_y_scroll()
                    .p_3()
                    .rounded(radius)
                    .border_1()
                    .border_color(colors.dot.opacity(0.35))
                    .bg(colors.bg)
                    .text_size(px(ui::tokens::TEXT_SMALL))
                    .text_color(colors.fg)
                    .child(div().font_semibold().child("Not backed by profile.yaml (kept, check before approving):"))
                    .children(self.warnings.iter().map(|w| div().font_family("monospace").child(w.clone())))
                    .into_any_element(),
            );
        }

        let editor_card = ui::card(cx)
            .flex_1()
            .flex_shrink_0()
            .min_h(px(340.))
            .gap_2()
            .child(pane_title("cv.yaml", editor_note))
            .child(div().flex_1().min_h_0().child(if self.has_cv {
                Editor::new(&self.editor).disabled(busy).h_full().into_any_element()
            } else {
                div()
                    .p_2()
                    .text_size(px(ui::tokens::TEXT_BODY))
                    .text_color(p.muted)
                    .child(if busy { "The CV is being written..." } else { "No CV yet: Generate creates one from the profile." })
                    .into_any_element()
            }));

        // The whole column scrolls when the cards and the editor do not fit; the editor takes what is left.
        let left = div()
            .id("cv-left")
            .v_flex()
            .w(relative(EDITOR_SHARE))
            .flex_shrink_0()
            .min_h_0()
            .gap_3()
            .overflow_y_scroll()
            .children(aside)
            .child(editor_card);

        // ---- right column: the CV on paper. A page is always a white sheet with dark text, in both appearances.
        let sheet = ui::tokens::LIGHT.surface;
        let sheet_border = ui::tokens::LIGHT.border;
        let sheet_ink = ui::tokens::LIGHT.muted;
        // Pages are as wide as the preview pane: window - chrome, the share left of the first column, less paddings.
        let content = f32::from(window.viewport_size().width) - CHROME_WIDTH;
        let page_width = (content * (1.0 - EDITOR_SHARE) - COLUMN_GAP - PANE_PADDING - SHEET_PADDING)
            .clamp(120.0, PAGE_MAX_WIDTH);
        let paper = || {
            div()
                .v_flex()
                .w(px(page_width + SHEET_PADDING))
                .flex_shrink_0()
                .overflow_hidden()
                .p_2()
                .bg(sheet)
                .border_1()
                .border_color(sheet_border)
                .shadow_sm()
        };
        let preview_pane = div()
            .v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .gap_2()
            .p_4()
            .rounded(px(ui::tokens::RADIUS_CARD))
            .bg(p.track)
            .child(pane_title("Preview", None))
            .child(
                div()
                    .id("cv-preview")
                    .v_flex()
                    .items_center()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .overflow_y_scroll()
                    .children(self.pages.iter().map(|page| paper().child(img(page.clone()).w(px(page_width)).flex_shrink_0())))
                    .when(self.pages.is_empty(), |d| {
                        d.child(
                            paper().min_h(px(240.)).p_6().font_family(ui::tokens::FONT_SERIF).text_color(sheet_ink).child(
                                if rendering { "Rendering..." } else { "No preview yet." },
                            ),
                        )
                    }),
            );

        div()
            .v_flex()
            .size_full()
            .gap_3()
            .child(div().h_flex().items_stretch().flex_1().min_h_0().gap_4().child(left).child(preview_pane))
            .child(Input::new(&self.ask_ai).disabled(busy || !self.has_cv))
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::pipeline::Analysis;
    use jobpilot_core::store::NewApplication;
    use jobpilot_core::{runtime, Core, Paths, Secrets};

    use super::{requirements, same_topic, save_and_render, Requirement};

    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        core.set_profile_path(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profile.yaml")).unwrap();
        (dir, core)
    }

    fn app(core: &Core) -> i64 {
        core.store
            .create_application(&NewApplication {
                slug: "acme".into(),
                company: "Acme".into(),
                role: "Engineer".into(),
                dir: core.paths.application_dir("acme"),
                ..Default::default()
            })
            .unwrap()
            .id
    }

    #[test]
    fn saving_renders_the_edit_and_reports_problems() {
        let (_dir, core) = core();
        let id = app(&core);

        // A valid CV: written, rendered, previews read back.
        let ok = runtime::block_on(save_and_render(&core, id, EXAMPLE.to_string())).unwrap();
        assert!(!ok.pages.is_empty());
        assert!(ok.pages[0].starts_with(b"\x89PNG"));
        let app = core.store.get_application(id).unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), EXAMPLE);

        // An invented employer is kept and rendered, but flagged (amber).
        let invented = EXAMPLE.replacen("company: ", "company: Made Up Corp #", 1);
        let flagged = runtime::block_on(save_and_render(&core, id, invented)).unwrap();
        assert!(!flagged.warnings.is_empty(), "guard findings are warnings: {:?}", flagged.warnings);

        // Broken YAML is an error (red) and the edit is still saved.
        let err = runtime::block_on(save_and_render(&core, id, "basics: [".into())).unwrap_err();
        assert!(!format!("{err:#}").is_empty());
        assert_eq!(std::fs::read_to_string(app.cv_yaml_path()).unwrap(), "basics: [");
    }

    fn analysis(must: &[&str], nice: &[&str]) -> Analysis {
        Analysis {
            must_have: must.iter().map(|s| s.to_string()).collect(),
            nice_to_have: nice.iter().map(|s| s.to_string()).collect(),
            ..Analysis::default()
        }
    }

    fn gaps(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_requirement_is_covered_unless_a_gap_is_about_it() {
        let a = analysis(&["Rust", "PostgreSQL", "Event streaming (Kafka)"], &["Terraform"]);
        let rows = requirements(Some(&a), &gaps(&["kafka", "Terraform: not in the profile"]));
        let covered: Vec<(&str, bool, bool)> = rows.iter().map(|r| (r.text.as_str(), r.nice, r.gap.is_none())).collect();
        assert_eq!(
            covered,
            [("Rust", false, true), ("PostgreSQL", false, true), ("Event streaming (Kafka)", false, false), ("Terraform", true, false)]
        );
        assert_eq!(rows[2].gap.as_deref(), Some("kafka"));
    }

    #[test]
    fn matching_is_by_whole_words() {
        assert!(same_topic("Event streaming (Kafka)", "kafka"));
        assert!(same_topic("Terraform", "Terraform: not in the profile"));
        assert!(same_topic("Event streaming (Kafka)", "Kafka: nothing in the profile shows it"));
        // Only the subject counts, not the explanation after the colon.
        assert!(!same_topic("Rust", "Haskell: nothing like Rust in the profile"));
        assert!(!same_topic("Go", "Google Cloud"));
        assert!(!same_topic("C++", "C#"));
        assert!(!same_topic("", "Rust"));
    }

    #[test]
    fn gap_lines_that_match_no_requirement_are_still_listed() {
        let a = analysis(&["Rust"], &[]);
        let rows = requirements(Some(&a), &gaps(&["Haskell"]));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1], Requirement { text: "Haskell".into(), nice: false, gap: Some("Haskell".into()) });
        // No analysis at all: the gaps alone are the list.
        assert_eq!(requirements(None, &gaps(&["Haskell"])).len(), 1);
    }

    #[test]
    fn nothing_to_list_gives_an_empty_checklist() {
        assert!(requirements(None, &[]).is_empty());
        assert!(requirements(Some(&analysis(&[], &[])), &[]).is_empty());
        // Blank and repeated requirements are dropped.
        let a = analysis(&["Rust", " ", "rust"], &["RUST"]);
        assert_eq!(requirements(Some(&a), &[]).len(), 1);
    }
}
