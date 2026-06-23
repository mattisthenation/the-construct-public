use async_trait::async_trait;
use construct_core::model::{ChatRequest, ChatResponse, ModelError, ModelProvider};
use construct_core::tool::{Tool, ToolError, ToolSpec};
use serde_json::{json, Value};
use std::sync::Mutex;

/// A model that returns a scripted sequence of responses, one per `chat` call.
pub struct ScriptedModel {
    responses: Mutex<std::collections::VecDeque<ChatResponse>>,
}

impl ScriptedModel {
    pub fn new(responses: Vec<ChatResponse>) -> Self {
        ScriptedModel {
            responses: Mutex::new(responses.into_iter().collect()),
        }
    }
}

#[async_trait]
impl ModelProvider for ScriptedModel {
    async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ModelError> {
        self.responses
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| ModelError::Invalid("scripted model exhausted".into()))
    }
}

/// A tool that returns a fixed string and records its calls.
pub struct EchoTool {
    pub name: String,
    pub output: String,
    pub calls: Mutex<Vec<Value>>,
}

impl EchoTool {
    pub fn new(name: &str, output: &str) -> Self {
        EchoTool {
            name: name.into(),
            output: output.into(),
            calls: Mutex::new(vec![]),
        }
    }
}

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.name.clone(),
            description: "echo".into(),
            parameters: json!({"type":"object"}),
        }
    }
    async fn call(&self, args: Value) -> Result<String, ToolError> {
        self.calls.lock().unwrap().push(args);
        Ok(self.output.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use construct_core::model::ChatMessage;

    #[tokio::test]
    async fn scripted_model_pops_in_order() {
        let m = ScriptedModel::new(vec![ChatResponse {
            message: ChatMessage::assistant("hi"),
        }]);
        let req = ChatRequest {
            model: "m".into(),
            messages: vec![],
            tools: vec![],
        };
        assert_eq!(m.chat(req).await.unwrap().message.content, "hi");
    }

    #[tokio::test]
    async fn echo_tool_records_calls() {
        let t = EchoTool::new("web_search", "results");
        let out = t.call(json!({"query":"x"})).await.unwrap();
        assert_eq!(out, "results");
        assert_eq!(t.calls.lock().unwrap().len(), 1);
    }
}
