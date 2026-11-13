//! Pure logic of the generate/edit UI (no GPUI, easy to test): what the sidebar box holds, progress
//! lines, what "Generate" re-runs, the CV file loader, and the edit debounce / render queue.

use std::path::{Path, PathBuf};
use std::time::Duration;

use jobpilot_core::pipeline::{JdInput, Progress};
use jobpilot_core::store::Application;

/// File types accepted by drag and drop.
pub const JD_EXTENSIONS: [&str; 5] = ["md", "txt", "html", "htm", "pdf"];

/// Quiet time after the last keystroke before the CV is saved and re-rendered.
pub const EDIT_DEBOUNCE: Duration = Duration::from_millis(800);

// ---------------------------------------------------------------- sidebar input

/// What the user typed or pasted into the sidebar box: a link, a path to a JD file, or the JD text.
/// `None` for blank input.
pub fn parse_input(raw: &str) -> Option<JdInput> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let single_token = !raw.contains(char::is_whitespace);
    if single_token && has_http_scheme(raw) {
        return Some(JdInput::Url(raw.to_string()));
    }
    if let Some(path) = as_file_path(raw) {
        return Some(JdInput::File(path));
    }
    if single_token && looks_like_bare_url(raw) {
        return Some(JdInput::Url(format!("https://{raw}")));
    }
    Some(JdInput::Text(raw.to_string()))
}

fn has_http_scheme(s: &str) -> bool {
    let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

/// `jobs.example.com/careers/42`: a host with a dot, then a path.
fn looks_like_bare_url(s: &str) -> bool {
    let Some((host, _)) = s.split_once('/') else { return false };
    host.contains('.') && host.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// A pasted path (`/home/me/jd.pdf`, `~`-less, optionally quoted or a `file://` URI) that is a file.
fn as_file_path(raw: &str) -> Option<PathBuf> {
    let unquoted = raw.trim_matches(|c| c == '"' || c == '\'');
    let path = match unquoted.strip_prefix("file://") {
        Some(rest) => PathBuf::from(percent_decode(rest)),
        None => PathBuf::from(unquoted),
    };
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(v) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The first dropped file that is a supported JD file; an error line when there is none.
pub fn file_input(paths: &[PathBuf]) -> Result<JdInput, String> {
    if paths.is_empty() {
        return Err("Nothing was dropped.".into());
    }
    for path in paths {
        if path.is_file() && extension(path).is_some_and(|e| JD_EXTENSIONS.contains(&e.as_str())) {
            return Ok(JdInput::File(path.clone()));
        }
    }
    let first = &paths[0];
    let name = first.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Err(match extension(first) {
        _ if first.is_dir() => format!("{name} is a folder: drop a .md, .txt, .html or .pdf file."),
        Some(ext) => format!("{name}: .{ext} files are not supported (use .md, .txt, .html or .pdf)."),
        None => format!("{name}: unknown file type (use .md, .txt, .html or .pdf)."),
    })
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase())
}

// ---------------------------------------------------------------- progress

/// One line for a row and the header: `Tailoring the CV` or `Tailoring the CV: repairing 2 problem(s)`.
pub fn progress_label(p: &Progress) -> String {
    if p.detail.is_empty() {
        p.stage.label().to_string()
    } else {
        format!("{}: {}", p.stage.label(), p.detail)
    }
}

// ---------------------------------------------------------------- what "Generate" re-runs

/// How to (re)produce a CV for a Draft that has none.
#[derive(Clone, Debug)]
pub enum GeneratePlan {
    /// The JD is stored: new CV from it, no fetching.
    Regenerate,
    /// The JD never made it to disk: run the whole pipeline again on this input.
    Reingest(JdInput),
    /// Nothing to start from.
    Unavailable(String),
}

/// `cached` is the input this session used to create the Draft, if it did.
pub fn generate_plan(app: &Application, cached: Option<&JdInput>) -> GeneratePlan {
    if app.jd_path().is_file() {
        return GeneratePlan::Regenerate;
    }
    if let Some(input) = cached {
        return GeneratePlan::Reingest(input.clone());
    }
    match app.source.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(src) if has_http_scheme(src) => GeneratePlan::Reingest(JdInput::Url(src.to_string())),
        Some(src) if Path::new(src).is_file() => GeneratePlan::Reingest(JdInput::File(PathBuf::from(src))),
        _ => GeneratePlan::Unavailable(
            "The job description was not saved and its source is gone: paste the link or text again.".into(),
        ),
    }
}

// ---------------------------------------------------------------- CV files

/// What the CV tab shows for an application, read from its folder.
#[derive(Debug, Default)]
pub struct CvFiles {
    /// `cv.yaml`, if it exists.
    pub yaml: Option<String>,
    /// PNG bytes of `page-1.png`, `page-2.png`, ... in page order.
    pub pages: Vec<Vec<u8>>,
    /// `cv.yaml` was saved after the previews were made (or there are none): they need a render.
    pub stale: bool,
}

/// Blocking: run it with `runtime::spawn`.
pub fn load_cv_files(app: &Application) -> CvFiles {
    let page_paths = page_files(&app.dir);
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let yaml_time = modified(&app.cv_yaml_path());
    let oldest_page = page_paths.iter().filter_map(|p| modified(p)).min();
    CvFiles {
        yaml: std::fs::read_to_string(app.cv_yaml_path()).ok(),
        pages: page_paths.iter().filter_map(|p| std::fs::read(p).ok()).collect(),
        stale: match (yaml_time, oldest_page) {
            (Some(yaml), Some(page)) => yaml > page,
            (Some(_), None) => true,
            _ => false,
        },
    }
}

/// `page-<n>.png` files of a folder in page order (`page-2` before `page-10`).
pub fn page_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut pages: Vec<(u32, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name.strip_prefix("page-")?.strip_suffix(".png")?.parse::<u32>().ok()?;
            Some((n, e.path()))
        })
        .collect();
    pages.sort();
    pages.into_iter().map(|(_, p)| p).collect()
}

