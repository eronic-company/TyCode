# TyCode — Claude Code Parity Design

**Date:** 2026-05-10  
**Status:** Approved  
**Scope:** 8 features to bring TyCode to Claude Code quality and best-practice parity

---

## Overview

TyCode is a Rust TUI agent powered by any LLM provider. This spec covers the gap between its current state and Claude Code's day-to-day UX: input ergonomics, safety guardrails, project awareness, context management, and live output streaming.

---

## Feature 1 — Ctrl+C: Cancel vs Quit

### Behaviour
| Situation | First Ctrl+C | Second Ctrl+C (≤2s later) |
|-----------|-------------|--------------------------|
| Processing | Cancel queue; agent finishes current iteration | Quit |
| Idle | Clear input; show hint in status bar | Quit |

### Implementation
- Add `last_ctrl_c: Option<Instant>` to `App`.
- In the global Ctrl+C handler (top of `handle_key`), check `mode` and elapsed time:
  - Processing + first press → clear `input_queue`, set status "Ctrl+C again to quit"
  - Idle + first press → clear `input`, set status "Ctrl+C again to quit"  
  - Second press within 2 s → `app.should_quit = true`
- The agent task is **not** aborted mid-tool; it completes its current iteration and then finds no pending prompt, so it idles naturally. This matches Claude Code: you cancel future work, not in-flight work.

### Files
- `src/tui/app.rs` — add field
- `src/tui/input.rs` — update Ctrl+C handler

---

## Feature 2 — Multiline Input (Shift+Enter)

### Behaviour
- `Shift+Enter` inserts `\n` at cursor position.
- Input remains a flat `String`; newlines live inside it.
- `Enter` submits the full string (including newlines) as-is.
- When the string contains `\n`, the input border title shows a dim badge: `3 lines`.
- All existing cursor movement, word deletion, and history still work unchanged.

### Rendering
- `Paragraph::new(text).wrap(Wrap { trim: false })` already renders `\n` correctly — no change needed in the widget layer.
- Cursor `x/y` calculation needs to account for physical line breaks: walk the string up to `cursor_pos`, count `\n` characters to get row, use remainder for column.

### Files
- `src/tui/input.rs` — add `Shift+Enter` → insert `\n`
- `src/tui/ui.rs` — cursor x/y calculation update; line-count badge in title

---

## Feature 3 — TYCODE.md + README Auto-Inject

### Behaviour
- On startup, before the TUI loop, scan `cwd` for `TYCODE.md` then `README.md`.
- Each file found is read and injected via `agent.inject_context(filename, content)` — the same mechanism used by `/import`.
- If a file exceeds 100 KB it is skipped with a warning (same limit as `/import`).
- The startup banner updates to: `Loaded: TYCODE.md · README.md` (or just the ones found).
- `/clear` and `/cache` both re-inject whatever was found at startup, so project context is never lost on reset.

### Agent side
- `Agent` grows a `project_files: Vec<(String, String)>` field populated at startup.
- `Agent::reinject_project_files()` replays them after `clear_history()`.

### Files
- `src/agent.rs` — add `project_files` field, `inject_project_files()`, `reinject_project_files()`
- `src/main.rs` — call `inject_project_files(cwd)` before entering TUI loop; update banner
- `src/tui/input.rs` — `/clear` and `/cache` call `reinject_project_files()` after clearing

---

## Feature 4 — Dangerous Command Confirmation

### Dangerous patterns (checked before `bash` executes)
```
rm -rf, rm -fr, rm --recursive --force
dd if=
mkfs
DROP TABLE, DROP DATABASE (SQL)
:(){:|:&};: (fork bomb)
git push --force / git push -f
git reset --hard
chmod -R 777
sudo rm, sudo dd
truncate
```

