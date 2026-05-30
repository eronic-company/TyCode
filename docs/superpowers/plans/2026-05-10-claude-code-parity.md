# Claude Code Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring TyCode to Claude Code feature parity: live streaming output, multiline input, smart Ctrl+C, project context auto-injection, real token tracking, dangerous-command confirmation, and context auto-compaction.

**Architecture:** Eight self-contained features implemented in dependency order. Features 5→2→1 touch only the TUI layer. Feature 3 touches Agent startup. Features 6→7 thread token data from providers through to the status bar. Features 4 and 8 add the deepest agent-loop changes.

**Tech Stack:** Rust, Tokio, Ratatui 0.29, Crossterm 0.28, `tokio::sync::oneshot` (new, already in tokio::full)

---

## File Map

| File | Changes |
|------|---------|
| `src/tui/app.rs` | `ChatMessage::AssistantLive`, `auto_scroll`, `last_ctrl_c`, token fields, `AppMode::Confirm`, `AgentEvent::NeedConfirmation`, `AgentEvent::Compacted` |
| `src/tui/input.rs` | Shift+Enter, smart Ctrl+C, `AppMode::Confirm` keys |
| `src/tui/ui.rs` | Live message rendering, multiline cursor, status bar tokens, confirm overlay |
| `src/agent.rs` | `project_files`, streaming-direct, token accumulation, dangerous sentinel handling, `compact_history()` |
| `src/provider/mod.rs` | `TokenUsage`, extend `ProviderResponse` |
| `src/provider/anthropic.rs` | Parse `usage` → `TokenUsage` |
| `src/provider/openai.rs` | Parse `usage` → `TokenUsage` |
| `src/provider/ollama.rs` | Parse `eval_count` → `TokenUsage` |
| `src/provider/gemini.rs` | Parse `usageMetadata` → `TokenUsage` |
| `src/tools/shell.rs` | `is_dangerous()` check, sentinel result |
| `src/config.rs` | `compact_threshold` field |
| `src/main.rs` | Project file injection on startup |

---