/// `gaps_json` as a list of "JD wants X, no evidence in the profile" lines (empty when absent or invalid).
pub fn parse_gaps(json: Option<&str>) -> Vec<String> {
    json.and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty())
        .collect()
}

// ---------------------------------------------------------------- debounce and render queue

/// Latest-wins debounce: `touch()` on every edit, then after the quiet time act only if
/// `is_latest(token)` still holds.
#[derive(Debug, Default)]
pub struct Debounce {
    generation: u64,
}

impl Debounce {
    pub fn touch(&mut self) -> u64 {
        self.generation += 1;
        self.generation
    }

    pub fn is_latest(&self, token: u64) -> bool {
        token == self.generation
    }

    /// Invalidate every pending token (e.g. the text was replaced from disk).
    pub fn cancel(&mut self) {
        self.generation += 1;
    }
}

/// One render at a time: edits that arrive during a render cause exactly one more render afterwards.
#[derive(Debug, Default)]
pub struct RenderQueue {
    running: bool,
    again: bool,
}

impl RenderQueue {
    /// A render is wanted. `true` = start it now; `false` = one is running and will be repeated.
    pub fn request(&mut self) -> bool {
        if self.running {
            self.again = true;
            false
        } else {
            self.running = true;
            true
        }
    }

    /// The running render ended. `true` = another one is due (start it now).
    pub fn finish(&mut self) -> bool {
        if std::mem::take(&mut self.again) {
            true
        } else {
            self.running = false;
            false
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }
}

#[cfg(test)]
mod tests {
    use jobpilot_core::pipeline::Stage;
    use jobpilot_core::store::NewApplication;
    use jobpilot_core::{Core, Paths, Secrets};

    use super::*;

    fn url(input: Option<JdInput>) -> String {
        match input {
            Some(JdInput::Url(u)) => u,
            other => panic!("not a URL: {other:?}"),
        }
    }

    #[test]
    fn links_files_and_text_are_told_apart() {
        assert_eq!(url(parse_input("  https://acme.test/jobs/1 ")), "https://acme.test/jobs/1");
        assert_eq!(url(parse_input("HTTP://ACME.test/x")), "HTTP://ACME.test/x");
        assert_eq!(url(parse_input("jobs.acme.test/careers/42")), "https://jobs.acme.test/careers/42");
        assert!(parse_input("   ").is_none());

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("my jd.md");
        std::fs::write(&file, "# JD").unwrap();
        let quoted = format!("'{}'", file.display());
        assert!(matches!(parse_input(&quoted), Some(JdInput::File(p)) if p == file));
        let uri = format!("file://{}", file.display().to_string().replace(' ', "%20"));
        assert!(matches!(parse_input(&uri), Some(JdInput::File(p)) if p == file));
        // A path that does not exist is just text.
        assert!(matches!(parse_input("/no/such/jd.md"), Some(JdInput::Text(_))));

        // A sentence that mentions a link is a JD, not a link.
        let text = "We are hiring a Rust engineer. Apply at https://acme.test/apply";
        assert!(matches!(parse_input(text), Some(JdInput::Text(t)) if t == text));
        assert!(matches!(parse_input("Senior.Engineer"), Some(JdInput::Text(_))));
    }

