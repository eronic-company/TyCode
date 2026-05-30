use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::{Message, Provider, ProviderResponse, Role, StopReason, StreamEvent, TokenUsage, ToolCall};
use crate::tools::ToolSchema;

pub struct GeminiProvider {
    model: String,
    api_key: String,
    max_tokens: u32,
    client: Client,
}

impl GeminiProvider {
    pub fn new(model: &str, api_key: &str, max_tokens: u32) -> Self {
        let key = if api_key.is_empty() {
            std::env::var("GOOGLE_API_KEY").unwrap_or_default()
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

    fn build_contents(&self, messages: &[Message]) -> (Option<String>, Vec<Value>) {
        let mut system = None;
        let mut contents = Vec::new();

        for m in messages {
            match m.role {
                Role::System => {
                    system = Some(m.content.clone());
                }
                Role::User | Role::Tool => {
                    contents.push(json!({
                        "role": "user",
                        "parts": [{"text": m.content}],
                    }));
                }
                Role::Assistant => {
                    contents.push(json!({
                        "role": "model",
                        "parts": [{"text": m.content}],
                    }));
                }
            }
        }
        (system, contents)
    }

    fn build_tools(&self, tools: &[ToolSchema]) -> Option<Value> {
        if tools.is_empty() {
            return None;
        }
        let decls: Vec<Value> = tools
            .iter()
            .map(|t| {
                let mut props = json!({});
                if let Some(properties) = t.parameters.get("properties") {
                    if let Some(obj) = properties.as_object() {
                        for (name, def) in obj {
                            let ptype = def
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("string")
                                .to_uppercase();
                            props[name] = json!({
                                "type": ptype,
                                "description": def.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                            });
                        }
                    }
                }
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": {
                        "type": "OBJECT",
                        "properties": props,
                        "required": t.parameters.get("required").cloned().unwrap_or(json!([])),
                    }
                })
            })
            .collect();
        Some(json!([{"functionDeclarations": decls}]))
    }
}

#[async_trait]
impl Provider for GeminiProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolSchema],
        delta_tx: Option<mpsc::UnboundedSender<StreamEvent>>,
    ) -> Result<ProviderResponse> {
        let (system, contents) = self.build_contents(messages);
        let gemini_tools = self.build_tools(tools);

        let mut body = json!({
            "contents": contents,
            "generationConfig": {
                "maxOutputTokens": self.max_tokens,
            }
        });
        if let Some(sys) = system {
            body["systemInstruction"] = json!({"parts": [{"text": sys}]});
        }
        if let Some(tools_val) = gemini_tools {
            body["tools"] = tools_val;
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API {status}: {text}");
        }

        let data: Value = resp.json().await?;

        let mut text = String::new();
        let mut tool_calls = Vec::new();

        if let Some(parts) = data["candidates"][0]["content"]["parts"].as_array() {
            for part in parts {
                if let Some(t) = part["text"].as_str() {
                    text.push_str(t);
                }
                if let Some(fc) = part.get("functionCall") {
                    tool_calls.push(ToolCall {
                        id: uuid::Uuid::new_v4().to_string(),
                        name: fc["name"].as_str().unwrap_or("").to_string(),
                        input: fc.get("args").cloned().unwrap_or(json!({})),
                    });
                }
            }
        }

        // Send as single delta if streaming was requested
        if let Some(tx) = delta_tx {
            if !text.is_empty() {
                let _ = tx.send(StreamEvent::TextDelta(text.clone()));
            }
            let _ = tx.send(StreamEvent::Done);
        }

        let stop_reason = if !tool_calls.is_empty() {
            StopReason::ToolUse
        } else {
            StopReason::EndTurn
        };

        let usage = TokenUsage {
            input: data["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0) as u32,
            output: data["usageMetadata"]["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
        };

        Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
    }

    async fn available_models(&self) -> Vec<String> {
        vec![
            "gemini-2.0-flash".into(),
            "gemini-2.0-flash-lite".into(),
            "gemini-1.5-pro".into(),
            "gemini-1.5-flash".into(),
        ]
    }

    fn name(&self) -> &str {
        "gemini"
    }
}
