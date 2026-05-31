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
    },
    System(String),
    Error(String),
}

// ── Typewriter reveal ────────────────────────────────────────────────────────

/// Drives the progressive "live generation" reveal of a fully-received
/// assistant message into the live viewport block. The whole text is known up
/// front; we expose more of it each tick so it appears to be typed out, then
/// commit the finished message to scrollback.
#[derive(Debug, Clone)]
pub struct RevealState {
    pub full: String,
    pub bounds: Vec<usize>,
    pub shown: usize,
    pub step: usize,
    pub last: Instant,
}

/// Compute progressive reveal boundaries for `text` (line boundaries, or word
/// groups for short messages so prose still types out instead of popping in).
pub fn build_reveal_bounds(text: &str) -> Vec<usize> {
    let mut line_bounds: Vec<usize> = Vec::new();
    let mut off = 0usize;
    for ch in text.char_indices() {
        if ch.1 == '\n' {
            line_bounds.push(ch.0 + 1);
        }
        off = ch.0 + ch.1.len_utf8();
    }
    if line_bounds.last() != Some(&off) {
        line_bounds.push(off);
    }

    if line_bounds.len() >= 4 {
        return line_bounds;
    }

    let mut bounds: Vec<usize> = Vec::new();
    let mut words = 0usize;
    let mut prev_ws = true;
    for (i, c) in text.char_indices() {
        let is_ws = c.is_whitespace();
        if is_ws && !prev_ws {
            words += 1;
            if words % 5 == 0 {
                bounds.push(i);
            }
        }
        prev_ws = is_ws;
    }
    if bounds.last() != Some(&text.len()) {
        bounds.push(text.len());
    }
    bounds
}

// ── App Mode ─────────────────────────────────────────────────────────────────

pub const PROVIDERS: &[&str] = &["anthropic", "openai", "ollama", "gemini"];

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    Processing,
    Settings(SettingsState),
    Help,
    ModelSelect(ModelSelectState),
    ProviderSelect(ProviderSelectState),
    Confirm(ConfirmState),
    KeySelect(KeySelectState),
    KeyManage(KeyManageState),
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
        let mut fields = vec![
            SettingsField { label: "Provider".into(), key: "provider".into(), value: config.provider.clone() },
            SettingsField { label: "Model".into(), key: "model".into(), value: config.model.clone() },
        ];

        match config.provider.to_lowercase().as_str() {
            "ollama" | "gguf" => {
                fields.push(SettingsField { label: "Ollama URL".into(), key: "ollama_url".into(), value: config.ollama_url.clone() });
                fields.push(SettingsField { label: "Max Iterations".into(), key: "max_iterations".into(), value: config.max_iterations.to_string() });
                fields.push(SettingsField { label: "Max Tokens".into(), key: "max_tokens".into(), value: config.max_tokens.to_string() });
            }
            _ => {
                // API providers: show key field + Manage Keys entry
                let (key_label, key_field, prov_id) = match config.provider.to_lowercase().as_str() {
                    "openai" => ("OpenAI API Key", "openai_api_key", "openai"),
                    "gemini" => ("Google API Key", "google_api_key", "gemini"),
                    _ => ("Anthropic API Key", "anthropic_api_key", "anthropic"),
                };
                let legacy_val = match prov_id {
                    "openai" => config.openai_api_key.clone(),
                    "gemini" => config.google_api_key.clone(),
                    _ => config.anthropic_api_key.clone(),
                };
                let count = config.api_keys.get(prov_id).map(|v| v.len()).unwrap_or(0);
                let active = config.active_key.get(prov_id).cloned().unwrap_or_default();

                fields.push(SettingsField { label: key_label.into(), key: key_field.into(), value: legacy_val });
                if prov_id == "openai" {
                    fields.push(SettingsField { label: "OpenAI Base URL".into(), key: "openai_base_url".into(), value: config.openai_base_url.clone() });
                }
                let active_display = if active.is_empty() { String::new() } else { format!(" active: {active}") };
                fields.push(SettingsField {
                    label: "Manage Keys".into(),
                    key: format!("manage_keys_{prov_id}"),
                    value: format!("[{count} keys{active_display}]"),
                });
            }
        }

        Self { selected_field: 0, editing: false, fields }
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
                "max_iterations" => config.max_iterations = f.value.parse().unwrap_or(100),
                "max_tokens" => config.max_tokens = f.value.parse().unwrap_or(8192),
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
    pub return_to_settings: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProviderSelectState {
    pub providers: Vec<String>,
    pub selected: usize,
    pub return_to_settings: bool,
}

