use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
    /// For assistant messages requesting tools.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// For tool-result messages: which call this answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

impl ChatMessage {
    pub fn system(s: impl Into<String>) -> Self {
        Self::simple(Role::System, s)
    }
    pub fn user(s: impl Into<String>) -> Self {
        Self::simple(Role::User, s)
    }
    pub fn assistant(s: impl Into<String>) -> Self {
        Self::simple(Role::Assistant, s)
    }
    fn simple(role: Role, s: impl Into<String>) -> Self {
        ChatMessage {
            role,
            content: s.into(),
            tool_calls: vec![],
            tool_call_id: None,
        }
    }
    pub fn tool_result(id: impl Into<String>, content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Tool,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(id.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    /// JSON arguments object.
    pub arguments: serde_json::Value,
}

/// What a model returns for one completion turn.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    pub message: ChatMessage,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("model transport error: {0}")]
    Transport(String),
    #[error("model returned invalid response: {0}")]
    Invalid(String),
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<crate::tool::ToolSpec>,
}

/// Abstraction over a chat model backend (Ollama now; frontier later).
#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_messages() {
        let m = ChatMessage::user("hi");
        assert_eq!(m.role, Role::User);
        assert!(m.tool_calls.is_empty());
        let t = ChatMessage::tool_result("call_1", "ok");
        assert_eq!(t.tool_call_id.as_deref(), Some("call_1"));
    }
}
