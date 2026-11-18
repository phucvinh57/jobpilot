//! Typst rendering: `cv.yaml` -> `cv.pdf` + `page-N.png` previews, by shelling out to `typst`.
//!
//! The renderer template is embedded and copied next to the YAML so `--root` stays inside the
//! output directory (the template reads `/<yaml name>` relative to the root).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use regex::Regex;
use tokio::process::Command;

/// The Typst renderer (`assets/cv.typ`).
pub const CV_TEMPLATE: &str = include_str!("../../../../assets/cv.typ");
/// Env var overriding the `typst` binary.
pub const TYPST_ENV: &str = "JOBPILOT_TYPST";
#[cfg(windows)]
const TYPST_EXE: &str = "typst.exe";
#[cfg(not(windows))]
const TYPST_EXE: &str = "typst";
/// PNG preview resolution.
const PREVIEW_PPI: &str = "110";
/// The first compile downloads the `@preview` package, so allow for a slow network.
const TYPST_TIMEOUT: Duration = Duration::from_secs(180);
/// Longest typst error text shown to the user.
const MAX_ERROR_CHARS: usize = 4000;

/// Renders share one output folder per CV; run them one at a time so previews are never half-deleted.
static RENDER_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Typst rejected the CV (bad Typst markup in a text field, missing key, ...). Carries typst's
/// own diagnostics; downcast to tell it from "typst is not installed".
#[derive(Clone, Debug)]
pub struct TypstError(pub String);

impl std::fmt::Display for TypstError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "typst failed:\n{}", self.0)
    }
}

impl std::error::Error for TypstError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderResult {
    pub pdf: PathBuf,
    /// `page-1.png`, `page-2.png`, ... in page order.
    pub pages: Vec<PathBuf>,
    pub page_count: usize,
}

/// Look for Typst in `JOBPILOT_TYPST`, on `PATH`, in Cargo's bin directory, then in the usual
/// platform install locations. Apps launched from the desktop can have a different `PATH`.
pub fn typst_binary() -> PathBuf {
    if let Some(path) = std::env::var_os(TYPST_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(path);
    }
    let path = std::env::var_os("PATH");
    let on_path = path.as_deref().map(std::env::split_paths).into_iter().flatten().map(|dir| dir.join(TYPST_EXE));
    let cargo = std::env::var_os("CARGO_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .map(|c| c.join("bin").join(TYPST_EXE));
    #[cfg(windows)]
    let installed = dirs::data_local_dir().map(|dir| dir.join("Microsoft/WinGet/Links").join(TYPST_EXE));
    #[cfg(not(windows))]
    let installed = dirs::home_dir()
        .map(|h| h.join(".local/bin/typst"))
        .into_iter()
        // `/opt/homebrew/bin`: Homebrew on Apple Silicon, not on the PATH of an app opened from Finder.
        .chain(["/usr/local/bin/typst", "/usr/bin/typst", "/snap/bin/typst", "/opt/homebrew/bin/typst"].map(PathBuf::from));
    on_path.chain(cargo).chain(installed).find(|p| p.is_file()).unwrap_or_else(|| TYPST_EXE.into())
}

pub(crate) fn typst_command(binary: &Path) -> Command {
    let mut cmd = Command::new(binary);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    cmd
}

/// Render `yaml_path` into `out_dir`: `<stem>.pdf` and `page-<n>.png`. The YAML is copied into
/// `out_dir` if it is not already there (for an application it is `cv.yaml` -> `cv.pdf`).
/// Typst's own error text (file/line, message) is the error on failure.
pub async fn render(yaml_path: &Path, out_dir: &Path) -> Result<RenderResult> {
    let _guard = RENDER_LOCK.lock().await;
    tokio::fs::create_dir_all(out_dir).await.with_context(|| format!("creating {}", out_dir.display()))?;
    let out_dir = out_dir.canonicalize().with_context(|| format!("resolving {}", out_dir.display()))?;

    let yaml_name = yaml_path.file_name().context("the CV path has no file name")?.to_string_lossy().into_owned();
    let local_yaml = out_dir.join(&yaml_name);
    let same_file = yaml_path.canonicalize().map(|p| p == local_yaml).unwrap_or(false);
    if !same_file {
        tokio::fs::copy(yaml_path, &local_yaml)
            .await
            .with_context(|| format!("copying {} to {}", yaml_path.display(), out_dir.display()))?;
    }
    let template = out_dir.join("cv.typ");
    if tokio::fs::read_to_string(&template).await.ok().as_deref() != Some(CV_TEMPLATE) {
        tokio::fs::write(&template, CV_TEMPLATE).await.context("writing cv.typ")?;
    }
    let stem = local_yaml.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "cv".into());
    let pdf = out_dir.join(format!("{stem}.pdf"));
    remove_old_pages(&out_dir).await?;

    // One after the other: the first compile downloads the `@preview` package into the shared cache.
    let bin = typst_binary();
    typst_compile(&bin, &out_dir, &template, &yaml_name, &[], &pdf).await?;
    let png_pattern = out_dir.join("page-{p}.png");
    typst_compile(&bin, &out_dir, &template, &yaml_name, &["--format", "png", "--ppi", PREVIEW_PPI], &png_pattern).await?;

    let pages = list_pages(&out_dir).await?;
    if pages.is_empty() {
        bail!("typst produced no pages");
    }
    Ok(RenderResult { pdf, page_count: pages.len(), pages })
}

