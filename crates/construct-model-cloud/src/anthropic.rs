//! Anthropic Messages API provider (`POST /v1/messages`). Its wire format differs
//! from OpenAI: the system prompt is a top-level field, messages carry typed
//! content blocks, and tool use/results are `tool_use`/`tool_result` blocks. The
//! body builder and response parser are pure functions so they're unit-tested
//! without a network.

use async_trait::async_trait;
use construct_core::model::{
    ChatMessage, ChatRequest, ChatResponse, ModelError, ModelProvider, Role, ToolCall,
};
use serde_json::{json, Value};

const API_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    base_url: String,
    api_key: String,
    max_tokens: u32,
    http: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        AnthropicProvider {
            base_url: base_url.into(),
            api_key: api_key.into(),
            max_tokens: DEFAULT_MAX_TOKENS,
            http,
        }
    }

    /// Build the `/v1/messages` request body. Pure → unit-testable.
    ///
    /// System messages are hoisted to the top-level `system` field. Assistant
    /// tool calls become `tool_use` blocks; `Role::Tool` results become
    /// `tool_result` blocks in a user message, merged with any adjacent results
    /// (Anthropic wants all results for a turn in one user message).
    pub fn build_body(req: &ChatRequest, max_tokens: u32) -> Value {
        let mut system = String::new();
        let mut messages: Vec<Value> = Vec::new();

        for m in &req.messages {
            match m.role {
                Role::System => {
                    if !system.is_empty() {
                        system.push('\n');
                    }
                    system.push_str(&m.content);
                }
                Role::User => messages.push(json!({
                    "role": "user",
                    "content": [{ "type": "text", "text": m.content }],
                })),
                Role::Assistant => {
                    let mut content: Vec<Value> = Vec::new();
                    if !m.content.trim().is_empty() {
                        content.push(json!({ "type": "text", "text": m.content }));
                    }
                    for tc in &m.tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.arguments,
                        }));
                    }
                    messages.push(json!({ "role": "assistant", "content": content }));
                }
                Role::Tool => {
                    let block = json!({
                        "type": "tool_result",
                        "tool_use_id": m.tool_call_id.clone().unwrap_or_default(),
                        "content": m.content,
                    });
                    // Merge into the previous user message if it's already a
                    // tool_result carrier (consecutive results in one turn).
                    if let Some(last) = messages.last_mut() {
                        if last["role"] == "user" && last["content"][0]["type"] == "tool_result" {
                            last["content"].as_array_mut().unwrap().push(block);
                            continue;
                        }
                    }
                    messages.push(json!({ "role": "user", "content": [block] }));
                }
            }
        }

        let mut body = json!({
            "model": req.model,
            "max_tokens": max_tokens,
            "messages": messages,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if !req.tools.is_empty() {
            body["tools"] = json!(req
                .tools
                .iter()
                .map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                }))
                .collect::<Vec<_>>());
        }
        body
    }

    /// Parse a `/v1/messages` response into our `ChatResponse`. Pure → testable.
    pub fn parse_response(v: &Value) -> Result<ChatResponse, ModelError> {
        // Surface API-level errors (e.g. {"type":"error","error":{"message":...}}).
        if v["type"] == "error" {
            let msg = v["error"]["message"].as_str().unwrap_or("unknown error");
            return Err(ModelError::Invalid(format!("anthropic: {msg}")));
        }
        let blocks = v["content"]
            .as_array()
            .ok_or_else(|| ModelError::Invalid("no content array".into()))?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for b in blocks {
            match b["type"].as_str() {
                Some("text") => text.push_str(b["text"].as_str().unwrap_or("")),
                Some("tool_use") => tool_calls.push(ToolCall {
                    id: b["id"].as_str().unwrap_or_default().to_string(),
                    name: b["name"].as_str().unwrap_or_default().to_string(),
                    arguments: b["input"].clone(),
                }),
                _ => {}
            }
        }
        Ok(ChatResponse {
            message: ChatMessage {
                role: Role::Assistant,
                content: text,
                tool_calls,
                tool_call_id: None,
            },
        })
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError> {
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .json(&Self::build_body(&req, self.max_tokens))
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("request failed");
            return Err(ModelError::Transport(format!("HTTP {status}: {msg}")));
        }
        Self::parse_response(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::tool::ToolSpec;

    #[test]
    fn hoists_system_and_builds_messages() {
        let req = ChatRequest {
            model: "claude-sonnet-4-6".into(),
            messages: vec![ChatMessage::system("be terse"), ChatMessage::user("hi")],
            tools: vec![],
        };
        let body = AnthropicProvider::build_body(&req, 1024);
        assert_eq!(body["system"], "be terse");
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"][0]["text"], "hi");
    }

    #[test]
    fn assistant_tool_use_and_merged_results() {
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![
                ChatMessage {
                    role: Role::Assistant,
                    content: String::new(),
                    tool_calls: vec![
                        ToolCall {
                            id: "a".into(),
                            name: "web_search".into(),
                            arguments: json!({"q": "x"}),
                        },
                        ToolCall {
                            id: "b".into(),
                            name: "web_fetch".into(),
                            arguments: json!({"url": "u"}),
                        },
                    ],
                    tool_call_id: None,
                },
                ChatMessage::tool_result("a", "res-a"),
                ChatMessage::tool_result("b", "res-b"),
            ],
            tools: vec![ToolSpec {
                name: "web_search".into(),
                description: "d".into(),
                parameters: json!({"type": "object"}),
            }],
        };
        let body = AnthropicProvider::build_body(&req, 4096);
        // assistant turn has two tool_use blocks
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][0]["content"][0]["type"], "tool_use");
        // the two tool results merged into ONE user message
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"].as_array().unwrap().len(), 2);
        assert_eq!(body["messages"][1]["content"][0]["tool_use_id"], "a");
        // tool schema mapped to input_schema
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn parses_text_and_tool_use() {
        let v = json!({
            "content": [
                {"type": "text", "text": "thinking"},
                {"type": "tool_use", "id": "c1", "name": "web_search", "input": {"query": "rust"}}
            ]
        });
        let r = AnthropicProvider::parse_response(&v).unwrap();
        assert_eq!(r.message.content, "thinking");
        assert_eq!(r.message.tool_calls.len(), 1);
        assert_eq!(r.message.tool_calls[0].name, "web_search");
        assert_eq!(r.message.tool_calls[0].arguments["query"], "rust");
    }

    #[test]
    fn surfaces_api_error() {
        let v = json!({"type": "error", "error": {"message": "overloaded"}});
        assert!(AnthropicProvider::parse_response(&v).is_err());
    }
}
