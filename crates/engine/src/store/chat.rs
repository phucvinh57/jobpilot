use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Row};

use super::{now, Store};
use crate::llm::{Message, Role};

/// One stored chat turn (the preference chat uses thread `prefs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatMessage {
    pub id: i64,
    pub thread: String,
    pub role: Role,
    pub content: String,
    pub at: DateTime<Utc>,
}

impl ChatMessage {
    /// The model-facing form (no id or time).
    pub fn message(&self) -> Message {
        Message { role: self.role, content: self.content.clone() }
    }

    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let role: String = r.get(2)?;
        Ok(Self {
            id: r.get(0)?,
            thread: r.get(1)?,
            role: Role::parse(&role).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, format!("role `{role}`").into())
            })?,
            content: r.get(3)?,
            at: r.get(4)?,
        })
    }
}

const COLS: &str = "id, thread, role, content, at";

impl Store {
    /// Append a message to `thread`.
    pub fn add_chat_message(&self, thread: &str, role: Role, content: &str) -> Result<ChatMessage> {
        let id = self.with_conn(|c| {
            c.execute(
                "INSERT INTO chat_messages (thread, role, content, at) VALUES (?1, ?2, ?3, ?4)",
                params![thread, role.as_str(), content, now()],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM chat_messages WHERE id = ?1"), [id], ChatMessage::from_row)?)
        })
        .context("chat message vanished after insert")
    }

    /// The whole conversation of `thread`, oldest first.
    pub fn chat_messages(&self, thread: &str) -> Result<Vec<ChatMessage>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLS} FROM chat_messages WHERE thread = ?1 ORDER BY id"))?;
            let rows = stmt.query_map([thread], ChatMessage::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    /// The newest message of `thread`, if any.
    pub fn last_chat_message(&self, thread: &str) -> Result<Option<ChatMessage>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLS} FROM chat_messages WHERE thread = ?1 ORDER BY id DESC LIMIT 1"),
                [thread],
                ChatMessage::from_row,
            )
            .optional()?)
        })
    }

    /// Forget a conversation.
    pub fn clear_chat(&self, thread: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM chat_messages WHERE thread = ?1", [thread])?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threads_are_separate_and_ordered() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.chat_messages("prefs").unwrap().is_empty());
        assert!(store.last_chat_message("prefs").unwrap().is_none());

        let a = store.add_chat_message("prefs", Role::User, "I want remote work").unwrap();
        store.add_chat_message("other", Role::User, "unrelated").unwrap();
        let b = store.add_chat_message("prefs", Role::Assistant, "Which time zones?").unwrap();
        assert!(b.id > a.id);

        let msgs = store.chat_messages("prefs").unwrap();
        assert_eq!(msgs.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(), ["I want remote work", "Which time zones?"]);
        assert_eq!(msgs[1].role, Role::Assistant);
        assert_eq!(msgs[0].message(), Message::user("I want remote work"));
        assert_eq!(store.last_chat_message("prefs").unwrap().unwrap().id, b.id);

        store.clear_chat("prefs").unwrap();
        assert!(store.chat_messages("prefs").unwrap().is_empty());
        assert_eq!(store.chat_messages("other").unwrap().len(), 1);
    }

    #[test]
    fn unknown_roles_are_refused_by_the_schema() {
        let store = Store::open_in_memory().unwrap();
        let bad = store.with_conn(|c| {
            Ok(c.execute("INSERT INTO chat_messages (thread, role, content, at) VALUES ('t', 'system', 'x', 'now')", [])?)
        });
        assert!(bad.is_err());
    }
}