/// Compile `template` (inside `root`) into the PDF `out_pdf`, with `--input data=<data_file_name>`
/// (a file in `root` the template reads). Errors as for the CV: "typst was not found" (the
/// no-typst Problem) or a [`TypstError`] with typst's diagnostics.
pub(crate) async fn compile(root: &Path, template: &Path, data_file_name: &str, out_pdf: &Path) -> Result<()> {
    typst_compile(&typst_binary(), root, template, data_file_name, &[], out_pdf).await
}

async fn typst_compile(bin: &Path, root: &Path, template: &Path, data_name: &str, extra: &[&str], output: &Path) -> Result<()> {
    let mut cmd = typst_command(bin);
    cmd.arg("compile")
        .arg("--root")
        .arg(root)
        .arg("--input")
        .arg(format!("data={data_name}"))
        .args(extra)
        .arg(template)
        .arg(output)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            anyhow::anyhow!(
                "typst was not found ({}): install it with `{}` (or from https://typst.app), or set {TYPST_ENV} to its path",
                bin.display(),
                crate::health::TYPST_INSTALL_HINT
            )
        } else {
            anyhow::Error::new(e).context(format!("starting {}", bin.display()))
        }
    })?;
    let out = tokio::time::timeout(TYPST_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| anyhow::anyhow!("typst did not finish within {} seconds", TYPST_TIMEOUT.as_secs()))?
        .context("running typst")?;
    if !out.status.success() {
        return Err(TypstError(readable_stderr(&String::from_utf8_lossy(&out.stderr))).into());
    }
    Ok(())
}

/// typst's diagnostics without escape codes, bounded in size.
fn readable_stderr(stderr: &str) -> String {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    let ansi = ANSI.get_or_init(|| Regex::new("\u{1b}\\[[0-9;]*m").expect("static regex"));
    let text = ansi.replace_all(stderr, "");
    let text = text.trim();
    if text.chars().count() > MAX_ERROR_CHARS {
        let cut: String = text.chars().take(MAX_ERROR_CHARS).collect();
        format!("{cut}\n...")
    } else {
        text.to_string()
    }
}

fn page_number(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    name.strip_prefix("page-")?.strip_suffix(".png")?.parse().ok()
}

async fn list_pages(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut pages = Vec::new();
    let mut entries = tokio::fs::read_dir(dir).await.with_context(|| format!("reading {}", dir.display()))?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if page_number(&path).is_some() {
            pages.push(path);
        }
    }
    pages.sort_by_key(|p| page_number(p));
    Ok(pages)
}

