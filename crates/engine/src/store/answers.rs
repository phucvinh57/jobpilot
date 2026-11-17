use anyhow::Result;
use rusqlite::params;

use super::{now, Store};

/// Application answers the user asked to remember: question -> answer, keyed by the
/// question's loose form (`apply::answer::normalize_key`), so "Notice period?" and "notice period"
/// are one answer.
impl Store {
    /// Every remembered `(question, answer)`, oldest first.
    pub fn saved_answers(&self) -> Result<Vec<(String, String)>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare("SELECT question, answer FROM application_answers ORDER BY rowid")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Remember `answer` to `question`, replacing an earlier answer with the same `key`.
    pub fn save_answer(&self, key: &str, question: &str, answer: &str) -> Result<()> {
        self.with_conn(|c| {
            c.execute(
                "INSERT INTO application_answers (key, question, answer, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(key) DO UPDATE SET question = excluded.question, answer = excluded.answer,
                   updated_at = excluded.updated_at",
                params![key, question, answer, now()],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_with_the_same_key_is_replaced_in_place() {
        let store = Store::open_in_memory().unwrap();
        store.save_answer("notice period", "Notice period?", "1 month").unwrap();
        store.save_answer("salary", "Salary", "Negotiable").unwrap();
        store.save_answer("notice period", "notice period", "2 weeks").unwrap();
        assert_eq!(
            store.saved_answers().unwrap(),
            [("notice period".to_string(), "2 weeks".to_string()), ("Salary".into(), "Negotiable".into())]
        );
    }
}
