//! The files of an application folder as this context reads them: `cv.yaml`, the `page-<n>.png`
//! previews, and what "Generate" re-runs. Blocking file work: callers run it through
//! `shared::run` / `shared::run_sync`.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use jobpilot_core::pipeline::{self, JdInput, Violation};
use jobpilot_core::store::Application;
use jobpilot_core::Core;

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

fn has_http_scheme(s: &str) -> bool {
    let lower = s.get(..8).unwrap_or(s).to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
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

/// What the CV tab shows for an application, read from its folder.
#[derive(Debug, Default)]
pub struct CvFiles {
    /// `cv.yaml`, if it exists.
    pub yaml: Option<String>,
    /// `page-1.png`, `page-2.png`, ... in page order.
    pub pages: Vec<PathBuf>,
    /// `cv.yaml` was saved after the previews were made (or there are none): they need a render.
    pub stale: bool,
}

pub fn load_cv_files(app: &Application) -> CvFiles {
    let pages = page_files(&app.dir);
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let yaml_time = modified(&app.cv_yaml_path());
    let oldest_page = pages.iter().filter_map(|p| modified(p)).min();
    CvFiles {
        yaml: std::fs::read_to_string(app.cv_yaml_path()).ok(),
        stale: match (yaml_time, oldest_page) {
            (Some(yaml), Some(page)) => yaml > page,
            (Some(_), None) => true,
            _ => false,
        },
        pages,
    }
}

/// `page-<n>.png` files of a folder in page order (`page-2` before `page-10`).
pub fn page_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut pages: Vec<(u32, PathBuf)> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let n = name
                .strip_prefix("page-")?
                .strip_suffix(".png")?
                .parse::<u32>()
                .ok()?;
            Some((n, e.path()))
        })
        .collect();
    pages.sort();
    pages.into_iter().map(|(_, p)| p).collect()
}

/// The bytes of preview page `index` (0-based).
pub fn read_page(app: &Application, index: usize) -> Result<Vec<u8>> {
    let pages = page_files(&app.dir);
    let path = pages
        .get(index)
        .with_context(|| format!("there is no preview page {}", index + 1))?;
    std::fs::read(path).with_context(|| format!("reading {}", path.display()))
}

/// Result of saving and re-rendering the CV.
#[derive(Debug)]
pub struct Rendered {
    pub page_count: usize,
    /// Fact-guard findings, as lines (the edit is kept, the user decides).
    pub warnings: Vec<String>,
}

fn violation_lines(violations: &[Violation]) -> Vec<String> {
    violations.iter().map(|v| v.to_string()).collect()
}

/// Write `cv.yaml`, then `pipeline::rerender` (parse, fact guard, Typst) and count the new previews.
pub async fn save_and_render(core: &Core, id: i64, yaml: String) -> Result<Rendered> {
    let app = core
        .store
        .get_application(id)?
        .context("the application no longer exists")?;
    tokio::fs::create_dir_all(&app.dir)
        .await
        .with_context(|| format!("creating {}", app.dir.display()))?;
    tokio::fs::write(app.cv_yaml_path(), yaml)
        .await
        .context("saving cv.yaml")?;
    let outcome = pipeline::rerender(core, id).await?;
    Ok(Rendered {
        page_count: page_files(&app.dir).len(),
        warnings: violation_lines(&outcome.violations),
    })
}

/// Write the editor text to `cv.yaml` (the revision starts from the file on disk).
pub async fn write_cv(core: &Core, id: i64, yaml: String) -> Result<()> {
    let app = core
        .store
        .get_application(id)?
        .context("the application no longer exists")?;
    tokio::fs::write(app.cv_yaml_path(), yaml)
        .await
        .context("saving cv.yaml")
}

/// Does the stored CV pass the fact guard against the profile? `false` when it cannot be told (no
/// CV, an unreadable or invalid profile, a CV that does not parse).
pub fn guard_clean(core: &Core, yaml: Option<&str>) -> bool {
    yaml.is_some_and(|yaml| core.cv_is_traced(yaml))
}

