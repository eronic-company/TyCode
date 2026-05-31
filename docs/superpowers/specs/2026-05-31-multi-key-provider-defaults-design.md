# Multi-Key Support & Provider-Defined Defaults

**Date:** 2026-05-31
**Items:** PLAN.md #6 (Multiple API Keys) and #7 (Provider-Defined Max Tokens & Iterations)

---

## Overview

Two related config-layer improvements:
1. Users can register up to 100 API keys per provider, each with an identifying label. A prompt lets the user choose which key is active when switching to or starting with that provider.
2. API providers (Anthropic, OpenAI, Gemini) define their own sensible max-tokens and max-iterations defaults. Those settings are hidden from the settings screen for API providers and remain visible only for Ollama.

All changes are confined to the settings layer. The main chat view is untouched.

---

## Data Model (`src/config.rs`)

Two new fields added to `Config`:

```rust
pub api_keys: HashMap<String, Vec<ApiKeyEntry>>,
pub active_key: HashMap<String, String>,
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiKeyEntry {
    pub label: String,
    pub key: String,
}
```

- `api_keys` maps provider name (e.g. `"anthropic"`) to an ordered list of key entries.
- `active_key` maps provider name to the label of the currently selected key.
- Both fields default to empty maps, so existing `config.json` files load without migration.
- Existing single-key fields (`anthropic_api_key`, `openai_api_key`, `google_api_key`) are kept as fallback: if `api_keys` has no entries for a provider, the old field is used.
- `max_tokens` and `max_iterations` remain in `Config` for Ollama use. API providers ignore them.
- Maximum 100 entries per provider, enforced on add.

**Key resolution order (at provider creation):**
1. Look up `active_key[provider]` → find matching label in `api_keys[provider]` → use that key.
2. If `api_keys[provider]` is non-empty but no active label is set → use index 0.
3. If `api_keys[provider]` is empty → fall back to the legacy single-key field.

---

## Provider Trait (`src/provider/mod.rs`)

Two new methods with default implementations so existing providers compile without changes:

```rust
fn default_max_tokens(&self) -> u32 { 8192 }
fn default_max_iterations(&self) -> u32 { 100 }
```

Per-provider overrides:

| Provider  | default_max_tokens | default_max_iterations |
|-----------|--------------------|------------------------|
| Anthropic | 8192               | 100                    |
| OpenAI    | 4096               | 100                    |
| Gemini    | 8192               | 100                    |
| Ollama    | reads `config`     | reads `config`         |
| GGUF      | reads `config`     | reads `config`         |

For local providers (Ollama, GGUF), the two methods return the config values passed at construction time so behaviour is unchanged. Any future local provider follows the same pattern.

**Agent loop (`src/agent.rs`):** Replace `config.max_iterations` with `provider.default_max_iterations()` and `config.max_tokens` with `provider.default_max_tokens()` in the chat call. Ollama still returns the config values from those methods, so no behaviour change for Ollama users.

---

## Key Selection Prompt (`src/tui/app.rs`)

New app mode:

```rust
AppMode::KeySelect(KeySelectState)
```

```rust
pub struct KeySelectState {
    pub provider: String,
    pub entries: Vec<ApiKeyEntry>,
    pub selected: usize,
    pub after: KeySelectAfter,
}

pub enum KeySelectAfter {
    Normal,           // startup path or direct provider switch
    ReturnToSettings, // came from settings screen
}
```

**Trigger conditions:**
- App startup: active provider has `> 1` entry in `api_keys`.
- Provider switch (via ProviderSelect): newly selected provider has `> 1` entry in `api_keys`.

**Interactions:**
- Up/Down — move selection.
- Enter — set `active_key[provider] = selected label`, save config, transition to `after` destination.
- Esc — cancel; if no key was previously active, silently default to index 0.

Key values are masked as `••••••••` in the list. Labels are shown in full.

---

## Settings Screen Changes (`src/tui/app.rs`, `src/tui/ui.rs`, `src/tui/input.rs`)

### Max tokens / max iterations visibility

`SettingsState::from_config` builds its field list dynamically. The `Max Tokens` and `Max Iterations` fields are included only for local providers (`"ollama"`, `"gguf"`). They are omitted for API providers. A helper `fn is_local_provider(name: &str) -> bool` centralises this check so future local providers are covered by adding one entry.

### "Manage Keys" entries

For each API provider (anthropic, openai, gemini), a read-only summary field is appended to the settings field list:

```
Anthropic Keys    [2 keys]
```

Pressing Enter on this field opens `AppMode::KeyManage(KeyManageState)`.

### KeyManage sub-screen

```rust
pub struct KeyManageState {
    pub provider: String,
    pub entries: Vec<ApiKeyEntry>,
    pub selected: usize,
    pub add_step: Option<AddKeyStep>,
}

pub enum AddKeyStep {
    EnterLabel(String),
    EnterKey { label: String, key_buf: String },
}
```

**Interactions:**
- Up/Down — move selection through the key list.
- `n` — begin add flow: step 1 prompts for label, step 2 prompts for key value; on completion appends to `api_keys[provider]` (if under 100) and saves.
- `d` — delete selected entry; if deleted entry was the active key, clears `active_key[provider]`.
- Enter — set selected entry as active key, save config.
- Esc — return to settings.

Key values are masked as `••••••••` in the list display. Full key is never shown after entry.

---

## Files Changed

| File | Change |
|------|--------|
| `src/config.rs` | Add `ApiKeyEntry`, `api_keys`, `active_key` fields; key resolution helper |
| `src/provider/mod.rs` | Add `default_max_tokens`, `default_max_iterations` to trait |
| `src/provider/anthropic.rs` | Override trait methods |
| `src/provider/openai.rs` | Override trait methods |
| `src/provider/gemini.rs` | Override trait methods |
| `src/provider/ollama.rs` | Override trait methods (read from config) |
| `src/agent.rs` | Use provider methods instead of config fields |
| `src/tui/app.rs` | Add `KeySelect`, `KeyManage` modes and states; update `SettingsState` |
| `src/tui/ui.rs` | Render `KeySelect` and `KeyManage` screens |
| `src/tui/input.rs` | Handle input for `KeySelect` and `KeyManage` modes |
