//! The documents the profile is read from: uploaded files and typed notes.
//!
//! Files: `.pdf`, `.md`, `.txt` (text extracted like a JD file, see `pipeline::ingest`), at most
//! [`MAX_BYTES`] on disk and [`MAX_CHARS`] of text each (the rest is cut). Notes: at most
//! [`MAX_CHARS`]. All documents together hold at most [`MAX_TOTAL_CHARS`], so one read fits a
//! model's context. The text is personal: it stays in the local database and is never logged.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};

use crate::engine::pipeline::analyze::truncate_chars;
use crate::engine::pipeline::ingest::{ingest, JdInput};
use crate::engine::store::{NewProfileSource, ProfileSource};
use crate::engine::Core;

/// File types accepted.
pub const EXTENSIONS: [&str; 3] = ["pdf", "md", "txt"];
/// Largest file accepted.
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
/// Longest text kept per document.
pub const MAX_CHARS: usize = 20_000;
/// Longest text of all documents together.
pub const MAX_TOTAL_CHARS: usize = 60_000;
/// Longest note title kept.
const MAX_NAME_CHARS: usize = 80;

/// Whether `path` has an accepted extension.
pub fn is_supported(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
    EXTENSIONS.contains(&ext.as_str())
}

impl Core {
    /// The user's documents, oldest first.
    pub fn profile_sources(&self) -> Result<Vec<ProfileSource>> {
        self.store.profile_sources()
    }

    /// Read `path`, extract its text and keep it as a document. Does not read the profile again
    /// (see [`Core::read_profile_sources`]).
    pub async fn add_profile_file(&self, path: &Path) -> Result<ProfileSource> {
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
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "document".into());
        let doc = ingest(JdInput::File(path.to_path_buf())).await.context("reading the document")?;
        ensure!(!doc.text.trim().is_empty(), "no text found in {name}: if it is a scanned PDF, paste its text instead");
        let kept = truncate_chars(&doc.text, MAX_CHARS);
        let truncated = kept.len() < doc.text.len();
        let source = self.store.add_profile_source(&NewProfileSource::file(&name, kept, truncated), MAX_TOTAL_CHARS)?;
        tracing::info!(id = source.id, chars = source.chars, truncated, "profile: document added");
        Ok(source)
    }

    /// Keep a note the user typed as a document.
    pub fn add_profile_text(&self, name: &str, text: &str) -> Result<ProfileSource> {
        let text = text.trim();
        ensure!(!text.is_empty(), "write something about your work first");
        let chars = text.chars().count();
        ensure!(chars <= MAX_CHARS, "keep a note under {MAX_CHARS} characters (this one has {chars})");
        let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
        let name = if name.is_empty() { "Note".to_string() } else { truncate_chars(&name, MAX_NAME_CHARS).to_string() };
        let source = self.store.add_profile_source(&NewProfileSource::text(&name, text), MAX_TOTAL_CHARS)?;
        tracing::info!(id = source.id, chars = source.chars, "profile: note added");
        Ok(source)
    }

    /// Forget a document. With none left, the profile is forgotten too.
    pub fn remove_profile_source(&self, id: i64) -> Result<()> {
        ensure!(self.store.remove_profile_source(id)?, "that document is already gone");
        if self.store.profile_sources()?.is_empty() {
            self.store.clear_profile()?;
        }
        tracing::info!(id, "profile: document removed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core =
            Core::open_at(crate::engine::Paths::new(dir.path().join("jp")), crate::engine::Secrets::memory()).unwrap();
        (dir, core)
    }

    #[tokio::test]
    async fn files_are_checked_read_and_kept() {
        let (dir, core) = core();
        let bad = dir.path().join("cv.docx");
        std::fs::write(&bad, "x").unwrap();
        assert!(core.add_profile_file(&bad).await.unwrap_err().to_string().contains("unsupported file type"));

        let empty = dir.path().join("empty.txt");
        std::fs::write(&empty, "  \n").unwrap();
        assert!(core.add_profile_file(&empty).await.is_err());

        let long = dir.path().join("cv.md");
        std::fs::write(&long, "Rust developer. ".repeat(2_000)).unwrap();
        let source = core.add_profile_file(&long).await.unwrap();
        assert_eq!((source.name.as_str(), source.kind.as_str(), source.chars), ("cv.md", "file", MAX_CHARS));
        assert!(source.truncated);
    }

    #[test]
    fn notes_are_trimmed_named_and_capped() {
        let (_dir, core) = core();
        assert!(core.add_profile_text("", "   ").is_err());
        assert!(core.add_profile_text("", &"x".repeat(MAX_CHARS + 1)).is_err());
        let note = core.add_profile_text("  ", "  I also ran the on-call rota.  ").unwrap();
        assert_eq!((note.name.as_str(), note.text.as_str(), note.kind.as_str()), ("Note", "I also ran the on-call rota.", "text"));
        for _ in 0..2 {
            core.add_profile_text("Big", &"y".repeat(MAX_CHARS)).unwrap();
        }
        let err = core.add_profile_text("Big", &"y".repeat(MAX_CHARS)).unwrap_err();
        assert!(err.to_string().contains("more than"), "{err}");
    }

    #[test]
    fn removing_the_last_document_forgets_the_profile() {
        let (_dir, core) = core();
        let a = core.add_profile_text("A", "basics: {name: Ann}").unwrap();
        let b = core.add_profile_text("B", "Rust").unwrap();
        core.save_profile_cv(&crate::engine::pipeline::cv::Cv::parse("basics: {name: Ann}").unwrap(), &[]).unwrap();
        core.remove_profile_source(b.id).unwrap();
        assert!(core.profile().unwrap().is_some());
        core.remove_profile_source(a.id).unwrap();
        assert_eq!(core.profile().unwrap(), None);
        assert!(core.remove_profile_source(a.id).is_err());
    }
}
