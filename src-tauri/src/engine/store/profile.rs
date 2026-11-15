use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// One document the user gave about their work: an uploaded file (its extracted text) or a note
/// they typed. Untrusted text: it reaches the model only between markers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileSource {
    pub id: i64,
    /// File name (no directory) or the note's title.
    pub name: String,
    /// `file` or `text`.
    pub kind: String,
    pub text: String,
    /// Characters kept.
    pub chars: usize,
    /// The file had more text than was kept.
    pub truncated: bool,
    pub created_at: DateTime<Utc>,
}

/// A document to store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewProfileSource {
    pub name: String,
    pub kind: String,
    pub text: String,
    pub truncated: bool,
}

impl NewProfileSource {
    /// A typed note.
    pub fn text(name: &str, text: &str) -> Self {
        Self { name: name.into(), kind: "text".into(), text: text.into(), truncated: false }
    }

    /// The text of an uploaded file.
    pub fn file(name: &str, text: &str, truncated: bool) -> Self {
        Self { name: name.into(), kind: "file".into(), text: text.into(), truncated }
    }
}

/// The profile last read from the documents (`profile.id = 1`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredProfile {
    /// The profile in the CV schema (`pipeline::cv::Cv`), as YAML.
    pub yaml: String,
    /// [`Store::profile_sources_hash`] of the documents it was read from.
    pub sources_hash: String,
    /// What the reader found but could not trace to a document, so left out.
    pub left_out: Vec<String>,
    pub built_at: DateTime<Utc>,
}

/// [`Store::profile_sources_hash`] of `sources` (all documents, oldest first).
pub fn sources_hash(sources: &[ProfileSource]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    for source in sources {
        hasher.update(source.id.to_le_bytes());
        hasher.update((source.text.len() as u64).to_le_bytes());
        hasher.update(source.text.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

const SOURCE_COLUMNS: &str = "id, name, kind, text, chars, truncated, created_at";

fn source_row(r: &Row<'_>) -> rusqlite::Result<ProfileSource> {
    let chars: i64 = r.get(4)?;
    Ok(ProfileSource {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        text: r.get(3)?,
        chars: usize::try_from(chars).unwrap_or_default(),
        truncated: r.get(5)?,
        created_at: r.get(6)?,
    })
}

impl Store {
    /// The user's documents, oldest first.
    pub fn profile_sources(&self) -> Result<Vec<ProfileSource>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {SOURCE_COLUMNS} FROM profile_sources ORDER BY id"))?;
            let rows = stmt.query_map([], source_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Store a document, unless all documents together would exceed `max_total_chars`.
    pub fn add_profile_source(&self, new: &NewProfileSource, max_total_chars: usize) -> Result<ProfileSource> {
        let chars = new.text.chars().count();
        self.with_tx(|tx| {
            let total: i64 = tx.query_row("SELECT COALESCE(SUM(chars), 0) FROM profile_sources", [], |r| r.get(0))?;
            let total = usize::try_from(total).unwrap_or_default();
            if total.saturating_add(chars) > max_total_chars {
                bail!(
                    "your documents would hold more than {max_total_chars} characters: remove one or add a shorter text"
                );
            }
            tx.execute(
                "INSERT INTO profile_sources (name, kind, text, chars, truncated, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![new.name, new.kind, new.text, chars as i64, new.truncated, now()],
            )?;
            let id = tx.last_insert_rowid();
            Ok(tx.query_row(
                &format!("SELECT {SOURCE_COLUMNS} FROM profile_sources WHERE id = ?1"),
                [id],
                source_row,
            )?)
        })
    }

    /// Forget a document. `false` when there was none with that id.
    pub fn remove_profile_source(&self, id: i64) -> Result<bool> {
        self.with_conn(|c| Ok(c.execute("DELETE FROM profile_sources WHERE id = ?1", [id])? > 0))
    }

    /// A fingerprint of the documents (ids and texts): a profile read from other documents is stale.
    pub fn profile_sources_hash(&self) -> Result<String> {
        Ok(sources_hash(&self.profile_sources()?))
    }

    /// The profile last read, if any.
    pub fn stored_profile(&self) -> Result<Option<StoredProfile>> {
        self.with_conn(|c| {
            let row = c
                .query_row("SELECT yaml, sources_hash, left_out_json, built_at FROM profile WHERE id = 1", [], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get(3)?))
                })
                .optional()?;
            let Some((yaml, sources_hash, left_out_json, built_at)) = row else { return Ok(None) };
            let left_out = serde_json::from_str(&left_out_json).context("the stored profile's notes are not valid JSON")?;
            Ok(Some(StoredProfile { yaml, sources_hash, left_out, built_at }))
        })
    }

    /// Replace the stored profile.
    pub fn save_profile(&self, yaml: &str, sources_hash: &str, left_out: &[String]) -> Result<()> {
        let left_out_json = serde_json::to_string(left_out)?;
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO profile (id, yaml, sources_hash, left_out_json, built_at) VALUES (1, ?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET yaml = excluded.yaml, sources_hash = excluded.sources_hash,
                   left_out_json = excluded.left_out_json, built_at = excluded.built_at",
                params![yaml, sources_hash, left_out_json, now()],
            )?;
            Ok(())
        })
    }

    /// Forget the stored profile (the user removed every document).
    pub fn clear_profile(&self) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM profile", [])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_round_trip_with_a_total_cap_and_a_hash_that_follows_them() {
        let store = Store::open_in_memory().unwrap();
        let empty = store.profile_sources_hash().unwrap();
        let a = store.add_profile_source(&NewProfileSource::file("cv.pdf", "Built APIs", true), 20).unwrap();
        assert_eq!((a.name.as_str(), a.kind.as_str(), a.chars, a.truncated), ("cv.pdf", "file", 10, true));
        let with_a = store.profile_sources_hash().unwrap();
        assert_ne!(empty, with_a);

        let err = store.add_profile_source(&NewProfileSource::text("Note", "eleven char"), 20).unwrap_err();
        assert!(err.to_string().contains("more than 20 characters"), "{err}");
        let b = store.add_profile_source(&NewProfileSource::text("Note", "Rust"), 20).unwrap();
        assert_eq!(store.profile_sources().unwrap().iter().map(|s| s.id).collect::<Vec<_>>(), [a.id, b.id]);

        assert!(store.remove_profile_source(b.id).unwrap());
        assert!(!store.remove_profile_source(b.id).unwrap());
        assert_eq!(store.profile_sources_hash().unwrap(), with_a);
    }

    #[test]
    fn the_stored_profile_is_replaced_and_cleared() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.stored_profile().unwrap(), None);
        store.save_profile("basics: {name: A}\n", "h1", &["Skill “Kafka”".into()]).unwrap();
        store.save_profile("basics: {name: B}\n", "h2", &[]).unwrap();
        let p = store.stored_profile().unwrap().unwrap();
        assert_eq!((p.yaml.as_str(), p.sources_hash.as_str(), p.left_out.len()), ("basics: {name: B}\n", "h2", 0));
        store.clear_profile().unwrap();
        assert_eq!(store.stored_profile().unwrap(), None);
    }
}
