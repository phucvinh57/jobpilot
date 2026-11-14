//! P1 - AI providers: OpenAI-compatible and Anthropic clients with JSON-schema output.
//!
//! ```text
//! let llm = core.default_llm()?;                       // or core.llm_for(provider_id)
//! let out = llm.complete_json(system, user, &schema).await?;   // serde_json::Value, validated
//! ```
//!
//! `complete_json` validates the model output against `schema` (tolerating ```` ```json ````
//! fences) and retries once, quoting the validation errors back to the model. Errors are
//! human-readable (`Anthropic: HTTP 401 Unauthorized: invalid x-api-key`) and never contain the
//! API key. API keys live only in the keyring under [`api_key_secret`]`(provider_id)`.

pub mod anthropic;
pub mod fake;
mod http;
pub mod message;
pub mod openai;
pub mod schema;

#[cfg(test)]
mod http_tests;

use std::future::Future;
use std::pin::Pin;

use anyhow::{bail, Context, Result};
use serde_json::{json, Value};

use crate::engine::store::{NewProvider, Provider, ProviderKind};
use crate::engine::Core;

pub use anthropic::AnthropicClient;
pub use message::{Message, Role};
pub use openai::OpenAiClient;

/// Boxed future returned by [`Llm`] methods (keeps the trait object-safe without `async-trait`).
pub type LlmFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// A configured AI provider. Obtain one with [`from_provider`], [`Core::llm_for`] or
/// [`Core::default_llm`]; it is cheap to create and `Send + Sync`.
pub trait Llm: Send + Sync {
    /// Ask for one JSON document that conforms to `schema` (plain JSON Schema). The result is
    /// validated against `schema`; on a mismatch the call is retried once with the errors.
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value>;

    /// Like [`complete_json`](Llm::complete_json) for a conversation: `messages` is the history
    /// (oldest first, normally ending with a user message). The default folds it into one user text
    /// with `[user]` / `[assistant]` labels, so providers need no change; one can override it with
    /// a native multi-turn call.
    fn chat_json<'a>(&'a self, system: &'a str, messages: &'a [Message], schema: &'a Value) -> LlmFuture<'a, Value> {
        Box::pin(async move { self.complete_json(system, &message::fold(messages), schema).await })
    }

    /// Model ids offered by the provider (for the model picker).
    fn list_models(&self) -> LlmFuture<'_, Vec<String>>;
}

/// Default base URL for a provider kind.
pub fn default_base_url(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => openai::DEFAULT_BASE_URL,
        ProviderKind::Anthropic => anthropic::DEFAULT_BASE_URL,
    }
}

/// Default model for a provider kind (`""` = the user must pick one).
pub fn default_model(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::OpenAi => "",
        ProviderKind::Anthropic => anthropic::DEFAULT_MODEL,
    }
}

/// Build the client for `provider`. `api_key` is optional (Ollama and friends need none).
pub fn from_provider(provider: &Provider, api_key: Option<String>) -> Box<dyn Llm> {
    match provider.kind {
        ProviderKind::OpenAi => Box::new(OpenAiClient::new(
            &provider.name,
            &provider.base_url,
            &provider.model,
            api_key,
            &provider.json_mode,
        )),
        ProviderKind::Anthropic => {
            Box::new(AnthropicClient::new(&provider.name, &provider.base_url, &provider.model, api_key))
        }
    }
}

/// Run a tiny `{ok: boolean}` request: `Ok(())` when the provider answered as asked.
pub async fn test_llm(llm: &dyn Llm) -> Result<()> {
    let result = test_llm_inner(llm).await;
    match &result {
        Ok(()) => tracing::info!("llm: provider test passed"),
        Err(e) => tracing::warn!("llm: provider test failed: {e:#}"),
    }
    result
}

async fn test_llm_inner(llm: &dyn Llm) -> Result<()> {
    let schema = json!({
        "type": "object",
        "properties": {"ok": {"type": "boolean"}},
        "required": ["ok"]
    });
    let out = llm
        .complete_json("You are a connectivity check. Reply with JSON only.", "Return {\"ok\": true}.", &schema)
        .await?;
    if out.get("ok") == Some(&Value::Bool(true)) {
        Ok(())
    } else {
        bail!("the model answered, but not as asked: {out}")
    }
}