/// `xdg-open <path>` (`open` on macOS): hand the file to the user's default editor.
pub fn open_path(path: &Path) -> Result<()> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("could not run {opener} on {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use jobpilot_core::store::NewApplication;
    use jobpilot_core::{runtime, Paths, Secrets};

    use super::*;

    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");

    fn fresh_core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let core =
            Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).expect("a core");
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
            .expect("an application")
    }

    #[test]
    fn generate_reruns_from_the_best_available_input() {
        let (_dir, core) = fresh_core();
        let app = app(&core, Some("https://acme.test/jobs/1"));

        // Nothing stored: the source link.
        assert!(
            matches!(generate_plan(&app, None), GeneratePlan::Reingest(JdInput::Url(u)) if u == "https://acme.test/jobs/1")
        );
        // The input used in this session wins over the source.
        let cached = JdInput::Text("pasted".into());
        assert!(
            matches!(generate_plan(&app, Some(&cached)), GeneratePlan::Reingest(JdInput::Text(t)) if t == "pasted")
        );
        // A stored JD means no fetching at all.
        std::fs::create_dir_all(&app.dir).expect("the application folder");
        std::fs::write(app.jd_path(), "# Acme\n\nbody").expect("a JD");
        assert!(matches!(
            generate_plan(&app, Some(&cached)),
            GeneratePlan::Regenerate
        ));

        let (_dir2, core2) = fresh_core();
        let pasted = self::app(&core2, None);
        assert!(matches!(
            generate_plan(&pasted, None),
            GeneratePlan::Unavailable(_)
        ));
    }

    #[test]
    fn cv_files_load_in_page_order() {
        let (_dir, core) = fresh_core();
        let app = app(&core, None);
        assert!(load_cv_files(&app).yaml.is_none());

        std::fs::create_dir_all(&app.dir).expect("the application folder");
        std::fs::write(app.cv_yaml_path(), "basics: {}").expect("a CV");
        for (name, body) in [
            ("page-10.png", "ten"),
            ("page-2.png", "two"),
            ("page-1.png", "one"),
            ("notes.png", "x"),
        ] {
            std::fs::write(app.dir.join(name), body).expect("a page");
        }
        let files = load_cv_files(&app);
        assert_eq!(files.yaml.as_deref(), Some("basics: {}"));
        let names: Vec<String> = files
            .pages
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        assert_eq!(names, ["page-1.png", "page-2.png", "page-10.png"]);
        assert_eq!(read_page(&app, 1).expect("page 2"), b"two".to_vec());
        assert!(read_page(&app, 3).is_err(), "there is no fourth page");

        // Staleness: previews older than the YAML (an edit that never got rendered) or missing.
        let set_modified = |when: std::time::SystemTime| {
            for page in page_files(&app.dir) {
                std::fs::File::options()
                    .write(true)
                    .open(page)
                    .expect("a page")
                    .set_modified(when)
                    .expect("a time");
            }
        };
        set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600));
        assert!(load_cv_files(&app).stale);
        set_modified(std::time::SystemTime::now() + std::time::Duration::from_secs(3600));
        assert!(!load_cv_files(&app).stale);
        for page in page_files(&app.dir) {
            std::fs::remove_file(page).expect("a page to remove");
        }
        assert!(load_cv_files(&app).stale, "no previews at all");
    }

    #[test]
    fn saving_renders_the_edit_and_reports_problems() {
        let (_dir, core) = fresh_core();
        core.set_profile_path(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profile.yaml"))
            .expect("the profile path");
        let id = app(&core, None).id;

        // A valid CV: written, rendered, previews counted.
        let ok =
            runtime::block_on(save_and_render(&core, id, EXAMPLE.to_string())).expect("a render");
        assert!(ok.page_count > 0);
        let app = core
            .store
            .get_application(id)
            .expect("a read")
            .expect("the application");
        assert_eq!(
            std::fs::read_to_string(app.cv_yaml_path()).expect("cv.yaml"),
            EXAMPLE
        );
        assert!(read_page(&app, 0).expect("page 1").starts_with(b"\x89PNG"));
        assert_eq!(
            guard_clean(&core, Some(EXAMPLE)),
            ok.warnings.is_empty(),
            "both ask the fact guard about the same CV"
        );
        assert!(!guard_clean(&core, None), "no CV, nothing traced");

        // An invented employer is kept and rendered, but flagged (amber).
        let invented = EXAMPLE.replacen("company: ", "company: Made Up Corp #", 1);
        let flagged = runtime::block_on(save_and_render(&core, id, invented)).expect("a render");
        assert!(
            !flagged.warnings.is_empty(),
            "guard findings are warnings: {:?}",
            flagged.warnings
        );

        // Broken YAML is an error (red) and the edit is still saved.
        let err = runtime::block_on(save_and_render(&core, id, "basics: [".into()))
            .expect_err("a broken CV");
        assert!(!format!("{err:#}").is_empty());
        assert_eq!(
            std::fs::read_to_string(app.cv_yaml_path()).expect("cv.yaml"),
            "basics: ["
        );
    }
}
