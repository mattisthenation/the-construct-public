//! OpenAI-compatible provider. The request/response wire format is identical to
//! the one `OllamaProvider` already implements, so we reuse its pure codec and
//! only add the `Authorization: Bearer <key>` header and a configurable base URL.

use async_trait::async_trait;
use construct_core::model::{ChatRequest, ChatResponse, ModelError, ModelProvider};
use construct_model_ollama::OllamaProvider;
use serde_json::Value;

pub struct OpenAiProvider {
    base_url: String,
    api_key: String,
    http: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        OpenAiProvider {
            base_url: base_url.into(),
            api_key: api_key.into(),
            http,
        }
    }
}

#[async_trait]
impl ModelProvider for OpenAiProvider {
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, ModelError> {
        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&OllamaProvider::build_body(&req)) // identical OpenAI wire format
            .send()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        if !resp.status().is_success() {
            let code = resp.status();
            // Body may carry an error message, but never echo the key; reqwest does
            // not include headers in the error path, so this is safe to surface.
            let body = resp.text().await.unwrap_or_default();
            return Err(ModelError::Transport(format!("HTTP {code}: {body}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| ModelError::Transport(e.to_string()))?;
        OllamaProvider::parse_response(&v)
    }
}