## Task 1 — Live Streaming Output

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/ui.rs`

### What changes
Currently `TextDelta` events accumulate in `buffered_response` and everything is revealed on `Done`. We replace this with a `ChatMessage::AssistantLive(String)` that is pushed to `messages` immediately and updated in-place on each delta. On `Done` it is renamed to `AssistantText`. We also add `auto_scroll: bool` so manual scrolling up doesn't snap the view back down.

- [ ] **Step 1 — Add `AssistantLive` variant and `auto_scroll` field**

In `src/tui/app.rs`:

```rust
// In ChatMessage enum, add after AssistantText:
AssistantLive(String),
```

Add to `App` struct:
```rust
pub auto_scroll: bool,
```

In `App::new()` initialiser:
```rust
auto_scroll: true,
// Remove: buffered_response: String::new(),
// Remove: pending_messages: Vec::new(),
```

Remove these two fields from the struct declaration too:
```rust
// DELETE: pub pending_messages: Vec<ChatMessage>,
// DELETE: pub buffered_response: String,
```

- [ ] **Step 2 — Rewrite `handle_agent_event` for streaming**

Replace the entire `handle_agent_event` method in `src/tui/app.rs`:

```rust
pub fn handle_agent_event(&mut self, event: AgentEvent) {
    match event {
        AgentEvent::Thinking => {
            self.thinking_dots = 0;
        }
        AgentEvent::TextDelta(text) => {
            // Find the live message and append, or create one.
            if let Some(ChatMessage::AssistantLive(ref mut s)) = self.messages.last_mut() {
                s.push_str(&text);
            } else {
                self.messages.push(ChatMessage::AssistantLive(text));
            }
            if self.auto_scroll {
                self.scroll_to_bottom();
            }
        }
        AgentEvent::TextDone => {
            // Promote live message to final text.
            if let Some(ChatMessage::AssistantLive(s)) = self.messages.last_mut() {
                let text = std::mem::take(s);
                *self.messages.last_mut().unwrap() = ChatMessage::AssistantText(text);
            }
        }
        AgentEvent::ToolStart { name, input } => {
            self.messages.push(ChatMessage::ToolCall {
                name,
                input_summary: summarize_input(&input),
                success: None,
                output: None,
            });
            self.set_status("Working...");
            if self.auto_scroll {
                self.scroll_to_bottom();
            }
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
        AgentEvent::Done { .. } => {
            self.mode = AppMode::Normal;
            self.thinking_dots = 0;
            self.auto_scroll = true;
            self.set_status("Done — Ready");
            self.scroll_to_bottom();
        }
        AgentEvent::Error(err) => {
            // Promote any live message first.
            if let Some(ChatMessage::AssistantLive(s)) = self.messages.last_mut() {
                let text = std::mem::take(s);
                *self.messages.last_mut().unwrap() = ChatMessage::AssistantText(text);
            }
            self.messages.push(ChatMessage::Error(err));
            self.mode = AppMode::Normal;
            self.thinking_dots = 0;
            self.auto_scroll = true;
            self.set_status("Error — Ready");
            self.scroll_to_bottom();
        }
    }
}
```

Note: `AgentEvent::Done` now has `{ .. }` — we'll add token fields in Task 6. For now add a placeholder:

```rust
// In AgentEvent enum (src/tui/app.rs imports from agent.rs — change agent.rs):
Done,  // unchanged for now, token fields added in Task 6
```

- [ ] **Step 3 — Auto-scroll: stop snapping when user scrolls up**

In `src/tui/ui.rs`, find the auto-scroll block in `render_chat` (around line 239):

```rust
// Replace the existing auto-scroll block:
if app.auto_scroll && app.scroll_offset == u16::MAX {
    app.scroll_offset = total_lines.saturating_sub(visible_height);
} else if app.scroll_offset == u16::MAX {
    app.scroll_offset = total_lines.saturating_sub(visible_height);
}
```

With:
```rust
if app.scroll_offset == u16::MAX {
    app.scroll_offset = total_lines.saturating_sub(visible_height);
}
// Clamp
app.scroll_offset = app
    .scroll_offset
    .min(total_lines.saturating_sub(visible_height));
```

In `src/tui/input.rs`, when user scrolls up (PgUp / Up while scrolled), set `auto_scroll = false`. In `handle_normal_key`:

```rust
KeyCode::PageUp => {
    app.scroll_offset = app.scroll_offset.saturating_sub(10);
    app.auto_scroll = false;
    true
}
KeyCode::Up if app.input.is_empty() => {
    // history nav only when input is non-empty
    app.history_up();
    true
}
```

Actually keep Up for history — only PgUp turns off auto_scroll. Replace only PageUp:
```rust
KeyCode::PageUp => {
    app.scroll_offset = app.scroll_offset.saturating_sub(10);
    app.auto_scroll = false;
    true
}
```

- [ ] **Step 4 — Render `AssistantLive` with trailing cursor glyph**

In `src/tui/ui.rs` inside `render_chat`, in the `for msg in &app.messages` loop, add after the `AssistantText` arm:

```rust
ChatMessage::AssistantLive(text) => {
    all_lines.push(Line::from(""));
    let rendered = markdown::render_markdown(text);
    let mut live_lines = rendered;
    // Append blinking cursor to last line
    if let Some(last) = live_lines.last_mut() {
        last.spans.push(Span::styled(
            "▊",
            Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::RAPID_BLINK),
        ));
    } else {
        live_lines.push(Line::from(Span::styled(
            "▊",
            Style::default().fg(Color::Magenta),
        )));
    }
    all_lines.extend(live_lines);
}
```

- [ ] **Step 5 — Remove thinking indicator while streaming (text is already visible)**

In `src/tui/ui.rs`, the thinking indicator block:
```rust
if matches!(app.mode, AppMode::Processing) {
```

Change to only show if there is no live message yet:
```rust
if matches!(app.mode, AppMode::Processing)
    && !app.messages.iter().any(|m| matches!(m, ChatMessage::AssistantLive(_)))
{
```

- [ ] **Step 6 — Build and verify**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```
Expected: no errors. Warnings about unused `buffered_response` / `pending_messages` references may appear — fix by removing any remaining references to those deleted fields.

- [ ] **Step 7 — Commit**
```bash
git add src/tui/app.rs src/tui/ui.rs src/tui/input.rs
git commit -m "feat: live streaming output — text appears as it arrives"
```

---

## Task 2 — Multiline Input (Shift+Enter)

**Files:**
- Modify: `src/tui/input.rs`
- Modify: `src/tui/ui.rs`

- [ ] **Step 1 — Handle Shift+Enter in normal mode**

In `src/tui/input.rs`, in `handle_normal_key`, add before the existing `KeyCode::Enter` arm:

```rust
KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
    app.input.insert(app.cursor_pos, '\n');
    app.cursor_pos += 1;
    true
}
```

Make sure this arm comes **before** the plain `KeyCode::Enter` arm in the match.

- [ ] **Step 2 — Handle Shift+Enter in processing mode too**

In `handle_processing_key`, add before the `KeyCode::Enter` arm:

```rust
KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
    app.input.insert(app.cursor_pos, '\n');
    app.cursor_pos += 1;
}
```

- [ ] **Step 3 — Show line-count badge in input title**

In `src/tui/ui.rs` in `render_input`, replace the title for non-processing state:

```rust
let title = if is_processing {
    // ... existing processing title ...
} else {
    let history_pos = app.get_history_position_text();
    let line_count = app.input.chars().filter(|&c| c == '\n').count() + 1;
    if line_count > 1 {
        format!(" > {} lines{} ", line_count, history_pos)
    } else {
        format!(" >{} ", history_pos)
    }
};
```

- [ ] **Step 4 — Fix cursor position for multiline input**

In `src/tui/ui.rs` in `render_input`, replace the cursor positioning block:

```rust
// Show cursor — account for line wrapping and newlines.
if !matches!(app.mode, AppMode::Settings(_) | AppMode::Help | AppMode::ModelSelect(_) | AppMode::ProviderSelect(_)) {
    let inner_width = area.width.saturating_sub(2) as usize;

    // Count how many newlines appear before cursor_pos, and the column on the current line.
    let text_before = &app.input[..app.cursor_pos.min(app.input.len())];
    let newline_rows = text_before.chars().filter(|&c| c == '\n').count();
    let last_newline = text_before.rfind('\n').map(|i| i + 1).unwrap_or(0);
    let col_chars = &text_before[last_newline..];
    let col = if inner_width > 0 { col_chars.len() % inner_width } else { col_chars.len() };
    let wrap_rows = if inner_width > 0 { col_chars.len() / inner_width } else { 0 };

    let cursor_col = (area.x + 1 + col as u16).min(area.x + area.width.saturating_sub(2));
    let cursor_row = (area.y + 1 + newline_rows as u16 + wrap_rows as u16)
        .min(area.y + area.height.saturating_sub(2));

    f.set_cursor_position((cursor_col, cursor_row));
}
```

- [ ] **Step 5 — Fix dynamic input height to count newlines too**

In `src/tui/ui.rs` in `render()`, replace the `input_rows` calculation:

```rust
let input_rows = if app.input.is_empty() || inner_width == 0 {
    1u16
} else {
    let newlines = app.input.chars().filter(|&c| c == '\n').count() as u16;
    let last_segment = app.input.split('\n').last().unwrap_or("");
    let wrap_rows = if inner_width > 0 {
        ((last_segment.len().saturating_sub(1)) / inner_width) as u16
    } else { 0 };
    (newlines + wrap_rows + 1).min(6)
};
```

- [ ] **Step 6 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 7 — Commit**
```bash
git add src/tui/input.rs src/tui/ui.rs
git commit -m "feat: multiline input with Shift+Enter"
```

---

## Task 3 — Smart Ctrl+C (Cancel vs Quit)

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/input.rs`

- [ ] **Step 1 — Add `last_ctrl_c` to App**

In `src/tui/app.rs`, in the `App` struct add:
```rust
pub last_ctrl_c: Option<Instant>,
```

In `App::new()` add:
```rust
last_ctrl_c: None,
```

- [ ] **Step 2 — Replace the global Ctrl+C handler**

In `src/tui/input.rs`, at the top of `handle_key`, replace:

```rust
if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
    app.should_quit = true;
    return true;
}
```

With:

```rust
if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
    let now = std::time::Instant::now();
    let double_press = app
        .last_ctrl_c
        .map(|t| t.elapsed().as_millis() < 2000)
        .unwrap_or(false);

    if double_press {
        app.should_quit = true;
    } else {
        app.last_ctrl_c = Some(now);
        match &app.mode {
            AppMode::Processing => {
                // Cancel queue but let agent finish current iteration.
                app.input_queue.clear();
                app.set_status("Queue cleared — Ctrl+C again to quit");
            }
            _ => {
                app.input.clear();
                app.cursor_pos = 0;
                app.set_status("Ctrl+C again to quit");
            }
        }
    }
    return true;
}
```

- [ ] **Step 3 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 4 — Commit**
```bash
git add src/tui/app.rs src/tui/input.rs
git commit -m "feat: Ctrl+C cancels queue, double Ctrl+C quits"
```

---

## Task 4 — TYCODE.md + README Auto-Inject

**Files:**
- Modify: `src/agent.rs`
- Modify: `src/main.rs`
- Modify: `src/tui/input.rs`

- [ ] **Step 1 — Add `project_files` to Agent**

In `src/agent.rs`, extend the `Agent` struct:

```rust
pub struct Agent {
    messages: Vec<Message>,
    tools: Vec<ToolSchema>,
    custom_system_prompt: Option<String>,
    project_files: Vec<(String, String)>,  // (filename, content)
}
```

In `Agent::new()`:
```rust
project_files: Vec::new(),
```

- [ ] **Step 2 — Add `inject_project_files` and `reinject_project_files`**

In `src/agent.rs`, add these methods to `impl Agent`:

```rust
/// Scan cwd for TYCODE.md and README.md and inject them.
/// Returns list of filenames that were loaded.
pub fn inject_project_files(&mut self, cwd: &str) -> Vec<String> {
    const MAX_BYTES: usize = 100 * 1024;
    let candidates = ["TYCODE.md", "README.md", "README"];
    let mut loaded = Vec::new();

    for name in &candidates {
        let path = std::path::Path::new(cwd).join(name);
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) if !content.trim().is_empty() && content.len() <= MAX_BYTES => {
                    self.project_files.push((name.to_string(), content.clone()));
                    self.inject_context(name, &content);
                    loaded.push(name.to_string());
                }
                Ok(content) if content.len() > MAX_BYTES => {
                    eprintln!("[tycode] {name} too large to inject (>100KB), skipping");
                }
                _ => {}
            }
        }
    }
    loaded
}

