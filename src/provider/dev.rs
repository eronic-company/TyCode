use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use tokio::sync::mpsc;

use crate::provider::{Message, Provider, StopReason, StreamEvent, ToolCall, TokenUsage};
use crate::tools::ToolSchema;

use super::ProviderResponse;

pub struct DevProvider;

impl DevProvider {
    /// Parse dev provider commands from message content.
    /// Format: "run <tool>: <input>" or "execute <tool>: <input>"
    fn parse_tool_call(content: &str) -> Option<ToolCall> {
        let prefixes = vec!["run ", "execute ", "call "];

        for prefix in prefixes {
            if let Some(rest) = content.trim().strip_prefix(prefix) {
                if let Some(colon_pos) = rest.find(':') {
                    let tool_name = rest[..colon_pos].trim().to_string();
                    let tool_input = rest[colon_pos + 1..].trim().to_string();

                    // Create a tool call with the input
                    return Some(ToolCall {
                        id: format!("dev_{}", uuid::Uuid::new_v4()),
                        name: tool_name,
                        input: json!({ "command": tool_input }),
                    });
                }
            }
        }
        None
    }
}

#[async_trait]
impl Provider for DevProvider {
    async fn chat(
        &self,
        messages: &[Message],
        _tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse> {
        // Get the last user message to check for tool calls
        let last_user_message = messages
            .iter()
            .rev()
            .find(|m| matches!(m.role, crate::provider::Role::User))
            .map(|m| m.content.as_str());

        // Try to parse a tool call from the message
        let (response_text, tool_calls) = if let Some(msg) = last_user_message {
            if let Some(tool_call) = Self::parse_tool_call(msg) {
                // Return empty text and the tool call
                ("".to_string(), vec![tool_call])
            } else {
                // Regular dev response
                (
                    "This is a test response to allow developers to test functionality without acting on actions.".to_string(),
                    vec![],
                )
            }
        } else {
            (
                "This is a test response to allow developers to test functionality without acting on actions.".to_string(),
                vec![],
            )
        };

        // Stream the response if delta_tx is provided and we're not calling tools
        if let Some(tx) = delta_tx {
            if !response_text.is_empty() {
                let _ = tx.send(StreamEvent::TextDelta(response_text.clone()));
            }
            let _ = tx.send(StreamEvent::Done);
        }

        // Calculate token usage
        let input_tokens = messages.iter().map(|m| m.content.len() / 4).sum::<usize>() as u32;
        let output_tokens = (response_text.len() / 4).max(1) as u32;

        Ok(ProviderResponse {
            text: response_text,
            tool_calls,
            stop_reason: StopReason::EndTurn,
            usage: TokenUsage { input: input_tokens, output: output_tokens },
        })
    }

    async fn available_models(&self) -> Vec<String> {
        vec!["dev-v4-pro".to_string()]
    }

    fn name(&self) -> &str {
        "dev"
    }
}
