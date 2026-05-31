# Multi-Key Support & Provider-Defined Defaults Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Allow up to 100 labelled API keys per provider with a runtime key-picker, and have each provider supply its own max-tokens / max-iterations defaults (hidden from the settings UI for API providers).

**Architecture:** Config gains `api_keys` and `active_key` maps alongside the legacy single-key fields (backward-compatible). The `Provider` trait gets two new defaulted methods; Ollama reads values from config while API providers return hardcoded defaults. Two new TUI modes (`KeySelect`, `KeyManage`) follow the existing `ProviderSelect`/`ModelSelect` patterns exactly.

**Tech Stack:** Rust, Ratatui (TUI), Serde/serde_json (config persistence), `std::collections::HashMap`

---

## File Map

| File | Change |
|------|--------|
| `src/config.rs` | Add `ApiKeyEntry`, `api_keys`, `active_key`; helpers `resolve_api_key`, `needs_key_select`, `is_local_provider` |
| `src/provider/mod.rs` | Add `default_max_tokens`, `default_max_iterations` to trait; add `default_max_iterations_for` free fn |
| `src/provider/anthropic.rs` | Override trait methods (hardcoded 8192 / 100) |
| `src/provider/openai.rs` | Override trait methods (hardcoded 4096 / 100) |
| `src/provider/gemini.rs` | Override trait methods (hardcoded 8192 / 100) |
| `src/provider/ollama.rs` | Add `max_tokens`/`max_iterations` fields; `new_with_limits`; override trait methods |
| `src/agent.rs` | Replace `config.max_iterations` with `provider::default_max_iterations_for(config)` |
| `src/tui/app.rs` | Add `KeySelect`/`KeyManage` `AppMode` variants + state types; update `SettingsState` |
| `src/tui/ui.rs` | Render `KeySelect` and `KeyManage` overlays; update render dispatch |
| `src/tui/input.rs` | Handle `KeySelect` and `KeyManage` key events; wire triggers |

---

## Task 1: Config data model + helpers