/// Re-inject project files after clear_history().
pub fn reinject_project_files(&mut self) {
    let files = self.project_files.clone();
    for (name, content) in &files {
        self.inject_context(name, content);
    }
}
```

- [ ] **Step 3 — Call on startup in `main.rs`**

In `src/main.rs`, after `App::new(config)` is created and before the loop, inject files and update the banner. Replace the banner push in `App::new()` (or add a System message):

In `run_app`, after `let mut app = App::new(config);`:

```rust
// Inject project files and update banner.
let loaded_files = {
    let mut agent = app.shared_agent.lock().await;
    agent.inject_project_files(&app.cwd)
};
if !loaded_files.is_empty() {
    app.messages.push(crate::tui::app::ChatMessage::System(format!(
        "Loaded: {}",
        loaded_files.join(" · ")
    )));
}
```

- [ ] **Step 4 — Re-inject after `/clear` and `/cache`**

In `src/tui/input.rs`, in the `/clear` handler, after `agent.clear_history()`:

```rust
agent.clear_history();
agent.reinject_project_files();
// Re-inject working dir context too
agent.inject_context(
    "current_directory",
    &format!("You are working in directory: {}", app.cwd),
);
```

Same for `/cache` handler after `agent.clear_history()`:
```rust
agent.clear_history();
agent.reinject_project_files();
```

- [ ] **Step 5 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 6 — Commit**
```bash
git add src/agent.rs src/main.rs src/tui/input.rs
git commit -m "feat: auto-inject TYCODE.md and README into agent context on startup"
```

---

## Task 5 — Real Token Usage per Turn

**Files:**
- Modify: `src/provider/mod.rs`
- Modify: `src/provider/anthropic.rs`
- Modify: `src/provider/openai.rs`
- Modify: `src/provider/ollama.rs`
- Modify: `src/provider/gemini.rs`
- Modify: `src/agent.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1 — Add `TokenUsage` to provider/mod.rs**

