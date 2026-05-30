use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{Message, Provider, ProviderResponse, Role, StopReason, StreamEvent, TokenUsage, ToolCall};
use crate::tools::ToolSchema;

pub struct AnthropicProvider {
    model: String,
    api_key: String,
    max_tokens: u32,
    client: Client,
}

impl AnthropicProvider {
    pub fn new(model: &str, api_key: &str, max_tokens: u32) -> Self {
        let key = if api_key.is_empty() {
            std::env::var("ANTHROPIC_API_KEY").unwrap_or_default()
        } else {
            api_key.to_string()
        };
        Self {
            model: model.to_string(),
            api_key: key,
            max_tokens,
            client: Client::new(),
        }
    }

    fn build_messages(&self, messages: &[Message]) -> (Option<String>, Vec<Value>) {
        let mut system = None;
        let mut msgs = Vec::new();

        for m in messages {
            match m.role {
                Role::System => {
                    system = Some(m.content.clone());
                }
                Role::User => {
                    msgs.push(json!({
                        "role": "user",
                        "content": m.content,
                    }));
                }
                Role::Assistant => {
                    let mut content: Vec<Value> = vec![];
                    if !m.content.is_empty() {
                        content.push(json!({"type": "text", "text": m.content}));
                    }
                    for tc in &m.tool_calls {
                        content.push(json!({
                            "type": "tool_use",
                            "id": tc.id,
                            "name": tc.name,
                            "input": tc.input,
                        }));
                    }
                    msgs.push(json!({
                        "role": "assistant",
                        "content": content,
                    }));
                }
                Role::Tool => {
                    msgs.push(json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
                            "content": m.content,
                        }],
                    }));
                }
            }
        }
        (system, msgs)
    }

    fn build_tools(&self, tools: &[ToolSchema]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect()
    }
}

#[async_trait]
impl Provider for AnthropicProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse> {
        let (system, msgs) = self.build_messages(messages);
        let api_tools = self.build_tools(tools);
        let use_stream = delta_tx.is_some();

        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": self.max_tokens,
            "stream": use_stream,
        });
        if let Some(sys) = &system {
            body["system"] = json!(sys);
        }
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
        }

        let resp = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .body(serde_json::to_string(&body)?)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Anthropic API {status}: {text}");
        }

        if use_stream {
            self.handle_stream(resp, delta_tx.unwrap()).await
        } else {
            self.handle_response(resp).await
        }
    }

    async fn available_models(&self) -> Vec<String> {
        vec![
            "claude-sonnet-4-6".into(),
            "claude-opus-4-6".into(),
            "claude-haiku-4-5-20251001".into(),
            "claude-3-5-sonnet-20241022".into(),
        ]
    }

    fn name(&self) -> &str {
        "anthropic"
    }
}

impl AnthropicProvider {
    async fn handle_response(&self, resp: reqwest::Response) -> Result<ProviderResponse> {
        let data: Value = resp.json().await?;
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        if let Some(content) = data["content"].as_array() {
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        text = block["text"].as_str().unwrap_or("").to_string();
                    }
                    Some("tool_use") => {
                        tool_calls.push(ToolCall {
                            id: block["id"].as_str().unwrap_or("").to_string(),
                            name: block["name"].as_str().unwrap_or("").to_string(),
                            input: block["input"].clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = match data["stop_reason"].as_str() {
            Some("tool_use") => StopReason::ToolUse,
            Some("max_tokens") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input: data["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
            output: data["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
    }

    async fn handle_stream(
        &self,
        resp: reqwest::Response,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<ProviderResponse> {
        let mut text = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut current_tool_input = String::new();
        let mut stop_reason = StopReason::EndTurn;
        let mut buffer = String::new();
        let mut usage_input: u32 = 0;
        let mut usage_output: u32 = 0;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => break,
            };
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            while let Some(pos) = buffer.find("\n\n") {
                let event_block = buffer[..pos].to_string();
                buffer.drain(..pos + 2);

                let mut event_type = String::new();
                let mut data_str = String::new();
                for line in event_block.lines() {
                    if let Some(rest) = line.strip_prefix("event: ") {
                        event_type = rest.trim().to_string();
                    } else if let Some(rest) = line.strip_prefix("data: ") {
                        data_str = rest.to_string();
                    }
                }

                if data_str.is_empty() || data_str == "[DONE]" {
                    continue;
                }

                let data: Value = match serde_json::from_str(&data_str) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                match event_type.as_str() {
                    "content_block_start" => {
                        if data["content_block"]["type"].as_str() == Some("tool_use") {
                            let tc = ToolCall {
                                id: data["content_block"]["id"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                                name: data["content_block"]["name"]
                                    .as_str()
                                    .unwrap_or("")
                                    .to_string(),
                                input: Value::Null,
                            };
                            tool_calls.push(tc);
                            current_tool_input.clear();
                        }
                    }
                    "content_block_delta" => {
                        if let Some(delta) = data["delta"].as_object() {
                            if let Some(t) = delta.get("text").and_then(|v| v.as_str()) {
                                text.push_str(t);
                                let _ = tx.send(StreamEvent::TextDelta(t.to_string()));
                            }
                            if let Some(json_delta) =
                                delta.get("partial_json").and_then(|v| v.as_str())
                            {
                                current_tool_input.push_str(json_delta);
                            }
                        }
                    }
                    "content_block_stop" => {
                        if !current_tool_input.is_empty() {
                            if let Some(tc) = tool_calls.last_mut() {
                                tc.input = serde_json::from_str(&current_tool_input)
                                    .unwrap_or(Value::Null);
                            }
                            current_tool_input.clear();
                        }
                    }
                    "message_start" => {
                        usage_input = data["message"]["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
                    }
                    "message_delta" => {
                        if let Some(sr) = data["delta"]["stop_reason"].as_str() {
                            stop_reason = match sr {
                                "tool_use" => StopReason::ToolUse,
                                "max_tokens" => StopReason::MaxTokens,
                                _ => StopReason::EndTurn,
                            };
                        }
                        usage_output += data["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
                    }
                    _ => {}
                }
            }
        }

        let _ = tx.send(StreamEvent::Done);
        Ok(ProviderResponse { text, tool_calls, stop_reason, usage: TokenUsage { input: usage_input, output: usage_output } })
    }
}
