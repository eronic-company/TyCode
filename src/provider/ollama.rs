use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::sync::mpsc;

use super::{Message, Provider, ProviderResponse, Role, StopReason, StreamEvent, TokenUsage, ToolCall};
use crate::tools::ToolSchema;

/// Ollama provider — uses both the native /api/chat endpoint (for streaming)
/// and OpenAI-compatible /v1/chat/completions.
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: Client,
}

impl OllamaProvider {
    pub fn new(model: &str, base_url: &str) -> Self {
        Self {
            model: model.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                // Disable auto-decompression: Ollama streams NDJSON and reqwest's
                // gzip/brotli auto-decompress can corrupt the stream causing
                // "error decoding response body".
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .build()
                .unwrap_or_else(|_| Client::new()),
        }
    }

    fn build_messages(&self, messages: &[Message], tools: &[ToolSchema]) -> Vec<Value> {
        let mut msgs = Vec::new();

        // Build tool instruction text to append to system prompt
        let tool_text = if !tools.is_empty() {
            let tool_list: Vec<String> = tools
                .iter()
                .map(|t| {
                    format!(
                        "- {}: {} | params: {}",
                        t.name,
                        t.description,
                        serde_json::to_string(&t.parameters.get("properties").unwrap_or(&json!({})))
                            .unwrap_or_default()
                    )
                })
                .collect();
            format!(
                "\n\nYou have access to these tools:\n{}\n\n\
                 To call a tool, respond with EXACTLY this JSON format (no other text around it):\n\
                 ```json\n{{\"name\": \"tool_name\", \"input\": {{\"param\": \"value\"}}}}\n```\n\
                 You may call multiple tools by putting multiple JSON blocks.\n\
                 Only output a tool call block when you need to use a tool. Otherwise respond normally.",
                tool_list.join("\n")
            )
        } else {
            String::new()
        };

        for m in messages {
            match m.role {
                Role::System => {
                    let content = format!("{}{}", m.content, tool_text);
                    msgs.push(json!({"role": "system", "content": content}));
                }
                Role::User | Role::Tool => {
                    msgs.push(json!({"role": "user", "content": m.content}));
                }
                Role::Assistant => {
                    msgs.push(json!({"role": "assistant", "content": m.content}));
                }
            }
        }

        // If no system message was present, prepend one with tool instructions
        if !tool_text.is_empty() && !messages.iter().any(|m| matches!(m.role, Role::System)) {
            msgs.insert(0, json!({"role": "system", "content": tool_text.trim()}));
        }

        msgs
    }

    /// Parse tool calls from JSON code blocks in text.
    fn parse_tool_calls(text: &str) -> Vec<ToolCall> {
        let mut calls = Vec::new();
        let mut search_from = 0;

        while let Some(start_marker) = text[search_from..].find("```json") {
            let abs_start = search_from + start_marker + 7;
            if let Some(end_marker) = text[abs_start..].find("```") {
                let json_str = text[abs_start..abs_start + end_marker].trim();
                if let Ok(data) = serde_json::from_str::<Value>(json_str) {
                    Self::append_tool_calls_from_value(&mut calls, &data);
                }
                search_from = abs_start + end_marker + 3;
            } else {
                break;
            }
        }

        if calls.is_empty() {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
                for data in stream.flatten() {
                    Self::append_tool_calls_from_value(&mut calls, &data);
                }
            }
        }