In `src/provider/mod.rs`, add before `ProviderResponse`:

```rust
#[derive(Debug, Clone, Default)]
pub struct TokenUsage {
    pub input: u32,
    pub output: u32,
}
```

Extend `ProviderResponse`:
```rust
pub struct ProviderResponse {
    pub text: String,
    pub tool_calls: Vec<ToolCall>,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
}
```

- [ ] **Step 2 — Parse usage in `anthropic.rs`**

In `handle_response` in `src/provider/anthropic.rs`, after the `stop_reason` assignment:

```rust
let usage = TokenUsage {
    input: data["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32,
    output: data["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32,
};
Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
```

In `handle_stream`, after the stream loop ends, find where you build the final `ProviderResponse` and add the usage field. In Anthropic SSE, `message_delta` events carry `usage.output_tokens` and `message_start` carries `usage.input_tokens`. Track them:

At the top of `handle_stream`, add:
```rust
let mut input_tokens: u32 = 0;
let mut output_tokens: u32 = 0;
```

In the `"message_start"` event block add:
```rust
"message_start" => {
    input_tokens = data["message"]["usage"]["input_tokens"]
        .as_u64().unwrap_or(0) as u32;
}
```

In the `"message_delta"` event block add:
```rust
"message_delta" => {
    output_tokens = data["usage"]["output_tokens"]
        .as_u64().unwrap_or(0) as u32;
    // ... existing stop_reason parsing ...
}
```

In the final return:
```rust
Ok(ProviderResponse {
    text,
    tool_calls,
    stop_reason,
    usage: TokenUsage { input: input_tokens, output: output_tokens },
})
```

- [ ] **Step 3 — Parse usage in `openai.rs`**

In `src/provider/openai.rs`, in `handle_response` (non-stream), after building text/tool_calls:

```rust
let usage = TokenUsage {
    input: data["usage"]["prompt_tokens"].as_u64().unwrap_or(0) as u32,
    output: data["usage"]["completion_tokens"].as_u64().unwrap_or(0) as u32,
};
Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
```

For the streaming path, OpenAI only sends usage in the final chunk when `stream_options: {include_usage: true}` is set. Add to the request body:
```rust
body["stream_options"] = json!({"include_usage": true});
```
Then in the stream loop, capture the last `data.usage` chunk:
```rust
if let Some(u) = data.get("usage") {
    input_tokens = u["prompt_tokens"].as_u64().unwrap_or(0) as u32;
    output_tokens = u["completion_tokens"].as_u64().unwrap_or(0) as u32;
}
```