### Flow
1. `shell::bash_execute` extracts the command string and runs it through `is_dangerous(cmd) -> Option<&str>` (returns the matched reason string).
2. If dangerous, the tool **does not execute**. Instead it returns a special sentinel `ToolResult` with `needs_confirm: true` and the reason.
3. The agent layer checks for `needs_confirm` and emits `AgentEvent::NeedConfirmation { tool, command, reason, tx: oneshot::Sender<bool> }`.
4. The main loop receives this event, sets `app.mode = AppMode::Confirm(ConfirmState { ... })`, and suspends agent-event processing until the sender is consumed.
5. A blocking overlay renders: command in a box, reason highlighted in amber, `[Y] Allow  [N] Deny` footer.
6. `Y`/`Enter` sends `true`; `N`/`Esc` sends `false`. Mode returns to `Processing`.
7. Agent receives the bool. If `true`, re-runs the command for real. If `false`, returns `ToolResult::err("User denied: <reason>")` and continues.

### Files
- `src/tools/shell.rs` — add `is_dangerous()`, sentinel result
- `src/agent.rs` — handle sentinel in the tool result loop; emit `NeedConfirmation`
- `src/tui/app.rs` — `AppMode::Confirm`, `ConfirmState`, `AgentEvent::NeedConfirmation`
- `src/tui/input.rs` — handle `AppMode::Confirm` keys (Y/N/Enter/Esc)
- `src/tui/ui.rs` — render confirmation overlay

---

## Feature 5 — Live Streaming Output

### Problem
Currently all agent text is buffered in `buffered_response` and revealed only on `AgentEvent::Done`. This hides output for long responses and makes the agent look frozen.

