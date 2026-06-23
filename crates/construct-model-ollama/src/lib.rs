use async_trait::async_trait;
use construct_core::model::{
    ChatMessage, ChatRequest, ChatResponse, ModelError, ModelProvider, Role, ToolCall,
};
use serde_json::{json, Value};

pub struct OllamaProvider {
    base_url: String,
    http: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>) -> Self {
        // Local models can be slow (cold load, large models), so the read timeout is
        // generous — but finite, so a hung/black-holed Ollama can't wedge a run forever
        // (which would hold the per-note lock indefinitely).
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(300))
            .build()
            .unwrap_or_default();
        OllamaProvider {
            base_url: base_url.into(),
            http,
        }
    }

    fn role_str(r: &Role) -> &'static str {
        match r {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        }
    }

    /// Build the OpenAI-compatible request body. Pure function → unit-testable.
    pub fn build_body(req: &ChatRequest) -> Value {
        let messages: Vec<Value> =
            req.messages
                .iter()
                .map(|m| {
                    let mut o = json!({ "role": Self::role_str(&m.role), "content": m.content });
                    if let Some(id) = &m.tool_call_id {
                        o["tool_call_id"] = json!(id);
                    }
                    if !m.tool_calls.is_empty() {
                        o["tool_calls"] = json!(m.tool_calls.iter().map(|tc| json!({
                    "id": tc.id,
                    "type": "function",
                    "function": { "name": tc.name, "arguments": tc.arguments.to_string() }
                })).collect::<Vec<_>>());
                    }
                    o
                })
                .collect();

        let mut body = json!({ "model": req.model, "messages": messages, "stream": false });
        if !req.tools.is_empty() {
            body["tools"] = json!(req.tools.iter().map(|t| json!({
                "type": "function",
                "function": { "name": t.name, "description": t.description, "parameters": t.parameters }
            })).collect::<Vec<_>>());
        }
        body
    }

    /// Parse one choice from an OpenAI-compatible response. Pure → unit-testable.
    pub fn parse_response(v: &Value) -> Result<ChatResponse, ModelError> {
        let msg = v["choices"]
            .get(0)
            .and_then(|c| c.get("message"))
            .ok_or_else(|| ModelError::Invalid("no choices[0].message".into()))?;
        let content = msg["content"].as_str().unwrap_or("").to_string();
        let mut tool_calls = vec![];
        if let Some(arr) = msg["tool_calls"].as_array() {
            for tc in arr {
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let arguments: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                tool_calls.push(ToolCall {
                    id: tc["id"].as_str().unwrap_or_default().to_string(),
                    name,
                    arguments,
                });
            }
        }
        Ok(ChatResponse {
            message: ChatMessage {
                role: Role::Assistant,
                content,
                tool_calls,
                tool_call_id: None,
            },
        })
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .post(url)
            .json(&Self::build_body(&req))
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        Self::parse_response(&v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::tool::ToolSpec;

    #[test]
    fn body_includes_tools_and_messages() {
        let req = ChatRequest {
            model: "qwen2.5:14b".into(),
            messages: vec![ChatMessage::system("s"), ChatMessage::user("q")],
            tools: vec![ToolSpec {
                name: "web_search".into(),
                description: "d".into(),
                parameters: json!({"type":"object"}),
            }],
        };
        let body = OllamaProvider::build_body(&req);
        assert_eq!(body["model"], "qwen2.5:14b");
        assert_eq!(body["messages"].as_array().unwrap().len(), 2);
        assert_eq!(body["tools"][0]["function"]["name"], "web_search");
    }

    #[test]
    fn parses_tool_call_response() {
        let v = json!({"choices":[{"message":{"content":"","tool_calls":[
            {"id":"call_1","function":{"name":"web_search","arguments":"{\"query\":\"rust\"}"}}
        ]}}]});
        let r = OllamaProvider::parse_response(&v).unwrap();
        assert_eq!(r.message.tool_calls.len(), 1);
        assert_eq!(r.message.tool_calls[0].name, "web_search");
        assert_eq!(r.message.tool_calls[0].arguments["query"], "rust");
    }

    #[test]
    fn parses_plain_text_response() {
        let v = json!({"choices":[{"message":{"content":"hello"}}]});
        let r = OllamaProvider::parse_response(&v).unwrap();
        assert_eq!(r.message.content, "hello");
        assert!(r.message.tool_calls.is_empty());
    }
}