        calls
    }

    fn append_tool_calls_from_value(calls: &mut Vec<ToolCall>, data: &Value) {
        if let Some(name) = data["name"].as_str() {
            let input = data
                .get("input")
                .or_else(|| data.get("parameters"))
                .cloned()
                .unwrap_or(json!({}));
            calls.push(ToolCall {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.to_string(),
                input,
            });
        } else if let Some(arr) = data.as_array() {
            for item in arr {
                Self::append_tool_calls_from_value(calls, item);
            }
        }
    }

    pub async fn is_reachable(&self) -> bool {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    pub async fn pull_model(&self, model: &str) -> Result<()> {
        let url = format!("{}/api/pull", self.base_url);
        let body = json!({ "name": model, "stream": false });
        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            anyhow::bail!("Failed to pull model: {}", resp.status());
        }
        Ok(())
    }

    pub fn can_start_local_service(&self) -> bool {
        self.base_url.starts_with("http://127.0.0.1:")
            || self.base_url.starts_with("http://localhost:")
    }

    pub fn try_start_local_service(&self) -> Result<()> {
        if !self.can_start_local_service() {
            anyhow::bail!("Configured Ollama URL is not local: {}", self.base_url);
        }

        #[cfg(target_os = "linux")]
        {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "start", "ollama"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();

            let _ = std::process::Command::new("systemctl")
                .args(["start", "ollama"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }

        let mut cmd = std::process::Command::new("ollama");
        cmd.arg("serve")
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if let Some(host) = self.base_url.strip_prefix("http://") {
            cmd.env("OLLAMA_HOST", host);
        }

        match cmd.spawn() {
            Ok(_) => Ok(()),
            Err(e) => anyhow::bail!("Failed to launch `ollama serve`: {e}"),
        }
    }

    pub async fn wait_until_reachable(
        &self,
        timeout: std::time::Duration,
        poll_interval: std::time::Duration,
    ) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if self.is_reachable().await {
                return true;
            }
            tokio::time::sleep(poll_interval).await;
        }
        self.is_reachable().await
    }

    pub async fn warm_model(&self, model: &str) -> Result<()> {
        let url = format!("{}/api/generate", self.base_url);
        let body = json!({
            "model": model,
            "prompt": "",
            "stream": false,
            "keep_alive": "30m",
        });

        let resp = self.client.post(&url).json(&body).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Failed to warm Ollama model {model}: {status} {text}");
        }
        Ok(())
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse> {
        let msgs = self.build_messages(messages, tools);
        let use_stream = delta_tx.is_some();

        // Use native Ollama /api/chat for streaming
        let body = json!({
            "model": self.model,
            "messages": msgs,
            "stream": use_stream,
            "options": {
                "temperature": 0.7,
                "num_predict": 8192,
            }
        });

        let url = format!("{}/api/chat", self.base_url);
        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Ollama API {status}: {text}");
        }

        if use_stream {
            self.handle_stream(resp, tools, delta_tx.unwrap()).await
        } else {
            let data: Value = resp.json().await?;
            let text = data["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let tool_calls = Self::parse_tool_calls(&text);
            let stop_reason = if tool_calls.is_empty() {
                StopReason::EndTurn
            } else {
                StopReason::ToolUse
            };
            let usage = TokenUsage {
                input: data["prompt_eval_count"].as_u64().unwrap_or(0) as u32,
                output: data["eval_count"].as_u64().unwrap_or(0) as u32,
            };
            Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
        }
    }

    async fn available_models(&self) -> Vec<String> {
        let url = format!("{}/api/tags", self.base_url);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(models) = data["models"].as_array() {
                        return models
                            .iter()
                            .filter_map(|m| m["name"].as_str().map(String::from))
                            .collect();
                    }
                }
                vec!["gemma3".into(), "llama3".into(), "mistral".into()]
            }
            Err(_) => vec!["gemma3".into(), "llama3".into(), "mistral".into()],
        }
    }

    fn name(&self) -> &str {
        "ollama"
    }
}

impl OllamaProvider {
    async fn handle_stream(
        &self,
        resp: reqwest::Response,
        tools: &[ToolSchema],
        tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Result<ProviderResponse> {
        let mut text = String::new();
        let mut buffer = String::new();
        let mut usage_input: u32 = 0;
        let mut usage_output: u32 = 0;

        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            // A decode/network error mid-stream is treated as end-of-stream so
            // whatever text was already accumulated is returned rather than
            // surfacing an "error decoding response body" to the user.
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => break,
            };
            let chunk_str = String::from_utf8_lossy(&chunk);
            buffer.push_str(&chunk_str);

            // Ollama streams NDJSON (one JSON object per line)
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer.drain(..pos + 1);

                if line.is_empty() {
                    continue;
                }

                let data: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                if let Some(content) = data["message"]["content"].as_str() {
                    if !content.is_empty() {
                        text.push_str(content);
                        let _ = tx.send(StreamEvent::TextDelta(content.to_string()));
                    }
                }

                if data["done"].as_bool() == Some(true) {
                    usage_input = data["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
                    usage_output = data["eval_count"].as_u64().unwrap_or(0) as u32;
                    break;
                }
            }
        }

        let tool_calls = if !tools.is_empty() {
            Self::parse_tool_calls(&text)
        } else {
            vec![]
        };

        let stop_reason = if tool_calls.is_empty() {
            StopReason::EndTurn
        } else {
            StopReason::ToolUse
        };

        let _ = tx.send(StreamEvent::Done);
        Ok(ProviderResponse { text, tool_calls, stop_reason, usage: TokenUsage { input: usage_input, output: usage_output } })
    }
}
