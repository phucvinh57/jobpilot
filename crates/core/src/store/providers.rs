use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row};

use super::Store;

/// Wire protocol of an AI provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProviderKind {
    /// OpenAI-compatible `/chat/completions` (OpenAI, OpenRouter, Groq, Ollama, ...).
    #[default]
    OpenAi,
    /// Anthropic-compatible `/v1/messages`.
    Anthropic,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAi => "openai",
            ProviderKind::Anthropic => "anthropic",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "openai" => Some(ProviderKind::OpenAi),
            "anthropic" => Some(ProviderKind::Anthropic),
            _ => None,
        }
    }
}

/// API keys are NOT stored here; they live in the keyring (see `secrets`).
#[derive(Clone, Debug)]
pub struct Provider {
    pub id: i64,
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
    /// `json_schema` (default) or `json_object` fallback; interpreted by `llm`.
    pub json_mode: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default)]
pub struct NewProvider {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    pub model: String,
}

const COLS: &str = "id, name, kind, base_url, model, json_mode, is_default";

impl Provider {
    fn from_row(r: &Row) -> rusqlite::Result<Self> {
        let kind: String = r.get(2)?;
        Ok(Self {
            id: r.get(0)?,
            name: r.get(1)?,
            kind: ProviderKind::parse(&kind).ok_or_else(|| {
                rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, format!("kind `{kind}`").into())
            })?,
            base_url: r.get(3)?,
            model: r.get(4)?,
            json_mode: r.get(5)?,
            is_default: r.get(6)?,
        })
    }
}

impl Store {
    pub fn add_provider(&self, new: &NewProvider) -> Result<Provider> {
        let id = self.with_conn(|c| {
            c.execute(
                "INSERT INTO providers (name, kind, base_url, model) VALUES (?1, ?2, ?3, ?4)",
                params![new.name, new.kind.as_str(), new.base_url, new.model],
            )?;
            Ok(c.last_insert_rowid())
        })?;
        self.get_provider(id)?.context("provider vanished after insert")
    }

    pub fn get_provider(&self, id: i64) -> Result<Option<Provider>> {
        self.with_conn(|c| {
            Ok(c.query_row(&format!("SELECT {COLS} FROM providers WHERE id = ?1"), [id], Provider::from_row)
                .optional()?)
        })
    }

    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        self.with_conn(|c| {
            let mut stmt = c.prepare(&format!("SELECT {COLS} FROM providers ORDER BY id"))?;
            let rows = stmt.query_map([], Provider::from_row)?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn default_provider(&self) -> Result<Option<Provider>> {
        self.with_conn(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLS} FROM providers WHERE is_default = 1 LIMIT 1"),
                [],
                Provider::from_row,
            )
            .optional()?)
        })
    }

    /// Update name/kind/base_url/model/json_mode (not the default flag).
    pub fn update_provider(&self, p: &Provider) -> Result<()> {
        self.with_conn(|c| {
            let n = c.execute(
                "UPDATE providers SET name = ?1, kind = ?2, base_url = ?3, model = ?4, json_mode = ?5 WHERE id = ?6",
                params![p.name, p.kind.as_str(), p.base_url, p.model, p.json_mode, p.id],
            )?;
            anyhow::ensure!(n == 1, "provider {} not found", p.id);
            Ok(())
        })
    }

    /// Exactly one default at a time.
    pub fn set_default_provider(&self, id: i64) -> Result<()> {
        self.with_tx(|tx| {
            tx.execute("UPDATE providers SET is_default = 0", [])?;
            let n = tx.execute("UPDATE providers SET is_default = 1 WHERE id = ?1", [id])?;
            anyhow::ensure!(n == 1, "provider {id} not found");
            Ok(())
        })
    }

    pub fn delete_provider(&self, id: i64) -> Result<()> {
        self.with_conn(|c| {
            c.execute("DELETE FROM providers WHERE id = ?1", [id])?;
            Ok(())
        })
    }
}