/// Shared validate-and-retry loop. `call(user_message)` performs one raw model call and returns
/// the answer text. Transport/API errors propagate immediately (no retry); a bad answer (not JSON
/// or not matching `schema`) is retried ONCE with the problems appended to the user message.
pub(crate) async fn complete_validated<F, Fut>(user: &str, schema: &Value, mut call: F) -> Result<Value>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<String>>,
{
    let validator = schema::compile(schema)?;
    let text = call(user.to_string()).await?;
    let problem = match schema::parse_and_validate(&validator, &text) {
        Ok(v) => return Ok(v),
        Err(p) => p,
    };
    tracing::debug!("model output rejected, retrying once: {problem}");
    let retry =
        format!("{user}\n\nYour previous answer was rejected: {problem}\n\nAnswer again with corrected JSON only.");
    let text = call(retry).await?;
    schema::parse_and_validate(&validator, &text)
        .map_err(|p| anyhow::anyhow!("the model's answer was still invalid after one retry: {p}"))
}

/// Keyring key holding a provider's API key.
pub fn api_key_secret(provider_id: i64) -> String {
    format!("provider:{provider_id}:api_key")
}

impl Core {
    /// The stored API key of a provider (`None` when none is set).
    pub fn provider_api_key(&self, provider_id: i64) -> Result<Option<String>> {
        Ok(self.secrets.get(&api_key_secret(provider_id))?.filter(|k| !k.is_empty()))
    }

    /// Client for one provider, with its stored key.
    pub fn llm_for(&self, provider_id: i64) -> Result<Box<dyn Llm>> {
        let provider = self
            .store
            .get_provider(provider_id)?
            .with_context(|| format!("AI provider {provider_id} no longer exists"))?;
        let key = self.provider_api_key(provider_id)?;
        Ok(from_provider(&provider, key))
    }

    /// Client for the default provider.
    pub fn default_llm(&self) -> Result<Box<dyn Llm>> {
        match self.store.default_provider()? {
            Some(p) => self.llm_for(p.id),
            None => bail!("no AI provider is set up: add one in Settings > AI providers and make it the default"),
        }
    }

    /// Create (`provider.id == 0`) or update a provider. `api_key`: `Some(non-empty)` stores a new
    /// key in the keyring, `None`/empty keeps the existing one. The first provider becomes the
    /// default automatically.
    pub fn save_provider(&self, provider: &Provider, api_key: Option<&str>) -> Result<Provider> {
        let mut saved = if provider.id == 0 {
            let created = self.store.add_provider(&NewProvider {
                name: provider.name.clone(),
                kind: provider.kind,
                base_url: provider.base_url.clone(),
                model: provider.model.clone(),
            })?;
            let with_mode = Provider { json_mode: provider.json_mode.clone(), ..created };
            self.store.update_provider(&with_mode)?;
            with_mode
        } else {
            self.store.update_provider(provider)?;
            self.store.get_provider(provider.id)?.context("provider vanished after update")?
        };
        if let Some(key) = api_key.map(str::trim).filter(|k| !k.is_empty()) {
            self.secrets.set(&api_key_secret(saved.id), key)?;
        }
        if self.store.default_provider()?.is_none() {
            self.store.set_default_provider(saved.id)?;
            saved.is_default = true;
        }
        Ok(saved)
    }

