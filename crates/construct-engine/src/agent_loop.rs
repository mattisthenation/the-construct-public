use construct_core::model::{ChatMessage, ChatRequest, ChatResponse, ModelError, ModelProvider};
use construct_core::tool::Tool;
use std::collections::HashMap;
use std::sync::Arc;

/// Call the model, retrying transient transport errors (e.g. a brief Ollama blip)
/// with a short backoff. Validation-class errors are returned immediately.
async fn chat_with_retry(
    provider: &dyn ModelProvider,
    req: ChatRequest,
) -> Result<ChatResponse, ModelError> {
    let mut attempt = 0u32;
    loop {
        match provider.chat(req.clone()).await {
            Ok(r) => return Ok(r),
            Err(ModelError::Transport(e)) if attempt < 2 => {
                attempt += 1;
                tracing::warn!("model transport error (attempt {attempt}/3): {e}; retrying");
                tokio::time::sleep(std::time::Duration::from_millis(250 * attempt as u64)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

pub struct LoopConfig {
    pub model: String,
    pub max_iterations: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    #[error("model error: {0}")]
    Model(String),
    #[error("exceeded max iterations ({0})")]
    Budget(usize),
}

/// The result of an agent loop: the final answer plus the evidence the agent
/// actually gathered (all tool outputs + every URL it passed to a tool). The
/// gate uses `evidence` to reject fabricated sources.
#[derive(Debug, Clone, PartialEq)]
pub struct LoopOutput {
    pub content: String,
    pub evidence: String,
}

/// Run the agent loop: the model may call tools repeatedly until it answers
/// with plain content or the iteration budget is exhausted. Returns the final
/// assistant text + gathered evidence. Performs NO file side effects.
pub async fn run_loop(
    provider: &dyn ModelProvider,
    tools: &HashMap<String, Arc<dyn Tool>>,
    mut messages: Vec<ChatMessage>,
    cfg: &LoopConfig,
) -> Result<LoopOutput, LoopError> {
    let specs: Vec<_> = tools.values().map(|t| t.spec()).collect();
    let mut evidence = String::new();

    for _ in 0..cfg.max_iterations {
        let req = ChatRequest {
            model: cfg.model.clone(),
            messages: messages.clone(),
            tools: specs.clone(),
        };
        let resp = chat_with_retry(provider, req)
            .await
            .map_err(|e| LoopError::Model(e.to_string()))?;
        let msg = resp.message;

        if msg.tool_calls.is_empty() {
            return Ok(LoopOutput {
                content: msg.content,
                evidence,
            });
        }

        // Record the assistant's tool-call turn, then answer each call.
        let calls = msg.tool_calls.clone();
        messages.push(msg);
        for call in calls {
            // Capture any URL argument so a fetched-but-not-echoed URL still counts as evidence.
            if let Some(u) = call.arguments.get("url").and_then(|v| v.as_str()) {
                evidence.push_str(u);
                evidence.push('\n');
            }
            let result = match tools.get(&call.name) {
                Some(tool) => tool
                    .call(call.arguments.clone())
                    .await
                    .unwrap_or_else(|e| format!("tool error: {e}")),
                None => format!("unknown tool: {}", call.name),
            };
            evidence.push_str(&result);
            evidence.push('\n');
            messages.push(ChatMessage::tool_result(call.id, result));
        }
    }
    Err(LoopError::Budget(cfg.max_iterations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::{EchoTool, ScriptedModel};
    use construct_core::model::{ChatResponse, Role, ToolCall};
    use serde_json::json;

    fn tool_call_response(name: &str, args: serde_json::Value) -> ChatResponse {
        ChatResponse {
            message: ChatMessage {
                role: Role::Assistant,
                content: String::new(),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: name.into(),
                    arguments: args,
                }],
                tool_call_id: None,
            },
        }
    }

    #[tokio::test]
    async fn calls_tool_then_returns_answer() {
        let model = ScriptedModel::new(vec![
            tool_call_response("web_search", json!({"query":"rust"})),
            ChatResponse {
                message: ChatMessage::assistant("final answer"),
            },
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_search".into(),
            Arc::new(EchoTool::new("web_search", "search results")),
        );

        let out = run_loop(
            &model,
            &tools,
            vec![ChatMessage::user("hi")],
            &LoopConfig {
                model: "m".into(),
                max_iterations: 5,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content, "final answer");
        assert!(out.evidence.contains("search results")); // tool output captured as evidence
    }

    #[tokio::test]
    async fn retries_transient_transport_errors() {
        use construct_core::model::{ChatRequest, ModelError};
        use std::sync::Mutex;
        // Model that fails with Transport twice, then succeeds.
        struct Flaky {
            remaining_failures: Mutex<u32>,
        }
        #[async_trait::async_trait]
        impl ModelProvider for Flaky {
            async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ModelError> {
                let mut n = self.remaining_failures.lock().unwrap();
                if *n > 0 {
                    *n -= 1;
                    return Err(ModelError::Transport("connection reset".into()));
                }
                Ok(ChatResponse {
                    message: ChatMessage::assistant(r#"{"ok":true}"#),
                })
            }
        }
        let model = Flaky {
            remaining_failures: Mutex::new(2),
        };
        let tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        let out = run_loop(
            &model,
            &tools,
            vec![ChatMessage::user("hi")],
            &LoopConfig {
                model: "m".into(),
                max_iterations: 3,
            },
        )
        .await
        .unwrap();
        assert_eq!(out.content, r#"{"ok":true}"#);
    }

    #[tokio::test]
    async fn enforces_iteration_budget() {
        // Model always asks for a tool → never terminates → budget hit.
        let model = ScriptedModel::new(vec![
            tool_call_response("web_search", json!({"query":"a"})),
            tool_call_response("web_search", json!({"query":"b"})),
        ]);
        let mut tools: HashMap<String, Arc<dyn Tool>> = HashMap::new();
        tools.insert(
            "web_search".into(),
            Arc::new(EchoTool::new("web_search", "r")),
        );
        let err = run_loop(
            &model,
            &tools,
            vec![ChatMessage::user("hi")],
            &LoopConfig {
                model: "m".into(),
                max_iterations: 2,
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, LoopError::Budget(2)));
    }
}
