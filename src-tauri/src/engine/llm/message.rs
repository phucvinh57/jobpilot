//! Multi-turn input for [`Llm::chat_json`](super::Llm::chat_json).

use serde::{Deserialize, Serialize};

/// Who wrote a chat message.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    /// `user` / `assistant` (as stored and as sent).
    pub fn as_str(self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }

    /// Inverse of [`Role::as_str`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        }
    }
}

/// One turn of a conversation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into() }
    }
}

/// Fold a conversation into one user text for providers without a native multi-turn call: each
/// message as `[user]` / `[assistant]` followed by its text, then the instruction to answer the last one.
pub(crate) fn fold(messages: &[Message]) -> String {
    let mut out = String::from("Conversation so far:\n");
    for m in messages {
        out.push_str(&format!("\n[{}]\n{}\n", m.role.as_str(), m.content.trim()));
    }
    out.push_str("\nReply to the last [user] message.");
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::engine::llm::fake::FakeLlm;
    use crate::engine::llm::Llm;

    #[tokio::test]
    async fn chat_json_folds_the_history_into_one_complete_json_call() {
        let llm = FakeLlm::with_replies([json!({"reply": "Remote or on-site?"})]);
        let schema = json!({"type": "object"});
        let history = [Message::user("I want a backend job"), Message::assistant("Which city?"), Message::user("Hanoi")];
        let out = llm.chat_json("You interview the user.", &history, &schema).await.unwrap();
        assert_eq!(out, json!({"reply": "Remote or on-site?"}));

        let calls = llm.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].system, "You interview the user.");
        assert_eq!(calls[0].schema, schema);
        let user = &calls[0].user;
        let (a, b, c) = (
            user.find("[user]\nI want a backend job").unwrap(),
            user.find("[assistant]\nWhich city?").unwrap(),
            user.find("[user]\nHanoi").unwrap(),
        );
        assert!(a < b && b < c, "order kept: {user}");
        assert!(user.ends_with("Reply to the last [user] message."));
    }

    #[tokio::test]
    async fn chat_json_propagates_errors() {
        let llm = FakeLlm::with_error("HTTP 503 Service Unavailable");
        let err = llm.chat_json("s", &[Message::user("hi")], &json!({})).await.unwrap_err();
        assert!(err.to_string().contains("503"));
    }

    #[test]
    fn role_and_message_serde() {
        assert_eq!(serde_json::to_value(Message::assistant("x")).unwrap(), json!({"role": "assistant", "content": "x"}));
        let m: Message = serde_json::from_value(json!({"role": "user", "content": "y"})).unwrap();
        assert_eq!(m, Message::user("y"));
        assert!(serde_json::from_value::<Message>(json!({"role": "system", "content": "z"})).is_err());
        assert_eq!(Role::parse("assistant"), Some(Role::Assistant));
        assert_eq!(Role::parse("system"), None);
        assert_eq!(fold(&[]), "Conversation so far:\n\nReply to the last [user] message.");
    }
}
