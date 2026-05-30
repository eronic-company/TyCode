pub mod anthropic;
pub mod dev;
pub mod gemini;
pub mod ollama;
pub mod openai;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::tools::ToolSchema;

// ── Message types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Tool calls returned by the assistant (only on assistant messages).
    pub tool_calls: Vec<ToolCall>,
    /// For tool-result messages, the id of the tool call this answers.
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: Role::User, content: content.into(), tool_calls: vec![], tool_call_id: None }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_calls: vec![], tool_call_id: None }
    }
    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self { role: Role::Assistant, content: content.into(), tool_calls, tool_call_id: None }
    }
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: Role::System, content: content.into(), tool_calls: vec![], tool_call_id: None }
    }
    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            tool_calls: vec![],
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Error,
}

#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}

#[derive(Debug, Clone)]
pub struct ProviderResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
}

/// Events streamed from the provider to the agent during generation.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    TextDelta(String),
    Done,
}

// ── Provider trait ───────────────────────────────────────────────────────────

#[async_trait]
pub trait Provider: Send + Sync {
    /// Send a chat request with optional streaming of text deltas.
    /// If `delta_tx` is provided, text tokens are streamed through it.
    /// Always returns the complete accumulated response.
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse>;

    /// List available models for this provider.
    async fn available_models(&self) -> Vec<String>;

    /// Provider display name.
    fn name(&self) -> &str;
}

// ── Readiness ────────────────────────────────────────────────────────────────

pub async fn ensure_provider_ready(config: &Config) -> Result<()> {
    if config.provider.to_lowercase() == "ollama" {
        let provider = ollama::OllamaProvider::new(&config.model, &config.ollama_url);

        if !provider.is_reachable().await {
            if provider.can_start_local_service() {
                provider.try_start_local_service()?;
            }

            if !provider
                .wait_until_reachable(
                    std::time::Duration::from_secs(20),
                    std::time::Duration::from_millis(500),
                )
                .await
            {
                anyhow::bail!(
                    "Ollama is not running at {} and could not be started automatically.",
                    config.ollama_url
                );
            }
        }

        // Check if model exists
        let models = provider.available_models().await;
        if !models.iter().any(|m| m == &config.model || m.starts_with(&format!("{}:", config.model))) {
            // Pull model
            provider.pull_model(&config.model).await?;
        }

        provider.warm_model(&config.model).await?;
    }
    Ok(())
}

// ── Factory ──────────────────────────────────────────────────────────────────

pub fn create_provider(config: &Config) -> Result<Box<dyn Provider>> {
    match config.provider.to_lowercase().as_str() {
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider::new(
            &config.model,
            &config.anthropic_api_key,
            config.max_tokens,
        ))),
        "dev" => Ok(Box::new(dev::DevProvider)),
        "openai" => Ok(Box::new(openai::OpenAIProvider::new(
            &config.model,
            &config.openai_api_key,
            &config.openai_base_url,
            config.max_tokens,
        ))),
        "ollama" => Ok(Box::new(ollama::OllamaProvider::new(
            &config.model,
            &config.ollama_url,
        ))),
        "gemini" => Ok(Box::new(gemini::GeminiProvider::new(
            &config.model,
            &config.google_api_key,
            config.max_tokens,
        ))),
        other => anyhow::bail!("Unknown provider '{other}'. Options: anthropic, dev, openai, ollama, gemini"),
    }
}