- [ ] **Step 4 — Parse usage in `ollama.rs`**

In `src/provider/ollama.rs`, in `available_models` this isn't needed. In `chat()`, non-stream path:

```rust
let usage = TokenUsage {
    input: data["prompt_eval_count"].as_u64().unwrap_or(0) as u32,
    output: data["eval_count"].as_u64().unwrap_or(0) as u32,
};
Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
```

In `handle_stream`, at the end of the stream loop, capture from `done` message:
```rust
if data["done"].as_bool() == Some(true) {
    input_tokens = data["prompt_eval_count"].as_u64().unwrap_or(0) as u32;
    output_tokens = data["eval_count"].as_u64().unwrap_or(0) as u32;
    break;
}
```

- [ ] **Step 5 — Parse usage in `gemini.rs`**

In `src/provider/gemini.rs`, in the response handling:

```rust
let usage = TokenUsage {
    input: data["usageMetadata"]["promptTokenCount"].as_u64().unwrap_or(0) as u32,
    output: data["usageMetadata"]["candidatesTokenCount"].as_u64().unwrap_or(0) as u32,
};
Ok(ProviderResponse { text, tool_calls, stop_reason, usage })
```

- [ ] **Step 6 — Thread usage through AgentEvent::Done**

In `src/tui/app.rs`, extend `AgentEvent`:

```rust
// Replace: Done,
Done {
    tokens_in: u32,
    tokens_out: u32,
},
```

In `src/agent.rs`, accumulate per-turn usage across the loop:

At the top of `run()`, add:
```rust
let mut total_in: u32 = 0;
let mut total_out: u32 = 0;
```

After `let response = match response { Ok(r) => r, ... }`:
```rust
total_in += response.usage.input;
total_out += response.usage.output;
```

Change the Done send at the bottom:
```rust
let _ = event_tx.send(AgentEvent::Done {
    tokens_in: total_in,
    tokens_out: total_out,
});
```

- [ ] **Step 7 — Store in App and display**

In `src/tui/app.rs`, add to `App` struct:
```rust
pub last_turn_in: u32,
pub last_turn_out: u32,
pub session_in: u32,
pub session_out: u32,
pub msg_count: usize,
```

In `App::new()`:
```rust
last_turn_in: 0,
last_turn_out: 0,
session_in: 0,
session_out: 0,
msg_count: 0,
```

In `handle_agent_event`, `Done` arm:
```rust
AgentEvent::Done { tokens_in, tokens_out } => {
    self.last_turn_in = tokens_in;
    self.last_turn_out = tokens_out;
    self.session_in += tokens_in;
    self.session_out += tokens_out;
    self.msg_count = self.messages.iter()
        .filter(|m| matches!(m, ChatMessage::User(_) | ChatMessage::AssistantText(_)))
        .count();
    self.mode = AppMode::Normal;
    self.thinking_dots = 0;
    self.auto_scroll = true;
    self.set_status("Done — Ready");
    self.scroll_to_bottom();
}
```

Also update pattern matching in `handle_agent_event` for the `Error` arm since it doesn't receive Done — no change needed there.

- [ ] **Step 8 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 9 — Commit**
```bash
git add src/provider/ src/agent.rs src/tui/app.rs
git commit -m "feat: real token usage tracking per turn from provider APIs"
```

---

## Task 6 — Token Counter + Context Estimate in Status Bar

**Files:**
- Modify: `src/tui/app.rs`
- Modify: `src/tui/ui.rs`

- [ ] **Step 1 — Add `context_token_estimate()` to App**

In `src/tui/app.rs`:

```rust
pub fn context_token_estimate(&self) -> usize {
    self.messages.iter().map(|m| {
        match m {
            ChatMessage::User(s) | ChatMessage::AssistantText(s) |
            ChatMessage::AssistantLive(s) | ChatMessage::System(s) |
            ChatMessage::Error(s) => s.len(),
            ChatMessage::ToolCall { name, input_summary, output, .. } => {
                name.len() + input_summary.len() +
                output.as_ref().map(|o| o.len()).unwrap_or(0)
            }
        }
    }).sum::<usize>() / 4
}
```

- [ ] **Step 2 — Rewrite `render_status` with token info and colour**

In `src/tui/ui.rs`, replace `render_status`:

```rust
fn render_status(f: &mut Frame, app: &App, area: Rect) {
    let commands = " ⌘help  model  settings  clear  │  PgUp/Dn  C-c×2:quit ";

    let ctx_tokens = app.context_token_estimate();
    let ctx_color = if ctx_tokens > 90_000 {
        Color::Red
    } else if ctx_tokens > 60_000 {
        Color::Yellow
    } else {
        Color::Green
    };

    let token_str = if app.last_turn_in > 0 || app.last_turn_out > 0 {
        format!(
            " ctx~{}  ↑{}↓{}  {}msgs ",
            fmt_tokens(ctx_tokens),
            fmt_tokens(app.last_turn_in as usize),
            fmt_tokens(app.last_turn_out as usize),
            app.msg_count,
        )
    } else {
        format!(" ctx~{}  {}msgs ", fmt_tokens(ctx_tokens), app.msg_count)
    };

    let status = &app.status_message;
    let status_with_indicator = if let Some(ts) = app.status_timestamp {
        let elapsed = ts.elapsed().as_millis() as u64;
        let remaining = (3000u64).saturating_sub(elapsed);
        let dot = if remaining > 2000 { "●" } else if remaining > 1000 { "○" } else { "·" };
        format!("{} {} ", status, dot)
    } else {
        format!("{} ", status)
    };

    let total_right = token_str.len() + status_with_indicator.len();
    let padding = (area.width as usize).saturating_sub(commands.len() + total_right);

    let status_line = Line::from(vec![
        Span::styled(commands, Style::default().fg(DIM).bg(STATUS_BG)),
        Span::styled(" ".repeat(padding), Style::default().bg(STATUS_BG)),
        Span::styled(&token_str, Style::default().fg(ctx_color).bg(STATUS_BG)),
        Span::styled(&status_with_indicator, Style::default().fg(Color::Green).bg(STATUS_BG)),
    ]);

    f.render_widget(
        Paragraph::new(status_line).style(Style::default().bg(STATUS_BG)),
        area,
    );
}

fn fmt_tokens(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
```

- [ ] **Step 3 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 4 — Commit**
```bash
git add src/tui/app.rs src/tui/ui.rs
git commit -m "feat: token counter and context estimate in status bar"
```

---

## Task 7 — Dangerous Command Confirmation

**Files:**
- Modify: `src/tools/shell.rs`
- Modify: `src/tui/app.rs`
- Modify: `src/agent.rs`
- Modify: `src/tui/input.rs`
- Modify: `src/tui/ui.rs`

- [ ] **Step 1 — Add `is_dangerous()` to shell.rs**

In `src/tools/shell.rs`, add before `bash_execute`:

```rust
/// Returns a human-readable reason if the command is considered dangerous.
pub fn is_dangerous(cmd: &str) -> Option<&'static str> {
    let c = cmd.to_lowercase();
    let patterns: &[(&str, &str)] = &[
        ("rm -rf", "recursive force delete"),
        ("rm -fr", "recursive force delete"),
        ("rm --recursive", "recursive delete"),
        ("dd if=", "disk write (dd)"),
        ("mkfs", "format filesystem"),
        ("drop table", "SQL: drop table"),
        ("drop database", "SQL: drop database"),
        (":(){:|:&};:", "fork bomb"),
        ("git push --force", "force push"),
        ("git push -f ", "force push"),
        ("git reset --hard", "hard reset — discards commits"),
        ("chmod -r 777", "world-writable recursive chmod"),
        ("chmod 777 /", "world-writable root"),
        ("truncate -s 0", "truncate file to zero"),
        ("> /dev/sd", "write to raw disk device"),
        ("sudo rm", "sudo delete"),
        ("sudo dd", "sudo disk write"),
    ];
    for (pattern, reason) in patterns {
        if c.contains(pattern) {
            return Some(reason);
        }
    }
    None
}
```

- [ ] **Step 2 — Add `NeedConfirmation` event and `ConfirmState` / `AppMode::Confirm`**

In `src/tui/app.rs`:

```rust
// Add to AppMode:
Confirm(ConfirmState),

// Add struct:
#[derive(Debug, Clone)]
pub struct ConfirmState {
    pub tool: String,
    pub command: String,
    pub reason: String,
}
```

In the `AgentEvent` enum (in `src/agent.rs`, imported into app.rs):
```rust
NeedConfirmation {
    tool: String,
    command: String,
    reason: String,
    tx: tokio::sync::oneshot::Sender<bool>,
},
```

Since `AgentEvent` has a `oneshot::Sender` (not `Clone`), remove `#[derive(Clone)]` from it and replace the derive with just `#[derive(Debug)]`.

Update any code that assumed `AgentEvent: Clone` — there shouldn't be any.

- [ ] **Step 3 — Handle `NeedConfirmation` in `handle_agent_event`**

In `src/tui/app.rs`, add to `handle_agent_event`:

```rust
AgentEvent::NeedConfirmation { tool, command, reason, tx } => {
    // Store the sender; we'll send the response when the user presses Y/N.
    self.pending_confirm = Some(tx);
    self.mode = AppMode::Confirm(ConfirmState { tool, command, reason });
}
```

Add `pending_confirm` field to `App`:
```rust
pub pending_confirm: Option<tokio::sync::oneshot::Sender<bool>>,
```

In `App::new()`:
```rust
pending_confirm: None,
```

