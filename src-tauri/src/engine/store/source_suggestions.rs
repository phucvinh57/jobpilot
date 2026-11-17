//! `source_suggestions`: job sources the "Suggest sources" search found and checked
//! (`discover::suggest`). One row per `(kind, input)`; the user adds one (`added`) or dismisses it
//! (`dismissed`), and a later search never brings either back.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};

/// Where a suggestion stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SuggestionState {
    /// Found and checked, waiting for the user.
    #[default]
    New,
    /// The user added it as a job source.
    Added,
    /// The user dismissed it.
    Dismissed,
}

impl SuggestionState {
    /// `new` / `added` / `dismissed` (as stored).
    pub fn as_str(self) -> &'static str {
        match self {
            SuggestionState::New => "new",
            SuggestionState::Added => "added",
            SuggestionState::Dismissed => "dismissed",
        }
    }

    /// Inverse of [`SuggestionState::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "new" => Some(SuggestionState::New),
            "added" => Some(SuggestionState::Added),
            "dismissed" => Some(SuggestionState::Dismissed),
            _ => None,
        }
    }
}

/// A suggested job source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceSuggestion {
    pub id: i64,
    /// The source kind (`greenhouse`, `career_page`, ...).
    pub kind: String,
    /// What to add as the source's address (the input of `add_job_source`).
    pub input: String,
    /// The name the source check read (the board's company, the page's host).
    pub name: String,
    /// Postings the check found.
    pub jobs: u32,
    /// Of those, postings whose title fits the wanted roles.
    pub matching: u32,
    /// The search query that found it.
    pub query: String,
    pub state: SuggestionState,
    /// When it was first found.
    pub found_at: DateTime<Utc>,
}

/// A suggestion to save ([`Store::upsert_suggestion`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NewSuggestion {
    pub kind: String,
    pub input: String,
    pub name: String,
    pub jobs: u32,
    pub matching: u32,
    pub query: String,
}

const COLS: &str = "id, kind, input, name, jobs, matching, query, state, found_at";

impl SourceSuggestion {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let state: String = r.get(7)?;
        Ok(Self {
            id: r.get(0)?,
            kind: r.get(1)?,
            input: r.get(2)?,
            name: r.get(3)?,
            jobs: r.get(4)?,
            matching: r.get(5)?,
            query: r.get(6)?,
            state: SuggestionState::parse(&state).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, format!("state `{state}`").into())
            })?,
            found_at: r.get(8)?,
        })
    }
}

