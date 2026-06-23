use construct_core::model::{ChatMessage, ChatRequest, ModelProvider};
use std::sync::Arc;

/// Holds the chat transcript and the in-progress input line.
pub struct ChatState {
    pub model: String,
    pub history: Vec<ChatMessage>,
    pub input: String,
    /// True while a model request is in flight (drives the "thinking…" hint).
    pub thinking: bool,
}

impl ChatState {
    pub fn new(model: String, system: &str) -> Self {
        ChatState {
            model,
            history: vec![ChatMessage::system(system)],
            input: String::new(),
            thinking: false,
        }
    }

    pub fn push_char(&mut self, c: char) {
        self.input.push(c);
    }
    pub fn backspace(&mut self) {
        self.input.pop();
    }

    /// Take the current input as a user message, clearing the buffer.
    pub fn take_input(&mut self) -> Option<String> {
        let t = self.input.trim().to_string();
        self.input.clear();
        if t.is_empty() {
            None
        } else {
            self.history.push(ChatMessage::user(&t));
            Some(t)
        }
    }

    /// Send the conversation to the model and append the reply. On failure the
    /// error is surfaced as an assistant message in the transcript (so the user
    /// sees what went wrong instead of silence) rather than being swallowed.
    pub async fn send(&mut self, provider: Arc<dyn ModelProvider>) {
        let req = ChatRequest {
            model: self.model.clone(),
            messages: self.history.clone(),
            tools: vec![],
        };
        match provider.chat(req).await {
            Ok(resp) => self.history.push(resp.message),
            Err(e) => self
                .history
                .push(ChatMessage::assistant(format!("⚠ chat error: {e}"))),
        }
    }

    /// Visible (non-system) lines for rendering.
    pub fn visible(&self) -> Vec<&ChatMessage> {
        self.history
            .iter()
            .filter(|m| !matches!(m.role, construct_core::model::Role::System))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_editing_and_take() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('h');
        s.push_char('i');
        s.push_char('x');
        s.backspace();
        assert_eq!(s.input, "hi");
        assert_eq!(s.take_input().as_deref(), Some("hi"));
        assert!(s.input.is_empty());
        // empty input is ignored
        assert!(s.take_input().is_none());
    }

    #[test]
    fn visible_excludes_system() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('q');
        let _ = s.take_input();
        assert_eq!(s.visible().len(), 1); // just the user message
    }

    use construct_core::model::{ChatRequest, ChatResponse, ModelError};

    struct FailingProvider;

    #[async_trait::async_trait]
    impl ModelProvider for FailingProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ModelError> {
            Err(ModelError::Transport("connection refused".into()))
        }
    }

    struct EchoProvider;

    #[async_trait::async_trait]
    impl ModelProvider for EchoProvider {
        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, ModelError> {
            Ok(ChatResponse {
                message: ChatMessage::assistant("pong"),
            })
        }
    }

    #[tokio::test]
    async fn send_surfaces_error_into_transcript() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('h');
        s.push_char('i');
        let _ = s.take_input();
        s.send(Arc::new(FailingProvider)).await;
        // The error is visible to the user as an assistant line, not swallowed.
        let visible = s.visible();
        let last = visible.last().unwrap();
        assert_eq!(last.role, construct_core::model::Role::Assistant);
        assert!(last.content.contains("chat error"));
        assert!(last.content.contains("connection refused"));
    }

    #[tokio::test]
    async fn send_appends_reply_on_success() {
        let mut s = ChatState::new("m".into(), "sys");
        s.push_char('h');
        let _ = s.take_input();
        s.send(Arc::new(EchoProvider)).await;
        assert_eq!(s.visible().last().unwrap().content, "pong");
    }
}