### Design
- Remove the buffer-until-done pattern. Each `AgentEvent::TextDelta` is appended directly to a **live response** entry in `messages`.
- A new message variant `ChatMessage::AssistantLive(String)` holds the in-progress text. On `Done`, it is converted to `ChatMessage::AssistantText(String)`.
- Tool calls and their results continue to be appended normally (they were never buffered).
- The chat widget re-renders on every tick (already the case) so the user sees each delta as it arrives.
- `scroll_to_bottom()` is called on each `TextDelta` if the user was already at the bottom (auto-follow), but not if they have scrolled up (don't hijack their position).

### Auto-follow logic
- Add `auto_scroll: bool` to `App` (default `true`).
- When the user manually scrolls up → set `auto_scroll = false`.
- When a new message arrives (Done, queued) → set `auto_scroll = true`.
- `scroll_to_bottom()` is called in the render loop only when `auto_scroll == true`.

### Files
- `src/tui/app.rs` — `ChatMessage::AssistantLive`, `auto_scroll` field, updated `handle_agent_event`
- `src/tui/ui.rs` — render `AssistantLive` with a trailing cursor `▊` to show it's live; auto-scroll logic

---

## Feature 6 — Real Token Usage per Turn

### Data source
Provider API responses include token counts. Each provider reports them differently:

| Provider | Field |
|----------|-------|
| Anthropic | `usage.input_tokens`, `usage.output_tokens` |
| OpenAI | `usage.prompt_tokens`, `usage.completion_tokens` |
| Ollama | `prompt_eval_count`, `eval_count` |
| Gemini | `usageMetadata.promptTokenCount`, `usageMetadata.candidatesTokenCount` |

### Design
- Add `TokenUsage { input: u32, output: u32 }` struct to `provider/mod.rs`.
- Each provider parses its response and populates `TokenUsage` inside `ProviderResponse`.
- `AgentEvent::Done` carries a `Vec<TokenUsage>` (one entry per iteration).
- `App` accumulates `session_tokens_in: u32` and `session_tokens_out: u32` across the session, plus `last_turn_tokens: TokenUsage` for the most recent turn.

### Display
- Status bar right side: `↑1.2K ↓3.4K tok · 12 msgs` (last turn in/out, total message count).
- On hover or via `/tokens` command: show session totals.

### Files
- `src/provider/mod.rs` — `TokenUsage`, extend `ProviderResponse`
- `src/provider/anthropic.rs`, `openai.rs`, `ollama.rs`, `gemini.rs` — parse and populate
- `src/agent.rs` — accumulate and emit in `AgentEvent::Done`
- `src/tui/app.rs` — store token fields; update on Done
- `src/tui/ui.rs` — render in status bar

---

## Feature 7 — Token/Message Counter in Status Bar (Estimated)

While Feature 6 shows real per-turn token usage, the status bar also needs a running estimate for tokens already in context (the full conversation, not just the last turn).

- `App::context_token_estimate()` → `messages.iter().map(content_char_len).sum() / 4`.
- Shown alongside real usage: `ctx ~18K | ↑1.2K ↓3.4K | 12 msgs`.
- Colour-coded: green < 60K, amber 60–90K, red > 90K (approaching most model limits).

### Files
- `src/tui/app.rs` — `context_token_estimate()` method
- `src/tui/ui.rs` — status bar rendering with colour thresholds

---

## Feature 8 — Auto-Compact Context

### Trigger
After each `AgentEvent::Done`, if `context_token_estimate() > compact_threshold` (default: 80 000 chars ≈ 20K tokens), auto-compact fires.

### Algorithm
1. Build a compaction prompt from the full message history (system prompt excluded).
2. Send a single-turn request to the provider: `"Summarise this conversation in full detail. Preserve all decisions, code changes, file paths, commands run, and their outputs. Be thorough."`
3. On response, replace all messages between the system prompt and the last 4 user/assistant turns with a single `Message::user("[Context summary]\n<summary text>")` + `Message::assistant("Summary loaded.")`.
4. Push `ChatMessage::System("♻ Context compacted — N messages → summary (~Xk tokens freed)")` to the display.
5. Recompute `context_token_estimate`.

### Config
```json
{
  "compact_threshold": 80000
}
```

Power users can raise this (e.g., for models with 128K+ context) or set it to `0` to disable.

### Files
- `src/config.rs` — add `compact_threshold: usize`
- `src/agent.rs` — `compact_history()` method; call from `run()` after Done
- `src/tui/app.rs` — new `AgentEvent::Compacting` / `AgentEvent::Compacted(String)` for display

---

## Unchanged / Out of Scope

- MCP (Model Context Protocol) — separate project
- Image / screenshot input — requires provider support per model
- Cost tracking in dollars — needs per-model pricing table, deferred
- `/undo` — deferred

---

## Implementation Order

Features should be implemented in this order to avoid merge conflicts and allow early testing:

1. **Feature 5** (streaming) — highest user-visible impact, touches output path
2. **Feature 2** (multiline input) — input path only, no conflicts
3. **Feature 1** (Ctrl+C cancel) — input handler tweak
4. **Feature 3** (TYCODE.md inject) — startup + agent
5. **Feature 6** (real token usage) — provider layer + display
6. **Feature 7** (context estimate counter) — display only
7. **Feature 4** (dangerous command confirm) — agent + overlay
8. **Feature 8** (auto-compact) — agent + config

---

## Testing Checklist

- [ ] Ctrl+C while idle shows hint, second Ctrl+C quits
- [ ] Ctrl+C while processing cancels queue, agent completes current turn
- [ ] Shift+Enter inserts newline; Enter submits multiline prompt
- [ ] Line-count badge appears when input has >1 line
- [ ] TYCODE.md and README.md appear in agent context on first message
- [ ] `/clear` and `/cache` preserve project file context
- [ ] `rm -rf /tmp/test` triggers confirmation overlay
- [ ] Denying confirmation returns error to agent, agent continues
- [ ] Output streams live character-by-character during generation
- [ ] Scrolling up while streaming does not snap back to bottom
- [ ] Token counts appear in status bar after each turn
- [ ] Status bar colour shifts amber at 60K, red at 90K
- [ ] Auto-compact fires when context exceeds threshold
- [ ] Post-compact agent still answers correctly using summary
