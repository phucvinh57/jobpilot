use anyhow::{Context, Result};
use rusqlite::{params, OptionalExtension, Row};

use super::Store;

/// Type of AI provider, as the user picks it in Settings > AI providers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProviderKind {
    /// Any OpenAI-compatible `/chat/completions` endpoint (OpenRouter, Groq, Ollama, LM Studio, ...);
    /// the user enters the base URL.
    #[default]
    OpenAiCompatible,
    /// The official OpenAI API (`https://api.openai.com/v1`).
    OpenAi,
    /// Any Anthropic-compatible `/v1/messages` endpoint; the user enters the base URL.
    AnthropicCompatible,
    /// The official Anthropic API (`https://api.anthropic.com`).
    Anthropic,
    /// OpenAI Codex through the user's ChatGPT sign-in (fixed backend URL, no API key).
    Codex,
}

/// Wire protocol a [`ProviderKind`] speaks (selects the client in `llm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    /// `POST {base}/chat/completions`.
    ChatCompletions,
    /// `POST {base}/v1/messages`.
    Messages,
    /// The Codex backend's `POST /responses` (streamed).
    CodexResponses,
}

impl ProviderKind {
    /// Every kind, in the order the UI offers them.
    pub const ALL: [ProviderKind; 5] = [
        ProviderKind::OpenAiCompatible,
        ProviderKind::AnthropicCompatible,
        ProviderKind::OpenAi,
        ProviderKind::Anthropic,
        ProviderKind::Codex,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::OpenAiCompatible => "openai_compatible",
            ProviderKind::OpenAi => "openai",
            ProviderKind::AnthropicCompatible => "anthropic_compatible",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Codex => "codex",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    pub fn protocol(self) -> Protocol {
        match self {
            ProviderKind::OpenAiCompatible | ProviderKind::OpenAi => Protocol::ChatCompletions,
            ProviderKind::AnthropicCompatible | ProviderKind::Anthropic => Protocol::Messages,
            ProviderKind::Codex => Protocol::CodexResponses,
        }
    }

    /// An official service with a fixed base URL (OpenAI, Anthropic, Codex); the compatible kinds
    /// need the user's base URL.
    pub fn is_official(self) -> bool {
        matches!(self, ProviderKind::OpenAi | ProviderKind::Anthropic | ProviderKind::Codex)
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
    /// The *learned* JSON mode of the endpoint, written by the engine (`llm`), never by the user:
    /// `json_schema` (default), `json_object` (the endpoint refused a schema) or `prompt` (no
    /// `response_format` at all; the schema is in the system prompt).
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_map_to_protocols() {
        for kind in ProviderKind::ALL {
            assert_eq!(ProviderKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(ProviderKind::parse("bogus"), None);
        assert_eq!(ProviderKind::default(), ProviderKind::OpenAiCompatible);
        assert_eq!(ProviderKind::OpenAi.protocol(), Protocol::ChatCompletions);
        assert_eq!(ProviderKind::OpenAiCompatible.protocol(), Protocol::ChatCompletions);
        assert_eq!(ProviderKind::Anthropic.protocol(), Protocol::Messages);
        assert_eq!(ProviderKind::AnthropicCompatible.protocol(), Protocol::Messages);
        assert_eq!(ProviderKind::Codex.protocol(), Protocol::CodexResponses);
        let official: Vec<_> = ProviderKind::ALL.into_iter().filter(|k| k.is_official()).collect();
        assert_eq!(official, [ProviderKind::OpenAi, ProviderKind::Anthropic, ProviderKind::Codex]);
    }
}
