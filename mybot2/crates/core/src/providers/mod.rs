//! Model providers behind one trait.

mod anthropic;
mod gemini;
mod http;
mod openai;

pub use anthropic::{Anthropic, INVALID_JSON_KEY};
pub use gemini::Gemini;
pub use openai::OpenAi;

use async_trait::async_trait;

use crate::model::{ChatOptions, Completion, Message, StreamEvent};

pub const PROVIDERS: [&str; 3] = ["anthropic", "openai", "gemini"];

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("{provider} {status}: {message}")]
    Http { provider: &'static str, status: u16, message: String },
    #[error("{0}")]
    Network(String),
    #[error("{0}")]
    Protocol(String),
    #[error("No API key for {provider}. Set {env_var} or add one in Settings → Keys.")]
    MissingKey { provider: String, env_var: String },
    #[error("Unknown provider \"{0}\"")]
    Unknown(String),
}

pub type EventSink<'a> = &'a (dyn Fn(StreamEvent) + Send + Sync);

#[async_trait]
pub trait Provider: Send + Sync {
    fn name(&self) -> &'static str;
    fn default_model(&self) -> &'static str;

    /// One assistant turn, streamed. Events arrive on `on_event` as they come;
    /// the returned completion is the turn to append to history.
    async fn chat(
        &self,
        messages: &[Message],
        opts: &ChatOptions,
        on_event: EventSink<'_>,
    ) -> Result<Completion, ProviderError>;

    /// Live model ids — model ids churn faster than any hardcoded table.
    async fn list_models(&self) -> Result<Vec<String>, ProviderError>;
}

/// Where a provider lives and how it authenticates.
#[derive(Debug, Clone, Default)]
pub struct ProviderConfig {
    pub api_key: String,
    /// Tests, self-hosted gateways, and the ChatGPT-account proxy.
    pub base_url: Option<String>,
}

pub fn make(name: &str, cfg: ProviderConfig) -> Result<Box<dyn Provider>, ProviderError> {
    Ok(match name {
        "anthropic" => Box::new(Anthropic::new(cfg)),
        "openai" => Box::new(OpenAi::new(cfg)),
        "gemini" => Box::new(Gemini::new(cfg)),
        other => return Err(ProviderError::Unknown(other.into())),
    })
}

pub fn default_model(name: &str) -> &'static str {
    match name {
        "anthropic" => anthropic::DEFAULT_MODEL,
        "openai" => openai::DEFAULT_MODEL,
        "gemini" => gemini::DEFAULT_MODEL,
        _ => "",
    }
}

/// Short display label: "Claude", "GPT", "Gemini".
pub fn family_label(name: &str) -> &'static str {
    match name {
        "anthropic" => "Claude",
        "openai" => "GPT",
        "gemini" => "Gemini",
        _ => "Model",
    }
}