**Files:**
- Modify: `src/config.rs`

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_local_provider() {
        assert!(is_local_provider("ollama"));
        assert!(is_local_provider("gguf"));
        assert!(!is_local_provider("anthropic"));
        assert!(!is_local_provider("openai"));
        assert!(!is_local_provider("gemini"));
    }

    #[test]
    fn test_resolve_api_key_falls_back_to_legacy() {
        let config = Config { anthropic_api_key: "legacy".into(), ..Config::default() };
        assert_eq!(config.resolve_api_key("anthropic"), "legacy");
    }

    #[test]
    fn test_resolve_api_key_uses_active_label() {
        let mut config = Config::default();
        config.api_keys.insert("anthropic".into(), vec![
            ApiKeyEntry { label: "work".into(), key: "k-work".into() },
            ApiKeyEntry { label: "personal".into(), key: "k-personal".into() },
        ]);
        config.active_key.insert("anthropic".into(), "personal".into());
        assert_eq!(config.resolve_api_key("anthropic"), "k-personal");
    }

    #[test]
    fn test_resolve_api_key_defaults_to_first_when_no_active() {
        let mut config = Config::default();
        config.api_keys.insert("anthropic".into(), vec![
            ApiKeyEntry { label: "first".into(), key: "k-first".into() },
            ApiKeyEntry { label: "second".into(), key: "k-second".into() },
        ]);
        assert_eq!(config.resolve_api_key("anthropic"), "k-first");
    }

    #[test]
    fn test_needs_key_select_true_for_multiple() {
        let mut config = Config::default();
        config.api_keys.insert("openai".into(), vec![
            ApiKeyEntry { label: "a".into(), key: "k1".into() },
            ApiKeyEntry { label: "b".into(), key: "k2".into() },
        ]);
        assert!(config.needs_key_select("openai"));
    }

    #[test]
    fn test_needs_key_select_false_for_one() {
        let mut config = Config::default();
        config.api_keys.insert("openai".into(), vec![
            ApiKeyEntry { label: "a".into(), key: "k1".into() },
        ]);
        assert!(!config.needs_key_select("openai"));
    }

    #[test]
    fn test_config_roundtrip_with_api_keys() {
        let mut config = Config::default();
        config.api_keys.insert("gemini".into(), vec![
            ApiKeyEntry { label: "prod".into(), key: "sk-prod".into() },
        ]);
        config.active_key.insert("gemini".into(), "prod".into());
        let json = serde_json::to_string(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.api_keys["gemini"][0].label, "prod");
        assert_eq!(loaded.active_key["gemini"], "prod");
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test 2>&1 | grep -E "FAILED|error|not found"
```

Expected: compile errors — `ApiKeyEntry`, `api_keys`, `is_local_provider`, `resolve_api_key`, `needs_key_select` not yet defined.

- [ ] **Step 3: Add `ApiKeyEntry` and new `Config` fields**

In `src/config.rs`, add after the `use` lines and before `fn config_dir`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyEntry {
    pub label: String,
    pub key: String,
}
```

Add to `Config` struct after `google_api_key`:

```rust
    #[serde(default)]
    pub api_keys: std::collections::HashMap<String, Vec<ApiKeyEntry>>,
    #[serde(default)]
    pub active_key: std::collections::HashMap<String, String>,
```

Add to `Config::default()` after `google_api_key: String::new()`:

```rust
            api_keys: std::collections::HashMap::new(),
            active_key: std::collections::HashMap::new(),
```

- [ ] **Step 4: Add helper functions**

Add after `impl Config { ... }` closing brace (as free functions in the module):

```rust
pub fn is_local_provider(name: &str) -> bool {
    matches!(name, "ollama" | "gguf")
}
```

Add inside `impl Config`:

```rust
    /// Return the API key to use for `provider`, honouring the multi-key store.
    /// Falls back to legacy single-key fields if no multi-key entries exist.
    pub fn resolve_api_key(&self, provider: &str) -> String {
        if let Some(entries) = self.api_keys.get(provider) {
            if !entries.is_empty() {
                if let Some(label) = self.active_key.get(provider) {
                    if let Some(e) = entries.iter().find(|e| &e.label == label) {
                        return e.key.clone();
                    }
                }
                return entries[0].key.clone();
            }
        }
        match provider {
            "anthropic" => self.anthropic_api_key.clone(),
            "openai"    => self.openai_api_key.clone(),
            "gemini"    => self.google_api_key.clone(),
            _           => String::new(),
        }
    }

    /// True when this provider has more than one registered key and a prompt is needed.
    pub fn needs_key_select(&self, provider: &str) -> bool {
        self.api_keys.get(provider).map(|v| v.len() > 1).unwrap_or(false)
    }
```

- [ ] **Step 5: Run tests — expect all to pass**

```bash
cargo test config::tests 2>&1
```

Expected output: `7 passed; 0 failed`

- [ ] **Step 6: Commit**

```bash
tiv add src/config.rs
tiv com "feat(config): add ApiKeyEntry, api_keys/active_key fields, key resolution helpers"
```

---

## Task 2: Provider trait default methods + per-provider overrides

**Files:**
- Modify: `src/provider/mod.rs`
- Modify: `src/provider/anthropic.rs`
- Modify: `src/provider/openai.rs`
- Modify: `src/provider/gemini.rs`
- Modify: `src/provider/ollama.rs`

- [ ] **Step 1: Write failing tests**

Add at the bottom of `src/provider/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn test_default_max_iterations_for_api_provider() {
        let config = Config { provider: "anthropic".into(), ..Config::default() };
        assert_eq!(default_max_iterations_for(&config), 100);
    }

    #[test]
    fn test_default_max_iterations_for_ollama_reads_config() {
        let config = Config { provider: "ollama".into(), max_iterations: 42, ..Config::default() };
        assert_eq!(default_max_iterations_for(&config), 42);
    }

    #[test]
    fn test_default_max_iterations_for_gguf_reads_config() {
        let config = Config { provider: "gguf".into(), max_iterations: 7, ..Config::default() };
        assert_eq!(default_max_iterations_for(&config), 7);
    }
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cargo test provider::tests 2>&1 | grep -E "FAILED|error"
```

Expected: `default_max_iterations_for` not found.

- [ ] **Step 3: Add trait methods to `Provider` in `src/provider/mod.rs`**

Inside the `pub trait Provider` block, after `fn name(&self) -> &str;`, add:

```rust
    fn default_max_tokens(&self) -> u32 { 8192 }
    fn default_max_iterations(&self) -> u32 { 100 }
```

- [ ] **Step 4: Add `default_max_iterations_for` free function to `src/provider/mod.rs`**

Add after the `create_provider` function:

```rust
/// Returns the appropriate max-iterations for this config's provider.
/// API providers use their own hardcoded default; local providers use config.
pub fn default_max_iterations_for(config: &Config) -> u32 {
    if crate::config::is_local_provider(&config.provider) {
        config.max_iterations
    } else {
        100
    }
}
```

- [ ] **Step 5: Override in `src/provider/anthropic.rs`**

Add inside `impl Provider for AnthropicProvider` (after `fn name`):

```rust
    fn default_max_tokens(&self) -> u32 { 8192 }
    fn default_max_iterations(&self) -> u32 { 100 }
```

- [ ] **Step 6: Override in `src/provider/openai.rs`**

Add inside `impl Provider for OpenAIProvider` (after `fn name`):

```rust
    fn default_max_tokens(&self) -> u32 { 4096 }
    fn default_max_iterations(&self) -> u32 { 100 }
```

- [ ] **Step 7: Override in `src/provider/gemini.rs`**

Add inside `impl Provider for GeminiProvider` (after `fn name`):

```rust
    fn default_max_tokens(&self) -> u32 { 8192 }
    fn default_max_iterations(&self) -> u32 { 100 }
```

- [ ] **Step 8: Update `src/provider/ollama.rs` — add limits fields and new constructor**

Change the `OllamaProvider` struct definition from:

```rust
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: Client,
}
```

to:

```rust
pub struct OllamaProvider {
    model: String,
    base_url: String,
    client: Client,
    max_tokens: u32,
    max_iterations: u32,
}
```

Change `OllamaProvider::new` to call a new `new_with_limits` function:

```rust
    pub fn new(model: &str, base_url: &str) -> Self {
        Self::new_with_limits(model, base_url, 8192, 100)
    }

    pub fn new_with_limits(model: &str, base_url: &str, max_tokens: u32, max_iterations: u32) -> Self {
        Self {
            model: model.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .build()
                .unwrap_or_else(|_| Client::new()),
            max_tokens,
            max_iterations,
        }
    }
```

Add inside `impl Provider for OllamaProvider` (after `fn name`):

```rust
    fn default_max_tokens(&self) -> u32 { self.max_tokens }
    fn default_max_iterations(&self) -> u32 { self.max_iterations }
```

- [ ] **Step 9: Run tests**

```bash
cargo test provider::tests 2>&1
```

Expected: `3 passed; 0 failed`

- [ ] **Step 10: Confirm project still compiles**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 11: Commit**

```bash
tiv add src/provider/mod.rs src/provider/anthropic.rs src/provider/openai.rs src/provider/gemini.rs src/provider/ollama.rs
tiv com "feat(provider): add default_max_tokens/default_max_iterations trait methods; Ollama new_with_limits"
```

---

## Task 3: Provider factory — use resolved keys + provider-defined max tokens

**Files:**
- Modify: `src/provider/mod.rs`

- [ ] **Step 1: Update `create_provider` to use `config.resolve_api_key` and hardcoded max_tokens**

Find the `create_provider` function in `src/provider/mod.rs`. Replace the entire function body:

```rust
pub fn create_provider(config: &Config) -> Result<Box<dyn Provider>> {
    match config.provider.to_lowercase().as_str() {
        "anthropic" => Ok(Box::new(anthropic::AnthropicProvider::new(
            &config.model,
            &config.resolve_api_key("anthropic"),
            8192,
        ))),
        "openai" => Ok(Box::new(openai::OpenAIProvider::new(
            &config.model,
            &config.resolve_api_key("openai"),
            &config.openai_base_url,
            4096,
        ))),
        "ollama" => Ok(Box::new(ollama::OllamaProvider::new_with_limits(
            &config.model,
            &config.ollama_url,
            config.max_tokens,
            config.max_iterations,
        ))),
        "gemini" => Ok(Box::new(gemini::GeminiProvider::new(
            &config.model,
            &config.resolve_api_key("gemini"),
            8192,
        ))),
        other => anyhow::bail!("Unknown provider '{other}'. Options: anthropic, openai, ollama, gemini"),
    }
}
```

- [ ] **Step 2: Build to confirm no compile errors**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
tiv add src/provider/mod.rs
tiv com "refactor(provider): factory uses resolved API keys and provider-defined max_tokens"
```

---

## Task 4: Agent — use provider-defined max iterations

**Files:**
- Modify: `src/agent.rs`

- [ ] **Step 1: Replace `config.max_iterations` in the agent run loop**

In `src/agent.rs`, find line:

```rust
        let max_iterations = config.max_iterations;
```

Replace with:

```rust
        let max_iterations = crate::provider::default_max_iterations_for(config);
```

- [ ] **Step 2: Build to confirm no compile errors**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 3: Commit**

```bash
tiv add src/agent.rs
tiv com "feat(agent): use provider-defined max_iterations instead of global config"
```

---

## Task 5: TUI app.rs — new AppMode variants, state types, updated SettingsState

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Add `KeySelectAfter` enum and `KeySelectState` struct**

In `src/tui/app.rs`, after the `ProviderSelectState` impl block (around line 175), add:

```rust
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
```

- [ ] **Step 2: Add `AddKeyStep` enum and `KeyManageState` struct**

Directly after the `KeySelectState` impl block, add:

```rust
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
```

- [ ] **Step 3: Add new variants to `AppMode`**

Find the `AppMode` enum:

```rust
pub enum AppMode {
    Normal,
    Processing,
    Settings(SettingsState),
    Help,
    ModelSelect(ModelSelectState),
    ProviderSelect(ProviderSelectState),
    Confirm(ConfirmState),
}
```

Replace with:

```rust
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
```

- [ ] **Step 4: Update `SettingsState::from_config` to be provider-aware**

Replace the entire `from_config` method body:

```rust
    pub fn from_config(config: &Config) -> Self {
        use crate::config::is_local_provider;

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
```

- [ ] **Step 5: Build to confirm no compile errors**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 6: Commit**

```bash
tiv add src/tui/app.rs
tiv com "feat(tui/app): add KeySelect/KeyManage modes; provider-aware SettingsState"
```

---

## Task 6: TUI ui.rs — render KeySelect and KeyManage overlays

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Update the import line at the top of ui.rs**

Find:

```rust
use super::app::{App, AppMode, ChatMessage, ConfirmState, ModelSelectState, ProviderSelectState, SettingsState};
```

Replace with:

```rust
use super::app::{App, AppMode, ChatMessage, ConfirmState, KeyManageState, KeySelectState, ModelSelectState, ProviderSelectState, SettingsState};
```

- [ ] **Step 2: Add the two new modes to the overlay dispatch**

Find:

```rust
        AppMode::Settings(state) => render_settings_overlay(f, state.clone(), size),
        AppMode::Help => render_help_overlay(f, size),
        AppMode::ModelSelect(state) => render_model_select_overlay(f, state.clone(), size),
        AppMode::ProviderSelect(state) => render_provider_select_overlay(f, state.clone(), size),
```

Replace with:

```rust
        AppMode::Settings(state) => render_settings_overlay(f, state.clone(), size),
        AppMode::Help => render_help_overlay(f, size),
        AppMode::ModelSelect(state) => render_model_select_overlay(f, state.clone(), size),
        AppMode::ProviderSelect(state) => render_provider_select_overlay(f, state.clone(), size),
        AppMode::KeySelect(state) => render_key_select_overlay(f, state.clone(), size),
        AppMode::KeyManage(state) => render_key_manage_overlay(f, state.clone(), size),
```

- [ ] **Step 3: Fix the `is_overlay_open` check in input.rs to include the new modes**

In `src/tui/input.rs`, find `fn is_overlay_open`:

```rust
fn is_overlay_open(mode: &AppMode) -> bool {
    matches!(
        mode,
        AppMode::Help
            | AppMode::Settings(_)
            | AppMode::ModelSelect(_)
            | AppMode::ProviderSelect(_)
    )
}
```

Replace with:

```rust
fn is_overlay_open(mode: &AppMode) -> bool {
    matches!(
        mode,
        AppMode::Help
            | AppMode::Settings(_)
            | AppMode::ModelSelect(_)
            | AppMode::ProviderSelect(_)
            | AppMode::KeySelect(_)
            | AppMode::KeyManage(_)
    )
}
```

- [ ] **Step 4: Fix the masking logic in `render_settings_overlay`**

The existing masking check `field.key.contains("key")` would incorrectly mask the `manage_keys_*` button fields. Find in `render_settings_overlay`:

```rust
        let value_display = if field.key == "provider" {
            format!("{}  ◀▶", field.value)
        } else if field.key.contains("key") && !field.value.is_empty() && !(state.editing && is_selected) {
            let visible = field.value.len().min(4);
            format!("{}...", &field.value[..visible])
        } else {
            field.value.clone()
        };
```

Replace with:

```rust
        let is_api_key_field = (field.key.ends_with("_api_key") || field.key == "google_api_key")
            && !field.key.starts_with("manage_keys");
        let value_display = if field.key == "provider" {
            format!("{}  ◀▶", field.value)
        } else if field.key.starts_with("manage_keys") {
            format!("{}  →", field.value)
        } else if is_api_key_field && !field.value.is_empty() && !(state.editing && is_selected) {
            let visible = field.value.len().min(4);
            format!("{}...", &field.value[..visible])
        } else {
            field.value.clone()
        };
```

- [ ] **Step 5: Add `render_key_select_overlay` function**

Add before the `// ── Helpers ──` section at the bottom of `src/tui/ui.rs`:

```rust
// ── Key select overlay ───────────────────────────────────────────────────────

fn render_key_select_overlay(f: &mut Frame, state: KeySelectState, area: Rect) {
    let width = 52u16.min(area.width.saturating_sub(4));
    let height = (state.entries.len() as u16 + 6)
        .min(area.height.saturating_sub(4))
        .max(8);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let title = format!(" 🔑  Select Key — {} ", state.provider);
    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(Line::from(vec![
            Span::styled(" ↑↓", Style::default().fg(Color::Yellow)),
            Span::styled("navigate  ", Style::default().fg(Color::DarkGray)),
            Span::styled("↵", Style::default().fg(Color::Yellow)),
            Span::styled("select  ", Style::default().fg(Color::DarkGray)),
            Span::styled("esc", Style::default().fg(Color::Yellow)),
            Span::styled("=cancel ", Style::default().fg(Color::DarkGray)),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let items: Vec<ListItem> = state.entries.iter().enumerate().map(|(i, entry)| {
        let masked = format!("  ••••••••  ({})", entry.label);
        if i == state.selected {
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("► {}", masked.trim_start()),
                    Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD),
                ),
            ]))
        } else {
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {}", masked.trim_start()), Style::default().fg(Color::Rgb(180, 180, 220))),
            ]))
        }
    }).collect();

    let list = List::new(items).block(block).style(Style::default().bg(POPUP_BG));
    f.render_widget(list, popup_area);
}
```

- [ ] **Step 6: Add `render_key_manage_overlay` function**

Add directly after `render_key_select_overlay`:

```rust
// ── Key manage overlay ───────────────────────────────────────────────────────

fn render_key_manage_overlay(f: &mut Frame, state: KeyManageState, area: Rect) {
    let width = 60u16.min(area.width.saturating_sub(4));
    let content_rows = (state.entries.len() as u16 + 2).max(4);
    let height = (content_rows + 8).min(area.height.saturating_sub(4)).max(10);
    let x = (area.width - width) / 2;
    let y = (area.height - height) / 2;
    let popup_area = Rect::new(x, y, width, height);

    f.render_widget(Clear, popup_area);

    let (title, hint_bottom) = match &state.add_step {
        None => (
            format!(" 🗝  Keys — {} ", state.provider),
            Line::from(vec![
                Span::styled(" n", Style::default().fg(Color::Yellow)),
                Span::styled("=add  ", Style::default().fg(Color::DarkGray)),
                Span::styled("d", Style::default().fg(Color::Yellow)),
                Span::styled("=delete  ", Style::default().fg(Color::DarkGray)),
                Span::styled("↵", Style::default().fg(Color::Yellow)),
                Span::styled("=activate  ", Style::default().fg(Color::DarkGray)),
                Span::styled("esc", Style::default().fg(Color::Yellow)),
                Span::styled("=back ", Style::default().fg(Color::DarkGray)),
            ]),
        ),
        Some(AddKeyStep::EnterLabel(_)) => (
            format!(" 🗝  Add Key — {} ", state.provider),
            Line::from(vec![
                Span::styled(" Enter label for this key (e.g. work, personal)", Style::default().fg(Color::DarkGray)),
            ]),
        ),
        Some(AddKeyStep::EnterKey { .. }) => (
            format!(" 🗝  Add Key — {} ", state.provider),
            Line::from(vec![
                Span::styled(" Paste or type the API key value", Style::default().fg(Color::DarkGray)),
            ]),
        ),
    };

    let block = Block::default()
        .title(title)
        .title_style(Style::default().fg(POPUP_TITLE).add_modifier(Modifier::BOLD))
        .title_bottom(hint_bottom)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(POPUP_BORDER))
        .padding(Padding::new(1, 1, 1, 1))
        .style(Style::default().bg(POPUP_BG));

    let inner = block.inner(popup_area);
    f.render_widget(block, popup_area);

    let mut lines: Vec<Line<'static>> = Vec::new();

    match &state.add_step {
        None => {
            if state.entries.is_empty() {
                lines.push(Line::from(Span::styled(
                    "  No keys registered. Press 'n' to add one.",
                    Style::default().fg(Color::DarkGray),
                )));
            } else {
                for (i, entry) in state.entries.iter().enumerate() {
                    let cursor = if i == state.selected { "► " } else { "  " };
                    let style = if i == state.selected {
                        Style::default().fg(Color::White).bg(Color::Rgb(80, 120, 220)).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::Rgb(180, 180, 220))
                    };
                    lines.push(Line::from(Span::styled(
                        format!("{}[{:<20}]  ••••••••", cursor, entry.label),
                        style,
                    )));
                }
            }
        }
        Some(AddKeyStep::EnterLabel(buf)) => {
            lines.push(Line::from(vec![
                Span::styled("  Label: ", Style::default().fg(Color::Rgb(130, 180, 255))),
                Span::styled(
                    format!("{}_", buf),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
                ),
            ]));
        }
        Some(AddKeyStep::EnterKey { label, key_buf }) => {
            lines.push(Line::from(vec![
                Span::styled(format!("  Label: {label}"), Style::default().fg(Color::Rgb(100, 220, 130))),
            ]));
            lines.push(Line::from(vec![
                Span::styled("  Key:   ", Style::default().fg(Color::Rgb(130, 180, 255))),
                Span::styled(
                    format!("{}_", "•".repeat(key_buf.len())),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::UNDERLINED),
                ),
            ]));
        }
    }

    let para = Paragraph::new(lines).style(Style::default().bg(POPUP_BG));
    f.render_widget(para, inner);
}
```

- [ ] **Step 7: Build to confirm no compile errors**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 8: Commit**

```bash
tiv add src/tui/ui.rs src/tui/input.rs
tiv com "feat(tui/ui): render KeySelect and KeyManage overlays; fix settings mask logic"
```

---

## Task 7: TUI input.rs — handle KeySelect and KeyManage modes + wire triggers

**Files:**
- Modify: `src/tui/input.rs`
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Update imports at top of `src/tui/input.rs`**

Find:

```rust
use super::app::{App, AppMode, ChatMessage, ModelSelectState, ProviderSelectState, SettingsState};
```

Replace with:

```rust
use super::app::{
    AddKeyStep, App, AppMode, ChatMessage, KeyManageState, KeySelectAfter, KeySelectState,
    ModelSelectState, ProviderSelectState, SettingsState,
};
```

- [ ] **Step 2: Add `KeySelect` and `KeyManage` to the main key dispatch**

Find:

```rust
        AppMode::Confirm(_) => {
            handle_confirm_key(app, key);
            true
        }
        AppMode::Normal => handle_normal_key(app, key, agent_tx).await,
```

Replace with:

```rust
        AppMode::Confirm(_) => {
            handle_confirm_key(app, key);
            true
        }
        AppMode::KeySelect(_) => {
            handle_key_select_key(app, key);
            true
        }
        AppMode::KeyManage(_) => {
            handle_key_manage_key(app, key);
            true
        }
        AppMode::Normal => handle_normal_key(app, key, agent_tx).await,
```

- [ ] **Step 3: Add `handle_key_select_key` function**

Add after the `handle_provider_select_key` function:

```rust
fn handle_key_select_key(app: &mut App, key: KeyEvent) {
    let state = match &app.mode {
        AppMode::KeySelect(s) => s.clone(),
        _ => return,
    };

    match key.code {
        KeyCode::Esc => {
            // No key was chosen — default to first if none active
            if !app.config.active_key.contains_key(&state.provider) && !state.entries.is_empty() {
                app.config.active_key.insert(state.provider.clone(), state.entries[0].label.clone());
                let _ = app.config.save();
            }
            match state.after {
                KeySelectAfter::ReturnToSettings => {
                    app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
                }
                KeySelectAfter::Normal => {
                    app.mode = AppMode::Normal;
                }
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::KeySelect(s) = &mut app.mode {
                if s.selected > 0 { s.selected -= 1; }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::KeySelect(s) = &mut app.mode {
                if s.selected < s.entries.len().saturating_sub(1) { s.selected += 1; }
            }
        }
        KeyCode::Enter => {
            if let Some(entry) = state.entries.get(state.selected) {
                app.config.active_key.insert(state.provider.clone(), entry.label.clone());
                let _ = app.config.save();
                app.set_status(&format!("Key '{}' active for {}", entry.label, state.provider));
            }
            match state.after {
                KeySelectAfter::ReturnToSettings => {
                    app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
                }
                KeySelectAfter::Normal => {
                    app.mode = AppMode::Normal;
                }
            }
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Add `handle_key_manage_key` function**

Add after `handle_key_select_key`:

```rust
fn handle_key_manage_key(app: &mut App, key: KeyEvent) {
    let provider = match &app.mode {
        AppMode::KeyManage(s) => s.provider.clone(),
        _ => return,
    };

    // Handle add-flow steps first
    let add_step = match &app.mode {
        AppMode::KeyManage(s) => s.add_step.clone(),
        _ => return,
    };

    if let Some(step) = add_step {
        match step {
            AddKeyStep::EnterLabel(mut buf) => {
                match key.code {
                    KeyCode::Esc => {
                        if let AppMode::KeyManage(s) = &mut app.mode { s.add_step = None; }
                    }
                    KeyCode::Enter => {
                        if !buf.trim().is_empty() {
                            if let AppMode::KeyManage(s) = &mut app.mode {
                                s.add_step = Some(AddKeyStep::EnterKey { label: buf.trim().to_string(), key_buf: String::new() });
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        if let AppMode::KeyManage(s) = &mut app.mode {
                            s.add_step = Some(AddKeyStep::EnterLabel(buf));
                        }
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                        if let AppMode::KeyManage(s) = &mut app.mode {
                            s.add_step = Some(AddKeyStep::EnterLabel(buf));
                        }
                    }
                    _ => {}
                }
                return;
            }
            AddKeyStep::EnterKey { label, mut key_buf } => {
                match key.code {
                    KeyCode::Esc => {
                        if let AppMode::KeyManage(s) = &mut app.mode { s.add_step = None; }
                    }
                    KeyCode::Enter => {
                        if !key_buf.trim().is_empty() {
                            let entries = app.config.api_keys.entry(provider.clone()).or_default();
                            if entries.len() < 100 {
                                entries.push(crate::config::ApiKeyEntry { label: label.clone(), key: key_buf.trim().to_string() });
                                let _ = app.config.save();
                            }
                            if let AppMode::KeyManage(s) = &mut app.mode {
                                s.entries = app.config.api_keys.get(&provider).cloned().unwrap_or_default();
                                s.add_step = None;
                            }
                        }
                    }
                    KeyCode::Char(c) => {
                        key_buf.push(c);
                        if let AppMode::KeyManage(s) = &mut app.mode {
                            s.add_step = Some(AddKeyStep::EnterKey { label, key_buf });
                        }
                    }
                    KeyCode::Backspace => {
                        key_buf.pop();
                        if let AppMode::KeyManage(s) = &mut app.mode {
                            s.add_step = Some(AddKeyStep::EnterKey { label, key_buf });
                        }
                    }
                    _ => {}
                }
                return;
            }
        }
    }

    // Normal list mode
    let (entries_len, selected) = match &app.mode {
        AppMode::KeyManage(s) => (s.entries.len(), s.selected),
        _ => return,
    };

    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::KeyManage(s) = &mut app.mode {
                if s.selected > 0 { s.selected -= 1; }
            }
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::KeyManage(s) = &mut app.mode {
                if s.selected < entries_len.saturating_sub(1) { s.selected += 1; }
            }
        }
        KeyCode::Char('n') => {
            if let AppMode::KeyManage(s) = &mut app.mode {
                s.add_step = Some(AddKeyStep::EnterLabel(String::new()));
            }
        }
        KeyCode::Char('d') => {
            if selected < entries_len {
                let deleted_label = {
                    let entries = app.config.api_keys.entry(provider.clone()).or_default();
                    let label = entries[selected].label.clone();
                    entries.remove(selected);
                    label
                };
                if app.config.active_key.get(&provider).map(|l| l == &deleted_label).unwrap_or(false) {
                    app.config.active_key.remove(&provider);
                }
                let _ = app.config.save();
                if let AppMode::KeyManage(s) = &mut app.mode {
                    s.entries = app.config.api_keys.get(&provider).cloned().unwrap_or_default();
                    s.selected = s.selected.min(s.entries.len().saturating_sub(1));
                }
            }
        }
        KeyCode::Enter => {
            if selected < entries_len {
                let label = match &app.mode {
                    AppMode::KeyManage(s) => s.entries[selected].label.clone(),
                    _ => return,
                };
                app.config.active_key.insert(provider.clone(), label.clone());
                let _ = app.config.save();
                app.set_status(&format!("Key '{}' active for {}", label, provider));
                if let AppMode::KeyManage(s) = &mut app.mode {
                    s.entries = app.config.api_keys.get(&provider).cloned().unwrap_or_default();
                }
            }
        }
        _ => {}
    }
}
```

- [ ] **Step 5: Wire the settings Enter handler for `manage_keys_*` fields**

In `handle_settings_key`, find the `KeyCode::Enter` match arm's inner match on `field_key.as_str()`:

```rust
                "provider" => {
                    let current = state.fields[state.selected_field].value.clone();
                    app.mode = AppMode::ProviderSelect(ProviderSelectState::new(&current, true));
                }
                "model" => {
                    app.mode = AppMode::ModelSelect(ModelSelectState {
                        models: vec![],
                        selected: 0,
                        loading: true,
                        return_to_settings: true,
                    });
                }
                _ => {
```

Replace with:

```rust
                "provider" => {
                    let current = state.fields[state.selected_field].value.clone();
                    app.mode = AppMode::ProviderSelect(ProviderSelectState::new(&current, true));
                }
                "model" => {
                    app.mode = AppMode::ModelSelect(ModelSelectState {
                        models: vec![],
                        selected: 0,
                        loading: true,
                        return_to_settings: true,
                    });
                }
                k if k.starts_with("manage_keys_") => {
                    let prov = k.trim_start_matches("manage_keys_").to_string();
                    app.mode = AppMode::KeyManage(KeyManageState::from_config(&prov, &app.config));
                }
                _ => {
```

- [ ] **Step 6: Wire the provider-switch KeySelect trigger in `handle_provider_select_key`**

Find the `KeyCode::Enter` arm in `handle_provider_select_key`:

```rust
        KeyCode::Enter => {
            if let Some(provider) = state.providers.get(state.selected) {
                app.config.provider = provider.clone();
                let _ = app.config.save();
                app.commit(ChatMessage::System(format!(
                    "Provider set to: {provider}"
                )));
                app.set_status(&format!("Provider: {}", app.config.provider_display()));
            }
            if state.return_to_settings {
                app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
            } else {
                app.mode = AppMode::Normal;
            }
        }
```

Replace with:

```rust
        KeyCode::Enter => {
            if let Some(provider) = state.providers.get(state.selected) {
                app.config.provider = provider.clone();
                let _ = app.config.save();
                app.commit(ChatMessage::System(format!(
                    "Provider set to: {provider}"
                )));
                app.set_status(&format!("Provider: {}", app.config.provider_display()));

                // If this provider has multiple keys, prompt now
                if app.config.needs_key_select(provider) {
                    let entries = app.config.api_keys.get(provider.as_str()).cloned().unwrap_or_default();
                    let after = if state.return_to_settings {
                        KeySelectAfter::ReturnToSettings
                    } else {
                        KeySelectAfter::Normal
                    };
                    app.mode = AppMode::KeySelect(KeySelectState::new(provider, entries, after));
                    return;
                }
            }
            if state.return_to_settings {
                app.mode = AppMode::Settings(SettingsState::from_config(&app.config));
            } else {
                app.mode = AppMode::Normal;
            }
        }
```

- [ ] **Step 7: Wire the startup KeySelect trigger in `App::new`**

In `src/tui/app.rs`, find `App::new`:

```rust
        Self {
            mode: AppMode::Normal,
```

Replace with:

```rust
        let initial_mode = if config.needs_key_select(&config.provider) {
            let entries = config.api_keys.get(config.provider.as_str()).cloned().unwrap_or_default();
            AppMode::KeySelect(KeySelectState::new(&config.provider.clone(), entries, KeySelectAfter::Normal))
        } else {
            AppMode::Normal
        };

        Self {
            mode: initial_mode,
```

- [ ] **Step 8: Build to confirm no compile errors**

```bash
cargo build --release 2>&1 | grep -E "^error"
```

Expected: no errors.

- [ ] **Step 9: Run all tests**

```bash
cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 10: Commit**

```bash
tiv add src/tui/input.rs src/tui/app.rs
tiv com "feat(tui/input): KeySelect and KeyManage input handlers; wire startup and provider-switch triggers"
```

---

## Task 8: Final release build + verification

- [ ] **Step 1: Full release build**

```bash
cargo build --release 2>&1
```

Expected: `Finished release profile` with no warnings about unused variables or dead code that relate to the new code.

- [ ] **Step 2: Run full test suite**

```bash
cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 3: Commit binary if changed**

```bash
cp target/release/tycode ~/.local/bin/tycode
```

No commit needed for the binary — it is not tracked.
