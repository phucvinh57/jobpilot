//! The CV the user attaches to the preference chat.
//!
//! Accepted: `.pdf`, `.md`, `.txt` (text extracted like a JD file, see `pipeline::ingest`), at most
//! [`MAX_BYTES`] on disk and [`MAX_CHARS`] of text. The text is kept in
//! `<data_dir>/prefs/uploaded-cv.txt` and is used as chat context only: it is untrusted, wrapped as
//! data in the prompt, and never becomes a fact of `profile.yaml` (importing it is a separate,
//! explicit step that shows a diff first).

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use crate::pipeline::analyze::truncate_chars;
use crate::pipeline::ingest::{ingest, JdInput};
use crate::Core;

/// File types the chat accepts.
pub const EXTENSIONS: [&str; 3] = ["pdf", "md", "txt"];
/// Largest file accepted.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Longest text kept; the rest is cut off.
pub const MAX_CHARS: usize = 20_000;
/// kv key holding the file name of the stored CV (the text lives in a file).
const NAME_KEY: &str = "prefs_cv_name";

/// A CV stored for the chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadedCv {
    /// File name as the user knows it (no directory).
    pub name: String,
    /// Characters of text kept.
    pub chars: usize,
    /// The text was longer than [`MAX_CHARS`] and was cut.
    pub truncated: bool,
}

fn text_path(core: &Core) -> PathBuf {
    core.paths.data_dir.join("prefs").join("uploaded-cv.txt")
}

/// Whether `path` has an accepted extension.
pub fn is_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
}

/// Read `path`, extract its text and store it as the chat's CV (replacing an earlier one).
pub async fn import(core: &Core, path: &Path) -> Result<UploadedCv> {
    ensure!(
        is_supported(path),
        "unsupported file type: use a {} file",
        EXTENSIONS.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", ")
    );
    let meta = tokio::fs::metadata(path).await.with_context(|| format!("reading {}", path.display()))?;
    ensure!(meta.is_file(), "{} is not a file", path.display());
    if meta.len() > MAX_BYTES {
        bail!("the file is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "cv".into());
    let jd = ingest(JdInput::File(path.to_path_buf())).await.context("reading the CV")?;
    let kept = truncate_chars(&jd.text, MAX_CHARS);
    let cv = UploadedCv { name, chars: kept.chars().count(), truncated: kept.len() < jd.text.len() };

    let file = text_path(core);
    let dir = file.parent().context("the CV path has no directory")?;
    tokio::fs::create_dir_all(dir).await.with_context(|| format!("creating {}", dir.display()))?;
    // Write beside the target, then rename: a crash never leaves a half-written CV.
    let tmp = file.with_extension("txt.tmp");
    // Private from the first byte (a leftover tmp file keeps its old mode, so `restrict` stays).
    write_private(&tmp, kept.as_bytes()).await.with_context(|| format!("writing {}", tmp.display()))?;
    restrict(&tmp)?;
    if let Err(e) = tokio::fs::rename(&tmp, &file).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e).context("saving the CV text");
    }
    core.store.kv_set(NAME_KEY, &cv.name)?;
    tracing::info!(chars = cv.chars, truncated = cv.truncated, "preference chat: CV stored");
    Ok(cv)
}

/// The stored CV and its text, if one was attached.
pub fn stored(core: &Core) -> Result<Option<(UploadedCv, String)>> {
    let Some(name) = core.store.kv_get(NAME_KEY)? else { return Ok(None) };
    let text = match std::fs::read_to_string(text_path(core)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context("reading the stored CV text"),
    };
    let cv = UploadedCv { name, chars: text.chars().count(), truncated: false };
    Ok(Some((cv, text)))
}

/// Forget the stored CV.
pub fn remove(core: &Core) -> Result<()> {
    match std::fs::remove_file(text_path(core)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context("removing the stored CV text"),
    }
    core.store.kv_delete(NAME_KEY)
}

async fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(path).await?;
    file.write_all(bytes).await?;
    file.flush().await
}

/// The CV is personal: readable by the user only.
fn restrict(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).context("restricting the CV file")?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{runtime, Paths, Secrets};

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn write(dir: &Path, name: &str, text: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    const CV: &str = "# Jane Doe\n\nBackend engineer. Five years of Rust and PostgreSQL at a payments company.\n";

    #[test]
    fn text_and_markdown_are_stored_and_read_back() {
        let (dir, core) = core();
        assert!(stored(&core).unwrap().is_none());
        for name in ["cv.txt", "CV.MD"] {
            let cv = runtime::block_on(import(&core, &write(dir.path(), name, CV))).unwrap();
            assert_eq!(cv.name, name);
            assert!(!cv.truncated);
            let (again, text) = stored(&core).unwrap().unwrap();
            assert_eq!(again.name, name);
            assert!(text.contains("payments company"));
        }
        let file = core.paths.data_dir.join("prefs").join("uploaded-cv.txt");
        assert!(file.is_file(), "stored under <data_dir>/prefs/uploaded-cv.txt");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        }
        remove(&core).unwrap();
        assert!(stored(&core).unwrap().is_none() && !file.exists());
    }

    #[test]
    fn only_pdf_md_and_txt_are_accepted() {
        let (dir, core) = core();
        for name in ["cv.docx", "cv.html", "cv", "cv.exe"] {
            let err = runtime::block_on(import(&core, &write(dir.path(), name, CV))).unwrap_err();
            assert!(err.to_string().contains("unsupported file type"), "{name}: {err}");
        }
        assert!(is_supported(Path::new("a/b/My CV.PDF")));
        assert!(stored(&core).unwrap().is_none(), "a refused file stores nothing");
    }

    #[test]
    fn big_files_are_refused_and_long_text_is_cut() {
        let (dir, core) = core();
        let big = dir.path().join("big.txt");
        let file = std::fs::File::create(&big).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        let err = runtime::block_on(import(&core, &big)).unwrap_err();
        assert!(err.to_string().contains("larger than 5 MB"), "{err}");

        let long = write(dir.path(), "long.txt", &"word ".repeat(MAX_CHARS));
        let cv = runtime::block_on(import(&core, &long)).unwrap();
        assert!(cv.truncated);
        assert!(cv.chars <= MAX_CHARS);
        assert!(stored(&core).unwrap().unwrap().1.chars().count() <= MAX_CHARS);
    }

    #[test]
    fn empty_and_missing_files_are_errors() {
        let (dir, core) = core();
        let empty = write(dir.path(), "empty.txt", "  \n");
        assert!(runtime::block_on(import(&core, &empty)).is_err());
        assert!(runtime::block_on(import(&core, &dir.path().join("nope.txt"))).is_err());
        assert!(runtime::block_on(import(&core, dir.path())).is_err(), "a directory is not a CV");
        assert!(stored(&core).unwrap().is_none());
    }

    #[test]
    fn an_unreadable_pdf_is_an_error_not_a_panic() {
        let (dir, core) = core();
        let bad = write(dir.path(), "cv.pdf", "this is not a pdf");
        assert!(runtime::block_on(import(&core, &bad)).is_err());
    }
}