- [ ] **Step 4 — Handle Y/N keys in `AppMode::Confirm`**

In `src/tui/input.rs`, in `handle_key`:

```rust
AppMode::Confirm(_) => {
    handle_confirm_key(app, key);
    true
}
```

Add function:
```rust
fn handle_confirm_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            if let Some(tx) = app.pending_confirm.take() {
                let _ = tx.send(true);
            }
            app.mode = AppMode::Processing;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
            if let Some(tx) = app.pending_confirm.take() {
                let _ = tx.send(false);
            }
            app.mode = AppMode::Processing;
        }
        _ => {}
    }
}
```

- [ ] **Step 5 — Check for danger in agent.rs before executing bash**

In `src/agent.rs`, in the tool execution loop, replace:

```rust
let result = tokio::task::spawn_blocking(move || {
    tools::execute_tool(&tool_name, &tool_input)
})
.await
.unwrap_or_else(|e| tools::ToolResult::err(format!("Task panic: {e}")));
```

With:

```rust
// Check for dangerous bash commands before executing.
let result = if tool_name == "bash" {
    let cmd = tool_input["command"].as_str().unwrap_or("").to_string();
    if let Some(reason) = crate::tools::shell::is_dangerous(&cmd) {
        let (confirm_tx, confirm_rx) = tokio::sync::oneshot::channel::<bool>();
        let _ = event_tx.send(AgentEvent::NeedConfirmation {
            tool: tool_name.clone(),
            command: cmd.clone(),
            reason: reason.to_string(),
            tx: confirm_tx,
        });
        // Block agent until user responds.
        match confirm_rx.await {
            Ok(true) => {
                tokio::task::spawn_blocking(move || {
                    tools::execute_tool(&tool_name, &tool_input)
                })
                .await
                .unwrap_or_else(|e| tools::ToolResult::err(format!("Task panic: {e}")))
            }
            _ => tools::ToolResult::err(format!("User denied: {}", reason)),
        }
    } else {
        tokio::task::spawn_blocking(move || {
            tools::execute_tool(&tool_name, &tool_input)
        })
        .await
        .unwrap_or_else(|e| tools::ToolResult::err(format!("Task panic: {e}")))
    }
} else {
    tokio::task::spawn_blocking(move || {
        tools::execute_tool(&tool_name, &tool_input)
    })
    .await
    .unwrap_or_else(|e| tools::ToolResult::err(format!("Task panic: {e}")))
};
```

- [ ] **Step 6 — Render confirm overlay in ui.rs**

In `src/tui/ui.rs`, add to `render()` overlay match:
```rust
AppMode::Confirm(state) => render_confirm_overlay(f, state, size),
```

Add function:
```rust
fn render_confirm_overlay(f: &mut Frame, state: &super::app::ConfirmState, area: Rect) {
    let width = 70u16.min(area.width.saturating_sub(4));
    let height = 10u16;
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let block = Block::default()
        .title(" ⚠  Dangerous Command — Confirm ")
        .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" [Y]", Style::default().fg(Color::Green)),
            Span::styled(" Allow  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[N]", Style::default().fg(Color::Red)),
            Span::styled(" Deny ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Red))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let cmd_display = if state.command.len() > 60 {
        format!("{}...", &state.command[..57])
    } else {
        state.command.clone()
    };

    let text = vec![
        Line::from(vec![
            Span::styled("Tool:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.tool, Style::default().fg(Color::Yellow)),
        ]),
        Line::from(vec![
            Span::styled("Command: ", Style::default().fg(Color::DarkGray)),
            Span::styled(cmd_display, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Risk:    ", Style::default().fg(Color::DarkGray)),
            Span::styled(&state.reason, Style::default().fg(Color::Red)),
        ]),
    ];

    f.render_widget(Paragraph::new(text).block(block), popup_area);
}
```

- [ ] **Step 7 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 8 — Commit**
```bash
git add src/tools/shell.rs src/tui/app.rs src/agent.rs src/tui/input.rs src/tui/ui.rs
git commit -m "feat: dangerous command confirmation overlay before executing destructive bash"
```

---

## Task 8 — Auto-Compact Context

**Files:**
- Modify: `src/config.rs`
- Modify: `src/agent.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1 — Add `compact_threshold` to Config**

In `src/config.rs`:

```rust
// In Config struct:
#[serde(default = "default_compact_threshold")]
pub compact_threshold: usize,

// Add default function:
fn default_compact_threshold() -> usize { 80_000 }