impl ProviderSelectState {
    pub fn new(current: &str, return_to_settings: bool) -> Self {
        let providers: Vec<String> = PROVIDERS.iter().map(|s| s.to_string()).collect();
        let selected = providers.iter().position(|p| p == current).unwrap_or(0);
        Self { providers, selected, return_to_settings }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum KeySelectAfter {
    Normal,
    ReturnToSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeySelectState {
    pub provider: String,
    pub entries: Vec<crate::config::ApiKeyEntry>,
    pub selected: usize,
    pub after: KeySelectAfter,
}

impl KeySelectState {
    pub fn new(provider: &str, entries: Vec<crate::config::ApiKeyEntry>, after: KeySelectAfter) -> Self {
        Self { provider: provider.to_string(), entries, selected: 0, after }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AddKeyStep {
    EnterLabel(String),
    EnterKey { label: String, key_buf: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct KeyManageState {
    pub provider: String,
    pub entries: Vec<crate::config::ApiKeyEntry>,
    pub selected: usize,
    pub add_step: Option<AddKeyStep>,
}

impl KeyManageState {
    pub fn from_config(provider: &str, config: &crate::config::Config) -> Self {
        let entries = config.api_keys.get(provider).cloned().unwrap_or_default();
        Self { provider: provider.to_string(), entries, selected: 0, add_step: None }
    }
}

// ── App State ────────────────────────────────────────────────────────────────

pub struct App {
    pub config: Config,
    pub mode: AppMode,
    pub shared_agent: Arc<Mutex<Agent>>,

    /// Interrupt flag + notifier for the running agent.
    pub cancel_flag: Arc<AtomicBool>,
    pub cancel_notify: Arc<Notify>,

    /// Finalized message blocks waiting to be flushed into the terminal's
    /// scrollback (via `insert_before`) by the main loop, then cleared. This is
    /// how history becomes natively scrollable/selectable by the terminal.
    pub to_commit: Vec<ChatMessage>,
    /// The single in-progress block shown live in the viewport: a streaming
    /// assistant reply (AssistantLive) or a running tool call.
    pub live: Option<ChatMessage>,

    // Input
    pub input: String,
    pub cursor_pos: usize,
    pub input_history: VecDeque<String>,
    pub history_index: Option<usize>,
    pub input_queue: VecDeque<String>,

    pub thinking_dots: usize,

    /// Streamed text held hidden behind the thinking spinner until the turn
    /// completes, then revealed progressively in the live block.
    pub stream_buffer: String,
    pub reveal: Option<RevealState>,

    // Status
    pub status_message: String,
    pub status_timestamp: Option<Instant>,
    pub cwd: String,

    // Token tracking
    pub last_turn_in: u32,
    pub last_turn_out: u32,
    pub session_in: u32,
    pub session_out: u32,

    pub pending_confirm: Option<oneshot::Sender<bool>>,
    pub last_ctrl_c: Option<Instant>,
    pub should_quit: bool,
}

impl App {
    pub fn new(config: Config) -> Self {
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "~".into());

        let initial_mode = if config.needs_key_select(&config.provider) {
            let entries = config.api_keys.get(config.provider.as_str()).cloned().unwrap_or_default();
            AppMode::KeySelect(KeySelectState::new(
                &config.provider.clone(),
                entries,
                KeySelectAfter::Normal,
            ))
        } else {
            AppMode::Normal
        };

        Self {
            mode: initial_mode,
            shared_agent: Arc::new(Mutex::new(Agent::new())),
            cancel_flag: Arc::new(AtomicBool::new(false)),
            cancel_notify: Arc::new(Notify::new()),
            to_commit: Vec::new(),
            live: None,
            input: String::new(),
            cursor_pos: 0,
            input_history: VecDeque::with_capacity(100),
            history_index: None,
            input_queue: VecDeque::new(),
            thinking_dots: 0,
            stream_buffer: String::new(),
            reveal: None,
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

    /// Queue a finalized block for the terminal scrollback.
    pub fn commit(&mut self, msg: ChatMessage) {
        self.to_commit.push(msg);
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
                self.stream_buffer.push_str(&text);
            }
            AgentEvent::TextDone => {
                self.finish_reveal();
                let full = std::mem::take(&mut self.stream_buffer);
                if full.trim().is_empty() {
                    return;
                }
                let bounds = build_reveal_bounds(&full);
                let step = (bounds.len() / 30).max(1);
                self.live = Some(ChatMessage::AssistantLive(String::new()));
                self.reveal = Some(RevealState { full, bounds, shown: 0, step, last: Instant::now() });
            }
            AgentEvent::ToolStart { name, input } => {
                self.finish_reveal();
                self.finalize_live();
                self.live = Some(ChatMessage::ToolCall {
                    name,
                    input_summary: summarize_input(&input),
                    success: None,
                    output: None,
                });
                self.set_status("Working...");
            }
            AgentEvent::ToolResult { name: _, success, output } => {
                if let Some(ChatMessage::ToolCall { success: s, output: o, .. }) = self.live.as_mut() {
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
                self.finalize_live();
            }
            AgentEvent::NeedConfirmation { command, reason, tx } => {
                self.finish_reveal();
                self.finalize_live();
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
                self.set_status("Done — Ready");
            }
            AgentEvent::Compacted(msg) | AgentEvent::Notice(msg) => {
                self.finish_reveal();
                self.commit(ChatMessage::System(msg));
            }
            AgentEvent::Error(err) => {
                self.finish_reveal();
                self.finalize_live();
                self.commit(ChatMessage::Error(err));
                self.mode = AppMode::Normal;
                self.thinking_dots = 0;
                self.set_status("Error — Ready");
            }
        }
    }

    /// Move the current live block into scrollback (used for finished tools or
    /// any leftover live state). Reveal-driven text uses finish_reveal instead.
    fn finalize_live(&mut self) {
        if self.reveal.is_some() {
            return; // a streaming reply owns `live`; leave it to the reveal.
        }
        if let Some(block) = self.live.take() {
            self.commit(block);
        }
    }

    /// True while a typewriter reveal is animating.
    pub fn reveal_active(&self) -> bool {
        self.reveal.is_some()
    }

    /// Advance the active reveal; commit the finished message to scrollback.
    pub fn tick_reveal(&mut self) {
        let ready = match &self.reveal {
            Some(r) => r.last.elapsed() >= Duration::from_millis(40),
            None => false,
        };
        if !ready {
            return;
        }
        let r = self.reveal.as_mut().unwrap();
        r.shown = (r.shown + r.step).min(r.bounds.len());
        let upto = r.bounds[r.shown - 1];
        let done = r.shown >= r.bounds.len();
        r.last = Instant::now();
        let shown_text = r.full[..upto].to_string();
        if done {
            let full = std::mem::take(&mut self.reveal.as_mut().unwrap().full);
            self.reveal = None;
            self.live = None;
            self.commit(ChatMessage::AssistantText(full));
        } else {
            self.live = Some(ChatMessage::AssistantLive(shown_text));
        }
    }

    /// Immediately complete any active reveal, committing the full text.
    pub fn finish_reveal(&mut self) {
        if let Some(r) = self.reveal.take() {
            self.live = None;
            self.commit(ChatMessage::AssistantText(r.full));
        }
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
            Some(idx) => format!(" ({}/{})", idx + 1, self.input_history.len()),
        }
    }
}

/// Summarize tool input for display.
pub fn summarize_input(input: &serde_json::Value) -> String {
    if let Some(obj) = input.as_object() {
        let parts: Vec<String> = obj
            .iter()
            .take(3)
            .map(|(k, v)| {
                let val_str = match v {
                    serde_json::Value::String(s) => {
                        if s.len() > 60 { format!("\"{}...\"", &s[..57]) } else { format!("\"{}\"", s) }
                    }
                    other => {
                        let s = other.to_string();
                        if s.len() > 60 { format!("{}...", &s[..57]) } else { s }
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
