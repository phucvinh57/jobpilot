//! CV tab: YAML editor and page preview side by side, the "Ask AI" box, fact-check warnings and gaps.
//!
//! Editing: every change restarts an 800 ms debounce; when it fires the YAML is written to `cv.yaml`
//! and `pipeline::rerender` runs (parse, fact guard, Typst). Typst/YAML errors show in red, guard
//! violations in amber, and the previous preview stays until a render succeeds.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui_kit::component::button::*;
use gpui_kit::component::input::*;
use gpui_kit::component::input::InputEvent;
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jobpilot_core::pipeline::{self, Violation};
use jobpilot_core::{runtime, Core};

use crate::app::jobs::{load_cv_files, page_files, parse_gaps, CvFiles, Debounce, RenderQueue, EDIT_DEBOUNCE};
use crate::app::AppState;

/// Width of the sidebar and the detail pane's horizontal padding, for sizing the preview.
const CHROME_WIDTH: f32 = 260.0 + 32.0;
/// Share of the row taken by the editor.
const EDITOR_SHARE: f32 = 0.42;

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
        let rx = runtime::spawn(async move { load_cv_files(&app) });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(files) = rx.await else { return };
            this.update_in(cx, |this, window, cx| {
                if this.load_seq == seq {
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
                    self.warnings = rendered.warnings;
                    self.has_cv = true;
                    self.set_pages(rendered.pages, cx);
                }
                Err(e) => {
                    self.problem = Some(format!("{e:#}"));
                    self.warnings.clear();
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

impl Render for CvTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (app, busy) = {
            let state = self.state.read(cx);
            let app = state.selected_app().cloned();
            let busy = app.as_ref().is_some_and(|a| state.is_busy(a.id));
            (app, busy)
        };
        let gaps = parse_gaps(app.as_ref().and_then(|a| a.gaps_json.as_deref()));
        let rendering = self.queue.is_running();
        let theme = cx.theme();
        let muted = theme.muted_foreground;

        // Page images are as wide as the preview pane: window - sidebar - paddings - editor - frame.
        let content = f32::from(window.viewport_size().width) - CHROME_WIDTH - 12.0;
        let preview_width = px((content * (1.0 - EDITOR_SHARE) - 2.0 - 16.0 - 14.0).max(120.0));

        let pane_title = |title: &'static str, note: Option<&'static str>| {
            div()
                .h_flex()
                .items_center()
                .justify_between()
                .text_xs()
                .child(div().font_semibold().child(title))
                .when_some(note, |d, note| d.child(div().text_color(muted).child(note)))
        };
        let frame = || {
            div().v_flex().min_w_0().h_full().gap_1().p_2().border_1().border_color(theme.border).rounded_md()
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

        let editor_pane = frame()
            .w(relative(EDITOR_SHARE))
            .flex_shrink_0()
            .child(pane_title("cv.yaml", editor_note))
            .child(div().flex_1().min_h_0().child(if self.has_cv {
                Editor::new(&self.editor).disabled(busy).h_full().into_any_element()
            } else {
                div()
                    .p_2()
                    .text_sm()
                    .text_color(muted)
                    .child(if busy { "The CV is being written..." } else { "No CV yet: Generate creates one from the profile." })
                    .into_any_element()
            }));

        let preview_pane = frame().flex_1().child(pane_title("Preview", None)).child(
            div()
                .id("cv-preview")
                .v_flex()
                .flex_1()
                .min_h_0()
                .gap_2()
                .overflow_y_scroll()
                .children(self.pages.iter().map(|page| {
                    img(page.clone()).w(preview_width).border_1().border_color(theme.border).flex_shrink_0()
                }))
                .when(self.pages.is_empty(), |d| {
                    d.child(div().p_2().text_sm().text_color(muted).child(if rendering { "Rendering..." } else { "No preview yet." }))
                }),
        );

        div()
            .v_flex()
            .size_full()
            .gap_2()
            .child(div().h_flex().flex_1().min_h_0().gap_3().child(editor_pane).child(preview_pane))
            .when_some(self.problem.clone(), |d, problem| {
                // A missing typst gets its install hint and a "Check again" button next to the error.
                let help = crate::problem::help("cv-problem", jobpilot_core::health::classify(&problem), &self.state, None, cx);
                d.child(
                    div()
                        .v_flex()
                        .gap_2()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.danger)
                        .child(
                            div()
                                .id("cv-problem")
                                .max_h(px(110.))
                                .overflow_y_scroll()
                                .font_family("monospace")
                                .text_xs()
                                .text_color(theme.danger)
                                .child(problem),
                        )
                        .children(help),
                )
            })
            .when(!self.warnings.is_empty(), |d| {
                d.child(
                    div()
                        .id("cv-warnings")
                        .v_flex()
                        .max_h(px(80.))
                        .overflow_y_scroll()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.warning)
                        .text_xs()
                        .text_color(theme.warning)
                        .child(div().font_semibold().child("Not backed by profile.yaml (kept, check before approving):"))
                        .children(self.warnings.iter().map(|w| div().font_family("monospace").child(w.clone()))),
                )
            })
            .when(!gaps.is_empty(), |d| {
                d.child(
                    div()
                        .v_flex()
                        .gap_1()
                        .p_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .h_flex()
                                .items_center()
                                .justify_between()
                                .child(div().text_xs().font_semibold().child("Gaps: the JD wants this, nothing in profile.yaml shows it"))
                                .child(
                                    Button::new("open-profile")
                                        .ghost()
                                        .compact()
                                        .icon(IconName::ExternalLink)
                                        .label("Open profile.yaml")
                                        .on_click(cx.listener(|this, _, _, cx| this.open_profile(cx))),
                                ),
                        )
                        .child(
                            div()
                                .id("cv-gaps")
                                .v_flex()
                                .max_h(px(48.))
                                .overflow_y_scroll()
                                .text_xs()
                                .text_color(muted)
                                .children(gaps.iter().map(|g| div().child(format!("- {g}")))),
                        )
                        .when_some(self.open_error.clone(), |d, e| d.child(div().text_xs().text_color(theme.danger).child(e))),
                )
            })
            .child(Input::new(&self.ask_ai).disabled(busy || !self.has_cv))
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::store::NewApplication;
    use jobpilot_core::{runtime, Core, Paths, Secrets};

    use super::save_and_render;

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
}
