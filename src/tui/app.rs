use std::collections::VecDeque;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{oneshot, Mutex, Notify};

use crate::agent::{Agent, AgentEvent};
use crate::config::Config;

// ── Message types for display ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ChatMessage {
    User(String),
    AssistantText(String),
    /// Live streaming response — replaced by AssistantText on TextDone.
    AssistantLive(String),
    ToolCall {
        name: String,
        input_summary: String,
        success: Option<bool>,
        output: Option<String>,
        expanded: bool,
    },
    System(String),
    Error(String),
}

// ── App Mode ─────────────────────────────────────────────────────────────────

pub const PROVIDERS: &[&str] = &["dev", "anthropic", "openai", "ollama", "gemini", "airllm"];

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Processing,
    Settings(SettingsState),
    Help,
    ModelSelect(ModelSelectState),
    ProviderSelect(ProviderSelectState),
    Confirm(ConfirmState),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfirmState {
    pub command: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsState {
    pub selected_field: usize,
    pub editing: bool,
    pub fields: Vec<SettingsField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsField {
    pub label: String,
    pub key: String,
    pub value: String,
}

impl SettingsState {
    pub fn from_config(config: &Config) -> Self {
        Self {
            selected_field: 0,
            editing: false,
            fields: vec![
                SettingsField {
                    label: "Provider".into(),
                    key: "provider".into(),
                    value: config.provider.clone(),
                },
                SettingsField {
                    label: "Model".into(),
                    key: "model".into(),
                    value: config.model.clone(),
                },
                SettingsField {
                    label: "Ollama URL".into(),
                    key: "ollama_url".into(),
                    value: config.ollama_url.clone(),
                },
                SettingsField {
                    label: "Anthropic API Key".into(),
                    key: "anthropic_api_key".into(),
                    value: config.anthropic_api_key.clone(),
                },
                SettingsField {
                    label: "OpenAI API Key".into(),
                    key: "openai_api_key".into(),
                    value: config.openai_api_key.clone(),
                },
                SettingsField {
                    label: "OpenAI Base URL".into(),
                    key: "openai_base_url".into(),
                    value: config.openai_base_url.clone(),
                },
                SettingsField {
                    label: "Google API Key".into(),
                    key: "google_api_key".into(),
                    value: config.google_api_key.clone(),
                },
                SettingsField {
                    label: "Max Iterations".into(),
                    key: "max_iterations".into(),
                    value: config.max_iterations.to_string(),
                },
                SettingsField {
                    label: "Max Tokens".into(),
                    key: "max_tokens".into(),
                    value: config.max_tokens.to_string(),
                },
            ],
        }
    }

    pub fn apply_to_config(&self, config: &mut Config) {
        for f in &self.fields {
            match f.key.as_str() {
                "provider" => config.provider = f.value.clone(),
                "model" => config.model = f.value.clone(),
                "ollama_url" => config.ollama_url = f.value.clone(),
                "anthropic_api_key" => config.anthropic_api_key = f.value.clone(),
                "openai_api_key" => config.openai_api_key = f.value.clone(),
                "openai_base_url" => config.openai_base_url = f.value.clone(),
                "google_api_key" => config.google_api_key = f.value.clone(),
                "max_iterations" => {
                    config.max_iterations = f.value.parse().unwrap_or(15);
                }
                "max_tokens" => {
                    config.max_tokens = f.value.parse().unwrap_or(8192);
                }
                _ => {}
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelSelectState {
    pub models: Vec<String>,
    pub selected: usize,
    pub loading: bool,
    /// When true, return to Settings after selection/cancel.
    pub return_to_settings: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSelectState {
    pub providers: Vec<String>,
    pub selected: usize,
    /// When true, return to Settings after selection/cancel.
    pub return_to_settings: bool,
}

impl ProviderSelectState {
    pub fn new(current: &str, return_to_settings: bool) -> Self {
        let providers: Vec<String> = PROVIDERS.iter().map(|s| s.to_string()).collect();
        let selected = providers.iter().position(|p| p == current).unwrap_or(0);
        Self { providers, selected, return_to_settings }
    }
}

// ── App State ────────────────────────────────────────────────────────────────

pub struct App {
    pub config: Config,
    pub mode: AppMode,
    pub shared_agent: Arc<Mutex<Agent>>,

    /// Set true to interrupt the running agent; paired notifier wakes it from
    /// an in-flight stream or backoff so cancellation is near-instant.
    pub cancel_flag: Arc<AtomicBool>,
    pub cancel_notify: Arc<Notify>,

    // Chat display
    pub messages: Vec<ChatMessage>,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub selected_message: Option<usize>,

    // Input
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: VecDeque<String>,
    pub history_index: Option<usize>,
    pub input_queue: VecDeque<String>,

    pub thinking_dots: usize,

    // Status
    pub status_message: String,
    pub status_timestamp: Option<Instant>,
    pub cwd: String,

    // Token tracking
    pub last_turn_in: u32,
    pub last_turn_out: u32,
    pub session_in: u32,
    pub session_out: u32,

    // Pending dangerous-command confirmation sender.
    pub pending_confirm: Option<oneshot::Sender<bool>>,

    // Smart Ctrl+C: track time of first press.
    pub last_ctrl_c: Option<Instant>,

    // Should quit
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());

        Self {
            mode: AppMode::Normal,
            shared_agent: Arc::new(Mutex::new(Agent::new())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cancel_notify: Arc::new(Notify::new()),
            messages: vec![ChatMessage::System(format!(
                "◈ TyCode — AI System Agent\nProvider: {}\n/help · /model · /settings · /clear · /import <path>",
                config.provider_display()
            ))],
            scroll_offset: 0,
            auto_scroll: true,
            selected_message: None,
            input: String::new(),
            cursor_pos: 0,
            input_history: VecDeque::with_capacity(100),
            history_index: None,
            input_queue: VecDeque::new(),
            thinking_dots: 0,
            status_message: "Ready".into(),
            status_timestamp: None,
            cwd,
            last_turn_in: 0,
            last_turn_out: 0,
            session_in: 0,
            session_out: 0,
            pending_confirm: None,
            last_ctrl_c: None,
            should_quit: false,
            config,
        }
    }

    /// Rough estimate of tokens in context (chars / 4).
    pub fn context_token_estimate(&self) -> usize {
        let chars: usize = self.messages.iter().map(|m| match m {
            ChatMessage::User(s) | ChatMessage::AssistantText(s) | ChatMessage::AssistantLive(s)
            | ChatMessage::System(s) | ChatMessage::Error(s) => s.len(),
            ChatMessage::ToolCall { input_summary, output, .. } => {
                input_summary.len() + output.as_ref().map(|o| o.len()).unwrap_or(0)
            }
        }).sum();
        chars / 4
    }

    /// Signal the running agent to stop as soon as possible. Idempotent.
    pub fn request_cancel(&mut self) {
        self.cancel_flag.store(true, std::sync::atomic::Ordering::SeqCst);
        self.cancel_notify.notify_waiters();
        self.input_queue.clear();
    }

    /// Handle an agent event (called from the event loop).
    pub fn handle_agent_event(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Thinking => {
                self.thinking_dots = 0;
            }
            AgentEvent::TextDelta(text) => {
                // Append directly to a live message for streaming display.
                match self.messages.last_mut() {
                    Some(ChatMessage::AssistantLive(buf)) => {
                        buf.push_str(&text);
                    }
                    _ => {
                        self.messages.push(ChatMessage::AssistantLive(text));
                    }
                }
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
            }
            AgentEvent::TextDone => {
                // Convert live message to final text.
                if let Some(ChatMessage::AssistantLive(text)) = self.messages.last().cloned() {
                    *self.messages.last_mut().unwrap() = ChatMessage::AssistantText(text);
                }
            }
            AgentEvent::ToolStart { name, input } => {
                self.messages.push(ChatMessage::ToolCall {
                    name,
                    input_summary: summarize_input(&input),
                    success: None,
                    output: None,
                    expanded: false,
                });
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
                self.set_status("Working...");
            }
            AgentEvent::ToolResult { name: _, success, output } => {
                if let Some(ChatMessage::ToolCall {
                    success: ref mut s,
                    output: ref mut o,
                    ..
                }) = self.messages.iter_mut().rfind(|m| {
                    matches!(m, ChatMessage::ToolCall { success: None, .. })
                }) {
                    *s = Some(success);
                    let display_output = if output.len() > 2000 {
                        let mut cut = 2000;
                        while !output.is_char_boundary(cut) { cut -= 1; }
                        format!("{}...\n(truncated)", &output[..cut])
                    } else {
                        output
                    };
                    *o = Some(display_output);
                }
            }
            AgentEvent::NeedConfirmation { command, reason, tx } => {
                self.pending_confirm = Some(tx);
                self.mode = AppMode::Confirm(ConfirmState { command, reason });
            }
            AgentEvent::Done { tokens_in, tokens_out } => {
                self.last_turn_in = tokens_in;
                self.last_turn_out = tokens_out;
                self.session_in += tokens_in;
                self.session_out += tokens_out;
                if !matches!(self.mode, AppMode::Confirm(_)) {
                    self.mode = AppMode::Normal;
                }
                self.thinking_dots = 0;
                self.auto_scroll = true;
                self.set_status("Done — Ready");
                self.scroll_to_bottom();
            }
            AgentEvent::Compacted(msg) | AgentEvent::Notice(msg) => {
                self.messages.push(ChatMessage::System(msg));
                if self.auto_scroll {
                    self.scroll_to_bottom();
                }
            }
            AgentEvent::Error(err) => {
                self.messages.push(ChatMessage::Error(err));
                self.mode = AppMode::Normal;
                self.thinking_dots = 0;
                self.auto_scroll = true;
                self.set_status("Error — Ready");
                self.scroll_to_bottom();
            }
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = u16::MAX;
    }

    pub fn add_to_history(&mut self, input: String) {
        if !input.is_empty() {
            self.input_history.retain(|s| s != &input);
            self.input_history.push_front(input);
            if self.input_history.len() > 100 {
                self.input_history.pop_back();
            }
        }
        self.history_index = None;
    }

    pub fn history_up(&mut self) {
        if self.input_history.is_empty() {
            return;
        }
        let new_idx = match self.history_index {
            None => 0,
            Some(i) => (i + 1).min(self.input_history.len() - 1),
        };
        self.history_index = Some(new_idx);
        if let Some(hist) = self.input_history.get(new_idx) {
            self.input = hist.clone();
            self.cursor_pos = self.input.len();
        }
    }

    pub fn history_down(&mut self) {
        match self.history_index {
            None => {}
            Some(0) => {
                self.history_index = None;
                self.input.clear();
                self.cursor_pos = 0;
            }
            Some(i) => {
                let new_idx = i - 1;
                self.history_index = Some(new_idx);
                if let Some(hist) = self.input_history.get(new_idx) {
                    self.input = hist.clone();
                    self.cursor_pos = self.input.len();
                }
            }
        }
    }

    pub fn set_status(&mut self, msg: &str) {
        self.status_message = msg.into();
        self.status_timestamp = Some(Instant::now());
    }

    pub fn update_status_expiry(&mut self) {
        if let Some(ts) = self.status_timestamp {
            if ts.elapsed() > Duration::from_secs(3) {
                self.status_message = "Ready".into();
                self.status_timestamp = None;
            }
        }
    }

    pub fn get_history_position_text(&self) -> String {
        match self.history_index {
            None => String::new(),
            Some(_) if self.input_history.is_empty() => String::new(),
            Some(idx) => format!(" ({}{})", idx + 1, format!("/{}", self.input_history.len())),
        }
    }
}

/// Summarize tool input for display.
fn summarize_input(input: &serde_json::Value) -> String {
    if let Some(obj) = input.as_object() {
        let parts: Vec<String> = obj
            .iter()
            .take(3)
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => {
                        if s.len() > 60 {
                            format!("\"{}...\"", &s[..57])
                        } else {
                            format!("\"{}\"", s)
                        }
                    }
                    other => {
                        let s = other.to_string();
                        if s.len() > 60 {
                            format!("{}...", &s[..57])
                        } else {
                            s
                        }
                    }
                };
                format!("{k}={val_str}")
            })
            .collect();
        parts.join(", ")
    } else {
        input.to_string()
    }
}
