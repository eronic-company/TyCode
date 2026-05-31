use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{Message, Provider, ProviderResponse, Role, StopReason, StreamEvent, TokenUsage, ToolCall};
use crate::tools::ToolSchema;

/// OpenAI-compatible provider. Works with OpenAI, Groq, LM Studio, Together,
/// OpenRouter, Mistral, and any other OpenAI-compatible endpoint.
pub struct OpenAIProvider {
    model: String,
    api_key: String,
    base_url: String,
    max_tokens: u32,
    client: Client,
}

impl OpenAIProvider {
    pub fn new(model: &str, api_key: &str, base_url: &str, max_tokens: u32) -> Self {
        let key = if api_key.is_empty() {
            std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| "not-set".into())
        } else {
            api_key.to_string()
        };
        let url = if base_url.is_empty() {
            "https://api.openai.com/v1".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        Self {
            model: model.to_string(),
            api_key: key,
            base_url: url,
            max_tokens,
            client: Client::new(),
        }
    }

    fn build_messages(&self, messages: &[Message]) -> Vec<Value> {
        let mut msgs = Vec::new();
        for m in messages {
            match m.role {
                Role::System => {
                    msgs.push(json!({"role": "system", "content": m.content}));
                }
                Role::User => {
                    msgs.push(json!({"role": "user", "content": m.content}));
                }
                Role::Assistant => {
                    let mut msg = json!({"role": "assistant", "content": m.content});
                    if !m.tool_calls.is_empty() {
                        let tcs: Vec<Value> = m
                            .tool_calls
                            .iter()
                            .map(|tc| {
                                json!({
                                    "id": tc.id,
                                    "type": "function",
                                    "function": {
                                        "name": tc.name,
                                        "arguments": serde_json::to_string(&tc.input).unwrap_or_default(),
                                    }
                                })
                            })
                            .collect();
                        msg["tool_calls"] = json!(tcs);
                    }
                    msgs.push(msg);
                }
                Role::Tool => {
                    msgs.push(json!({
                        "role": "tool",
                        "tool_call_id": m.tool_call_id.as_deref().unwrap_or(""),
                        "content": m.content,
                    }));
                }
            }
        }
        msgs
    }

    fn build_tools(&self, tools: &[ToolSchema]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect()
    }
}

#[async_trait]
impl Provider for OpenAIProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse> {
        let msgs = self.build_messages(messages);
        let api_tools = self.build_tools(tools);
        let use_stream = delta_tx.is_some();

        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "max_tokens": self.max_tokens,
            "stream": use_stream,
        });
        if !api_tools.is_empty() {
            body["tools"] = json!(api_tools);
            body["tool_choice"] = json!("auto");
        }

        let url = format!("{}/chat/completions", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .body(serde_json::to_string(&body)?)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("OpenAI API {status}: {text}");
        }

        if use_stream {
            self.handle_stream(resp, delta_tx.unwrap()).await
        } else {
            self.handle_response(resp).await
        }
    }

    async fn available_models(&self) -> Vec<String> {
        let url = format!("{}/models", self.base_url);
        match self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
        {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(arr) = data["data"].as_array() {
                        let mut models: Vec<String> = arr
                            .iter()
                            .filter_map(|m| m["id"].as_str().map(String::from))
                            .collect();
                        models.sort();
                        return models;
                    }
                }
                self.default_models()
            }
            Err(_) => self.default_models(),
        }
    }

    fn name(&self) -> &str {
        "openai"
    }

    fn default_max_tokens(&self) -> u32 { 4096 }
    fn default_max_iterations(&self) -> u32 { 100 }
}

impl OpenAIProvider {
    fn default_models(&self) -> Vec<String> {
        vec![
            "gpt-4o".into(),
            "gpt-4o-mini".into(),
            "gpt-4-turbo".into(),
            "gpt-3.5-turbo".into(),
        ]
    }

    async fn handle_response(&self, resp: reqwest::Response) -> Result<ProviderResponse> {
        let data: Value = resp.json().await?;
        let choice = &data["choices"][0];
        let msg = &choice["message"];

        let text = msg["content"].as_str().unwrap_or("").to_string();
        let mut tool_calls = Vec::new();

        if let Some(tcs) = msg["tool_calls"].as_array() {
            for tc in tcs {
                let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
                let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));
                tool_calls.push(ToolCall {
                    id: tc["id"].as_str().unwrap_or("").to_string(),
                    name: tc["function"]["name"].as_str().unwrap_or("").to_string(),
                    input,
                });
            }
        }

        let stop_reason = match choice["finish_reason"].as_str() {
            Some("tool_calls") => StopReason::ToolUse,
            Some("length") => StopReason::MaxTokens,
            _ => StopReason::EndTurn,
        };

        let usage = TokenUsage {
            input: data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            output: data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
    }

    async fn handle_stream(
        &self,
        resp: reqwest::Response,
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<ProviderResponse> {
        let mut text = String::new();
        let mut tool_calls_map: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new(); // index -> (id, name, arguments)
        let mut stop_reason = StopReason::EndTurn;
        let mut buffer = String::new();
        let mut usage_input: u32 = 0;
        let mut usage_output: u32 = 0;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            // Only process complete lines; retain any trailing partial line so a
            // record split across chunk boundaries isn't parsed twice or dropped.
            let split_at = match buffer.rfind('\n') {
                Some(p) => p + 1,
                None => continue,
            };
            let complete: String = buffer.drain(..split_at).collect();

            for line in complete.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                if let Some(data_str) = line.strip_prefix("data: ") {
                    if data_str.trim() == "[DONE]" {
                        continue;
                    }
                    let data: Value = match serde_json::from_str(data_str) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    let choice = &data["choices"][0];
                    let delta = &choice["delta"];

                    // Text content
                    if let Some(content) = delta["content"].as_str() {
                        text.push_str(content);
                        let _ = tx.send(StreamEvent::TextDelta(content.to_string()));
                    }

                    // Tool calls
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                            let entry = tool_calls_map.entry(idx).or_insert_with(|| {
                                (String::new(), String::new(), String::new())
                            });
                            if let Some(id) = tc["id"].as_str() {
                                entry.0 = id.to_string();
                            }
                            if let Some(name) = tc["function"]["name"].as_str() {
                                entry.1.push_str(name);
                            }
                            if let Some(args) = tc["function"]["arguments"].as_str() {
                                entry.2.push_str(args);
                            }
                        }
                    }

                    // Stop reason
                    if let Some(fr) = choice["finish_reason"].as_str() {
                        stop_reason = match fr {
                            "tool_calls" => StopReason::ToolUse,
                            "length" => StopReason::MaxTokens,
                            _ => StopReason::EndTurn,
                        };
                    }

                    // Usage (present in final chunk when stream_options.include_usage is set)
                    if !data["usage"].is_null() {
                        usage_input = data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32;
                        usage_output = data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32;
                    }
                }
            }
        }

        let mut tool_calls: Vec<ToolCall> = tool_calls_map
            .into_iter()
            .map(|(_, (id, name, args))| {
                let input: Value = serde_json::from_str(&args).unwrap_or(json!({}));
                ToolCall { id, name, input }
            })
            .collect();
        tool_calls.sort_by(|a, b| a.name.cmp(&b.name));

        if !tool_calls.is_empty() && matches!(stop_reason, StopReason::EndTurn) {
            stop_reason = StopReason::ToolUse;
        }

        let _ = tx.send(StreamEvent::Done);
        Ok(ProviderResponse { text, tool_calls, stop_reason, usage: TokenUsage { input: usage_input, output: usage_output } })
    }
}
