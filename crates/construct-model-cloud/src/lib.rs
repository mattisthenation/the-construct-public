//! Cloud / escalation-tier model providers. Ollama is the on-thesis default
//! (see `construct-model-ollama`); these are the opt-in cloud escalation paths.
//!
//! - `OpenAiProvider`: any OpenAI-compatible `/v1/chat/completions` endpoint
//!   (OpenAI, Groq, Together, OpenRouter, vLLM, …). Reuses the Ollama request/
//!   response codec — the wire format is identical — and adds a bearer token.
//! - `AnthropicProvider`: Anthropic's `/v1/messages` API (different shape).
//!
//! API keys are passed in by the caller (resolved from an env var named in
//! config); they are never logged and never written to disk by these types.
pub mod anthropic;
pub mod openai;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;

/// Default base URLs, used when an agent config omits `base_url` for a cloud provider.
pub const OPENAI_DEFAULT_BASE: &str = "https://api.openai.com";
pub const ANTHROPIC_DEFAULT_BASE: &str = "https://api.anthropic.com";