impl Store {
    /// Save a checked suggestion: a new row, or fresh counts and name for a row that is still
    /// `new`. A suggestion the user added or dismissed is never revived (nor changed): `None` then.
    /// Returns the row as stored when it is `new`.
    pub fn upsert_suggestion(&self, new: &NewSuggestion) -> Result<Option<SourceSuggestion>> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO source_suggestions (kind, input, name, jobs, matching, query, found_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(kind, input) DO UPDATE SET
                     name = excluded.name, jobs = excluded.jobs, matching = excluded.matching, query = excluded.query
                 WHERE source_suggestions.state = 'new'",
                params![new.kind, new.input, new.name, new.jobs, new.matching, new.query, now()],
            )?;
            let row = c
                .query_row(
                    &format!("SELECT {COLS} FROM source_suggestions WHERE kind = ?1 AND input = ?2"),
                    params![new.kind, new.input],
                    SourceSuggestion::from_row,
                )
                .optional()?
                .context("suggestion vanished after insert")?;
            Ok((row.state == SuggestionState::New).then_some(row))
        })
    }

    /// Suggestions in `state`: the most matching postings first, then the most postings, then the
    /// oldest.
    pub fn list_suggestions(&self, state: SuggestionState) -> Result<Vec<SourceSuggestion>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!(
                "SELECT {COLS} FROM source_suggestions WHERE state = ?1 ORDER BY matching DESC, jobs DESC, id"
            ))?;
            let rows = stmt.query_map([state.as_str()], SourceSuggestion::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// Every suggestion ever saved, in any state (what a new search must not suggest again).
    pub fn all_suggestions(&self) -> Result<Vec<SourceSuggestion>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLS} FROM source_suggestions ORDER BY id"))?;
            let rows = stmt.query_map([], SourceSuggestion::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn get_suggestion(&self, id: i64) -> Result<Option<SourceSuggestion>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM source_suggestions WHERE id = ?1"), [id], SourceSuggestion::from_row)
                .optional()?)
        })
    }

    /// Mark a suggestion added or dismissed (or new again). `Err` when there is no such row.
    pub fn set_suggestion_state(&self, id: i64, state: SuggestionState) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute("UPDATE source_suggestions SET state = ?1 WHERE id = ?2", params![state.as_str(), id])?;
            anyhow::ensure!(n == 1, "suggestion {id} not found");
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new(kind: &str, input: &str, jobs: u32, matching: u32) -> NewSuggestion {
        NewSuggestion { kind: kind.into(), input: input.into(), name: input.into(), jobs, matching, query: "q".into() }
    }

    #[test]
    fn a_new_row_is_inserted_then_updated_while_new() {
        let store = Store::open_in_memory().unwrap();
        let first = store.upsert_suggestion(&new("greenhouse", "boards.greenhouse.io/acme", 3, 1)).unwrap().unwrap();
        assert_eq!((first.state, first.jobs, first.matching, first.name.as_str()), (SuggestionState::New, 3, 1, "boards.greenhouse.io/acme"));
        let again = NewSuggestion { name: "Acme".into(), query: "q2".into(), ..new("greenhouse", "boards.greenhouse.io/acme", 9, 4) };
        let updated = store.upsert_suggestion(&again).unwrap().unwrap();
        assert_eq!(updated.id, first.id, "one row per (kind, input)");
        assert_eq!((updated.jobs, updated.matching, updated.name.as_str(), updated.query.as_str()), (9, 4, "Acme", "q2"));
        assert_eq!(updated.found_at, first.found_at, "first found stays");
        assert_eq!(store.get_suggestion(first.id).unwrap().unwrap(), updated);
        assert!(store.get_suggestion(999).unwrap().is_none());
    }

    #[test]
    fn added_and_dismissed_rows_are_never_revived_or_changed() {
        let store = Store::open_in_memory().unwrap();
        let a = store.upsert_suggestion(&new("lever", "jobs.lever.co/a", 2, 0)).unwrap().unwrap();
        let b = store.upsert_suggestion(&new("career_page", "https://b.example/careers", 5, 5)).unwrap().unwrap();
        store.set_suggestion_state(a.id, SuggestionState::Dismissed).unwrap();
        store.set_suggestion_state(b.id, SuggestionState::Added).unwrap();
        assert_eq!(store.upsert_suggestion(&new("lever", "jobs.lever.co/a", 20, 20)).unwrap(), None);
        assert_eq!(store.upsert_suggestion(&new("career_page", "https://b.example/careers", 1, 1)).unwrap(), None);
        let a = store.get_suggestion(a.id).unwrap().unwrap();
        assert_eq!((a.state, a.jobs, a.matching), (SuggestionState::Dismissed, 2, 0));
        assert_eq!(store.get_suggestion(b.id).unwrap().unwrap().state, SuggestionState::Added);
        assert!(store.list_suggestions(SuggestionState::New).unwrap().is_empty());
        assert_eq!(store.all_suggestions().unwrap().len(), 2);
        assert!(store.set_suggestion_state(999, SuggestionState::Added).is_err());
    }

    #[test]
    fn new_suggestions_are_listed_best_first() {
        let store = Store::open_in_memory().unwrap();
        for (input, jobs, matching) in [("a", 10, 1), ("b", 3, 3), ("c", 20, 1), ("d", 1, 0)] {
            store.upsert_suggestion(&new("ashby", input, jobs, matching)).unwrap();
        }
        let order: Vec<String> = store.list_suggestions(SuggestionState::New).unwrap().into_iter().map(|s| s.input).collect();
        assert_eq!(order, ["b", "c", "a", "d"]);
    }

    #[test]
    fn the_state_is_checked_by_the_schema() {
        let store = Store::open_in_memory().unwrap();
        let bad = store.with_conn(|c| {
            Ok(c.execute("INSERT INTO source_suggestions (kind, input, state, found_at) VALUES ('x', 'y', 'odd', '2026-01-01')", [])?)
        });
        assert!(bad.is_err());
        for state in [SuggestionState::New, SuggestionState::Added, SuggestionState::Dismissed] {
            assert_eq!(SuggestionState::parse(state.as_str()), Some(state));
        }
    }
}