// In Default impl:
compact_threshold: default_compact_threshold(),
```

- [ ] **Step 2 — Add `Compacted` agent event**

In `src/agent.rs` (AgentEvent enum):
```rust
Compacted(String),  // summary of what was compacted
```

- [ ] **Step 3 — Add `compact_history()` to Agent**

In `src/agent.rs`:

```rust
/// Summarise old conversation turns to free context space.
/// Keeps the system prompt and the last 4 user/assistant turns intact.
async fn compact_history(
    &mut self,
    provider: &dyn crate::provider::Provider,
    event_tx: &mpsc::UnboundedSender<AgentEvent>,
) {
    // Find the system message and last 4 conversation messages.
    let system_idx = self.messages.iter()
        .position(|m| matches!(m.role, Role::System));
    let conversation: Vec<&Message> = self.messages.iter()
        .filter(|m| !matches!(m.role, Role::System))
        .collect();

    if conversation.len() <= 8 {
        return; // Not enough to compact
    }

    // Build the compaction prompt from all but the last 4 turns.
    let keep_count = 8; // last 4 user+assistant pairs
    let to_summarise = &conversation[..conversation.len().saturating_sub(keep_count)];
    let history_text: String = to_summarise.iter().map(|m| {
        let role = match m.role { Role::User => "User", Role::Tool => "Tool", _ => "Assistant" };
        format!("{}: {}\n", role, m.content)
    }).collect();

    let summary_prompt = format!(
        "Summarise the following conversation in full detail. Preserve all decisions, \
         code changes, file paths, commands run, and their outputs. Be thorough:\n\n{}",
        history_text
    );

    let summary_msg = vec![Message::user(&summary_prompt)];
    let result = provider.chat(&summary_msg, &[], None).await;

    if let Ok(resp) = result {
        let old_count = to_summarise.len();
        // Rebuild messages: system + summary pair + last 4 turns.
        let last_turns: Vec<Message> = self.messages
            .iter()
            .filter(|m| !matches!(m.role, Role::System))
            .rev()
            .take(keep_count)
            .rev()
            .cloned()
            .collect();

        self.messages.clear();
        if let Some(idx) = system_idx {
            // Re-add system message first (we still have it).
        }
        // Re-insert system from project_files (rebuild cleanly).
        let system_prompt = self.custom_system_prompt
            .clone()
            .unwrap_or_else(|| SYSTEM_PROMPT.to_string());
        self.messages.push(Message::system(&system_prompt));
        for (name, content) in &self.project_files.clone() {
            self.inject_context(name, content);
        }
        self.messages.push(Message::user(format!(
            "[Context Summary — {} messages compacted]\n{}",
            old_count, resp.text
        )));
        self.messages.push(Message::assistant("Summary loaded."));
        self.messages.extend(last_turns);

        let _ = event_tx.send(AgentEvent::Compacted(format!(
            "♻ Context compacted — {} messages → summary",
            old_count
        )));
    }
}
```

- [ ] **Step 4 — Call `compact_history()` after each Done in `run()`**

In `src/agent.rs`, at the end of the agent loop before `event_tx.send(AgentEvent::Done)`:

```rust
// Auto-compact if context is getting large.
let ctx_size: usize = self.messages.iter()
    .map(|m| m.content.len())
    .sum();
if config.compact_threshold > 0 && ctx_size > config.compact_threshold {
    self.compact_history(provider.as_ref(), &event_tx).await;
}

let _ = event_tx.send(AgentEvent::Done { tokens_in: total_in, tokens_out: total_out });
```

- [ ] **Step 5 — Handle `Compacted` event in App**

In `src/tui/app.rs`, add to `handle_agent_event`:

```rust
AgentEvent::Compacted(msg) => {
    self.messages.push(ChatMessage::System(msg));
}
```

- [ ] **Step 6 — Build**
```bash
cargo build --release 2>&1 | grep -E "(error|warning)" && hash -r
```

- [ ] **Step 7 — Commit**
```bash
git add src/config.rs src/agent.rs src/tui/app.rs
git commit -m "feat: auto-compact context when it exceeds threshold"
```

---

## Final Verification

- [ ] Run full build clean
```bash
cargo build --release 2>&1 | grep -E "(error|warning)"
hash -r
```

- [ ] Smoke-test checklist (manual, requires running tycode):
  - [ ] Type a prompt — response streams live, cursor `▊` visible
  - [ ] Scroll up mid-stream — view stays put, doesn't snap
  - [ ] Shift+Enter creates newline in input box
  - [ ] Input height grows with content, resets on send
  - [ ] Ctrl+C while idle shows "again to quit" hint
  - [ ] Two Ctrl+C within 2s quits
  - [ ] Create TYCODE.md in cwd, restart — banner shows "Loaded: TYCODE.md"
  - [ ] `/clear` — agent still responds correctly (context preserved)
  - [ ] Run a bash command with `rm -rf /tmp/tycode_test` — confirm overlay appears
  - [ ] Press N — agent receives denial, continues
  - [ ] Status bar shows `ctx~Xk  ↑Yk↓Zk  Nmsgs` after first response

- [ ] Final commit if any fixups needed
```bash
git add -A && git commit -m "fix: post-integration cleanup"
```
