//! A scripted [`Llm`] for tests in other modules (pipeline, apply, mail): no network, no schema
//! validation, replies are returned in order.

use std::collections::VecDeque;
use std::sync::Mutex;

use anyhow::anyhow;
use serde_json::Value;

use super::{Llm, LlmFuture};

/// One recorded `complete_json` call.
#[derive(Clone, Debug)]
pub struct FakeCall {
    pub system: String,
    pub user: String,
    pub schema: Value,
}

pub struct FakeLlm {
    replies: Mutex<VecDeque<Result<Value, String>>>,
    calls: Mutex<Vec<FakeCall>>,
    models: Vec<String>,
}

impl FakeLlm {
    /// Answer successive `complete_json` calls with `replies`; further calls fail.
    pub fn with_replies(replies: impl IntoIterator<Item = Value>) -> Self {
        Self {
            replies: Mutex::new(replies.into_iter().map(Ok).collect()),
            calls: Mutex::default(),
            models: vec!["fake-model".into()],
        }
    }

    /// Every call fails with `message`.
    pub fn with_error(message: &str) -> Self {
        Self {
            replies: Mutex::new(std::iter::repeat_with(|| Err(message.to_string())).take(64).collect()),
            calls: Mutex::default(),
            models: Vec::new(),
        }
    }

    /// Calls received so far.
    pub fn calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Llm for FakeLlm {
    fn complete_json<'a>(&'a self, system: &'a str, user: &'a str, schema: &'a Value) -> LlmFuture<'a, Value> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(FakeCall {
            system: system.into(),
            user: user.into(),
            schema: schema.clone(),
        });
        let next = self.replies.lock().unwrap_or_else(|e| e.into_inner()).pop_front();
        Box::pin(std::future::ready(match next {
            Some(Ok(v)) => Ok(v),
            Some(Err(m)) => Err(anyhow!(m)),
            None => Err(anyhow!("FakeLlm: no more scripted replies")),
        }))
    }

    fn list_models(&self) -> LlmFuture<'_, Vec<String>> {
        Box::pin(std::future::ready(Ok(self.models.clone())))
    }
}