    #[test]
    fn dropped_files_need_a_supported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, "x").unwrap();
            p
        };
        let (md, pdf, png) = (make("a.MD"), make("b.pdf"), make("c.png"));
        assert!(matches!(file_input(&[md.clone()]), Ok(JdInput::File(p)) if p == md));
        assert!(matches!(file_input(&[png.clone(), pdf.clone()]), Ok(JdInput::File(p)) if p == pdf));
        assert!(file_input(&[png]).unwrap_err().contains(".png"));
        assert!(file_input(&[dir.path().to_path_buf()]).unwrap_err().contains("folder"));
        assert!(file_input(&[]).is_err());
        assert!(file_input(&[dir.path().join("missing.md")]).is_err());
    }

    #[test]
    fn progress_lines_carry_the_detail() {
        let p = Progress { app_id: 1, stage: Stage::Tailoring, detail: String::new() };
        assert_eq!(progress_label(&p), "Tailoring the CV");
        let p = Progress { detail: "repairing 2 problem(s)".into(), ..p };
        assert_eq!(progress_label(&p), "Tailoring the CV: repairing 2 problem(s)");
    }

    fn fresh_core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn app(core: &Core, source: Option<&str>) -> Application {
        core.store
            .create_application(&NewApplication {
                slug: "acme".into(),
                company: "Acme".into(),
                role: "Engineer".into(),
                source: source.map(Into::into),
                dir: core.paths.application_dir("acme"),
                ..Default::default()
            })
            .unwrap()
    }

    #[test]
    fn generate_reruns_from_the_best_available_input() {
        let (_dir, core) = fresh_core();
        let app = app(&core, Some("https://acme.test/jobs/1"));

        // Nothing stored: the source link.
        assert!(matches!(generate_plan(&app, None), GeneratePlan::Reingest(JdInput::Url(u)) if u == "https://acme.test/jobs/1"));
        // The input used in this session wins over the source.
        let cached = JdInput::Text("pasted".into());
        assert!(matches!(generate_plan(&app, Some(&cached)), GeneratePlan::Reingest(JdInput::Text(t)) if t == "pasted"));
        // A stored JD means no fetching at all.
        std::fs::create_dir_all(&app.dir).unwrap();
        std::fs::write(app.jd_path(), "# Acme\n\nbody").unwrap();
        assert!(matches!(generate_plan(&app, Some(&cached)), GeneratePlan::Regenerate));

        let (_dir2, core2) = fresh_core();
        let pasted = self::app(&core2, None);
        assert!(matches!(generate_plan(&pasted, None), GeneratePlan::Unavailable(_)));
    }

    #[test]
    fn cv_files_load_in_page_order() {
        let (_dir, core) = fresh_core();
        let app = app(&core, None);
        assert!(load_cv_files(&app).yaml.is_none());

        std::fs::create_dir_all(&app.dir).unwrap();
        std::fs::write(app.cv_yaml_path(), "basics: {}").unwrap();
        for (name, body) in [("page-10.png", "ten"), ("page-2.png", "two"), ("page-1.png", "one"), ("notes.png", "x")] {
            std::fs::write(app.dir.join(name), body).unwrap();
        }
        let files = load_cv_files(&app);
        assert_eq!(files.yaml.as_deref(), Some("basics: {}"));
        assert_eq!(files.pages, vec![b"one".to_vec(), b"two".to_vec(), b"ten".to_vec()]);

        // Staleness: previews older than the YAML (an edit that never got rendered) or missing.
        let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        for page in page_files(&app.dir) {
            std::fs::File::options().write(true).open(page).unwrap().set_modified(long_ago).unwrap();
        }
        assert!(load_cv_files(&app).stale);
        let long_ago = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        for page in page_files(&app.dir) {
            std::fs::File::options().write(true).open(page).unwrap().set_modified(long_ago).unwrap();
        }
        assert!(!load_cv_files(&app).stale);
        for page in page_files(&app.dir) {
            std::fs::remove_file(page).unwrap();
        }
        assert!(load_cv_files(&app).stale, "no previews at all");
    }

    #[test]
    fn gaps_come_from_json_strings() {
        assert_eq!(parse_gaps(Some(r#"["Kubernetes at scale", " ", "Haskell"]"#)), ["Kubernetes at scale", "Haskell"]);
        assert!(parse_gaps(Some("not json")).is_empty());
        assert!(parse_gaps(None).is_empty());
    }

    #[test]
    fn only_the_last_edit_survives_the_debounce() {
        let mut d = Debounce::default();
        let first = d.touch();
        let second = d.touch();
        assert!(!d.is_latest(first));
        assert!(d.is_latest(second));
        d.cancel();
        assert!(!d.is_latest(second));
        assert!(d.is_latest(d.generation));
    }

    #[test]
    fn edits_during_a_render_cause_one_more_render() {
        let mut q = RenderQueue::default();
        assert!(q.request(), "idle: start");
        assert!(!q.request(), "busy: queued");
        assert!(!q.request(), "busy: still one");
        assert!(q.finish(), "one repeat");
        assert!(q.is_running());
        assert!(!q.finish(), "done");
        assert!(!q.is_running());
        assert!(q.request());
    }
}