    /// Delete a provider and its stored key; another provider becomes the default if needed.
    pub fn remove_provider(&self, provider_id: i64) -> Result<()> {
        self.secrets.delete(&api_key_secret(provider_id))?;
        self.store.delete_provider(provider_id)?;
        if self.store.default_provider()?.is_none() {
            if let Some(first) = self.store.list_providers()?.first() {
                self.store.set_default_provider(first.id)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::engine::{Paths, Secrets};

    fn schema() -> Value {
        json!({"type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]})
    }

    /// Scripted raw model: returns the queued texts in order, recording each user message.
    #[allow(clippy::type_complexity)]
    fn scripted(
        texts: Vec<&'static str>,
    ) -> (Arc<Mutex<Vec<String>>>, impl FnMut(String) -> std::future::Ready<Result<String>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut queue = texts.into_iter();
        (seen, move |user: String| {
            log.lock().unwrap().push(user);
            std::future::ready(queue.next().map(str::to_string).ok_or_else(|| anyhow::anyhow!("script exhausted")))
        })
    }

    #[tokio::test]
    async fn valid_first_answer_is_not_retried() {
        let (seen, call) = scripted(vec!["```json\n{\"n\": 3}\n```"]);
        let out = complete_validated("U", &schema(), call).await.unwrap();
        assert_eq!(out, json!({"n": 3}));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0], "U");
    }

    #[tokio::test]
    async fn invalid_answer_is_retried_once_with_the_errors() {
        let (seen, call) = scripted(vec![r#"{"n": "three"}"#, r#"{"n": 3}"#]);
        let out = complete_validated("U", &schema(), call).await.unwrap();
        assert_eq!(out, json!({"n": 3}));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[1].starts_with("U\n\n"), "{}", seen[1]);
        assert!(seen[1].contains("rejected"), "{}", seen[1]);
        assert!(seen[1].contains("/n"), "the validation error names the field: {}", seen[1]);
    }

    #[tokio::test]
    async fn not_json_is_retried_then_fails_after_one_retry() {
        let (seen, call) = scripted(vec!["Sure! Here you go", "still prose", "never reached"]);
        let err = complete_validated("U", &schema(), call).await.unwrap_err().to_string();
        assert!(err.contains("still invalid after one retry"), "{err}");
        assert!(err.contains("not valid JSON"), "{err}");
        assert_eq!(seen.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn transport_errors_are_not_retried() {
        let calls = AtomicUsize::new(0);
        let err = complete_validated("U", &schema(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Err::<String, _>(anyhow::anyhow!("HTTP 401 Unauthorized: invalid x-api-key")))
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("invalid x-api-key"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn invalid_schema_fails_before_any_call() {
        let calls = AtomicUsize::new(0);
        let err = complete_validated("U", &json!({"type": "bogus"}), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(Ok(String::new()))
        })
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("invalid JSON schema"), "{err}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_llm_checks_the_answer() {
        let good = fake::FakeLlm::with_replies([json!({"ok": true})]);
        test_llm(&good).await.unwrap();
        let bad = fake::FakeLlm::with_replies([json!({"ok": false})]);
        assert!(test_llm(&bad).await.unwrap_err().to_string().contains("not as asked"));
        let down = fake::FakeLlm::with_error("HTTP 503 Service Unavailable");
        assert!(test_llm(&down).await.unwrap_err().to_string().contains("503"));
    }

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().unwrap();
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory()).unwrap();
        (dir, core)
    }

    fn draft(name: &str, kind: ProviderKind) -> Provider {
        Provider {
            id: 0,
            name: name.into(),
            kind,
            base_url: default_base_url(kind).into(),
            model: "m1".into(),
            json_mode: "json_schema".into(),
            is_default: false,
        }
    }

    #[test]
    fn save_provider_stores_key_in_secrets_and_defaults_the_first() {
        let (_dir, core) = core();
        let mut d = draft("OpenAI", ProviderKind::OpenAi);
        d.json_mode = "json_object".into();
        let a = core.save_provider(&d, Some("sk-test-key")).unwrap();
        assert!(a.id > 0 && a.is_default);
        assert_eq!(a.json_mode, "json_object");
        assert_eq!(core.provider_api_key(a.id).unwrap().as_deref(), Some("sk-test-key"));
        assert_eq!(core.secrets.get(&format!("provider:{}:api_key", a.id)).unwrap().as_deref(), Some("sk-test-key"));

        let b = core.save_provider(&draft("Claude", ProviderKind::Anthropic), None).unwrap();
        assert!(!b.is_default, "only the first provider is auto-default");
        assert_eq!(core.provider_api_key(b.id).unwrap(), None);
        assert_eq!(core.store.default_provider().unwrap().unwrap().id, a.id);

        // Update keeps the key when none is given, replaces it when one is.
        let mut a2 = a.clone();
        a2.model = "m2".into();
        core.save_provider(&a2, None).unwrap();
        assert_eq!(core.store.get_provider(a.id).unwrap().unwrap().model, "m2");
        assert_eq!(core.provider_api_key(a.id).unwrap().as_deref(), Some("sk-test-key"));
        core.save_provider(&a2, Some("sk-new")).unwrap();
        assert_eq!(core.provider_api_key(a.id).unwrap().as_deref(), Some("sk-new"));

        // The key never reaches the database.
        let rows = format!("{:?}", core.store.list_providers().unwrap());
        assert!(!rows.contains("sk-new") && !rows.contains("sk-test-key"));
    }

    #[test]
    fn default_llm_and_removal() {
        let (_dir, core) = core();
        assert!(core.default_llm().err().unwrap().to_string().contains("no AI provider"));

        let a = core.save_provider(&draft("A", ProviderKind::OpenAi), Some("k1-secret")).unwrap();
        let b = core.save_provider(&draft("B", ProviderKind::Anthropic), Some("k2-secret")).unwrap();
        assert!(core.default_llm().is_ok());
        assert!(core.llm_for(b.id).is_ok());
        assert!(core.llm_for(9999).is_err());

        core.store.set_default_provider(b.id).unwrap();
        core.remove_provider(b.id).unwrap();
        assert_eq!(core.provider_api_key(b.id).unwrap(), None, "key removed with the provider");
        assert_eq!(core.store.default_provider().unwrap().unwrap().id, a.id, "default falls back");
        core.remove_provider(a.id).unwrap();
        assert!(core.default_llm().is_err());
    }
}
