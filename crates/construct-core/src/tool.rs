use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Advertised tool schema handed to the model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema for the arguments object.
    pub parameters: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolError {
    #[error("tool execution failed: {0}")]
    Failed(String),
    #[error("invalid arguments: {0}")]
    BadArgs(String),
}

/// A callable capability the agent loop can invoke.
#[async_trait]
pub trait Tool: Send + Sync {
    fn spec(&self) -> ToolSpec;
    /// `args` is the JSON object the model produced.
    async fn call(&self, args: serde_json::Value) -> Result<String, ToolError>;
}