async fn remove_old_pages(dir: &Path) -> Result<()> {
    for page in list_pages(dir).await? {
        tokio::fs::remove_file(&page).await.with_context(|| format!("removing {}", page.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = include_str!("../../../../assets/prompts/example-cv.yaml");

    #[test]
    fn page_files_sort_numerically() {
        assert_eq!(page_number(Path::new("/x/page-10.png")), Some(10));
        assert_eq!(page_number(Path::new("/x/page-1.png")), Some(1));
        assert_eq!(page_number(Path::new("/x/cv.png")), None);
        assert_eq!(page_number(Path::new("/x/page-1.pdf")), None);
    }

    #[test]
    fn stderr_is_cleaned() {
        let s = readable_stderr("\u{1b}[1;31merror\u{1b}[0m: boom\n");
        assert_eq!(s, "error: boom");
        assert!(readable_stderr(&"x".repeat(MAX_ERROR_CHARS * 2)).ends_with("..."));
    }

    #[tokio::test]
    async fn renders_pdf_and_pngs_and_replaces_old_pages() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Hồ sơ CV");
        std::fs::create_dir_all(&app).unwrap();
        let yaml = app.join("cv.yaml");
        std::fs::write(&yaml, EXAMPLE).unwrap();
        std::fs::write(app.join("page-9.png"), "stale").unwrap();

        let r = render(&yaml, &app).await.unwrap();
        assert_eq!(r.pdf, app.canonicalize().unwrap().join("cv.pdf"));
        assert!(std::fs::metadata(&r.pdf).unwrap().len() > 1000);
        assert!(r.page_count >= 1 && r.pages.len() == r.page_count);
        assert!(r.pages.iter().all(|p| p.is_file()));
        assert!(!app.join("page-9.png").exists(), "stale previews are removed");
        assert!(app.join("cv.typ").is_file(), "template copied next to the YAML");

        // A YAML elsewhere is copied into out_dir.
        let elsewhere = dir.path().join("CV thứ hai.yaml");
        std::fs::write(&elsewhere, EXAMPLE).unwrap();
        let out = dir.path().join("Bản xem trước");
        let r = render(&elsewhere, &out).await.unwrap();
        assert!(out.join("CV thứ hai.yaml").is_file() && r.pdf.ends_with("CV thứ hai.pdf"));
    }

    #[tokio::test]
    async fn profile_links_get_one_scheme_and_handles_their_site() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("cv.yaml");
        let cv = EXAMPLE
            .replace("github: github.com/phucvinh57", "github: \"@phucvinh57\"")
            .replace("linkedin: linkedin.com/in/npvinh57", "linkedin: HTTPS://www.linkedin.com/in/npvinh57/\n  website: https://vinh.dev/");
        std::fs::write(&yaml, cv).unwrap();
        let r = render(&yaml, dir.path()).await.unwrap();
        let links = crate::pipeline::ingest::pdf_links(&std::fs::read(&r.pdf).unwrap());
        for want in ["https://github.com/phucvinh57", "https://linkedin.com/in/npvinh57", "https://vinh.dev"] {
            assert!(links.iter().any(|l| l == want), "{want} not in {links:?}");
        }
        assert!(!links.iter().any(|l| l.matches("://").count() > 1), "{links:?}");
    }

    #[tokio::test]
    async fn typst_errors_are_readable() {
        let dir = tempfile::tempdir().unwrap();
        let yaml = dir.path().join("cv.yaml");
        std::fs::write(&yaml, "basics: {name: X}\nsummary: [\"a #bad thing\"]\n").unwrap();
        let err = render(&yaml, dir.path()).await.unwrap_err().to_string();
        assert!(err.starts_with("typst failed:") && err.contains("unknown variable: bad"), "{err}");
    }

    #[tokio::test]
    async fn missing_typst_is_explained() {
        let dir = tempfile::tempdir().unwrap();
        let err = typst_compile(
            Path::new("/nonexistent/typst-binary"),
            dir.path(),
            &dir.path().join("cv.typ"),
            "cv.yaml",
            &[],
            &dir.path().join("o.pdf"),
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("typst was not found") && err.contains(TYPST_ENV), "{err}");
    }
}
