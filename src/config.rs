use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiKeyEntry {
    pub label: String,
    pub key: String,
}

fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tycode")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Provider selection
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default = "default_model")]
    pub model: String,

    // Ollama
    #[serde(default = "default_ollama_url")]
    pub ollama_url: String,

    // Anthropic
    #[serde(default)]
    pub anthropic_api_key: String,

    // OpenAI / compatible
    #[serde(default)]
    pub openai_api_key: String,
    #[serde(default)]
    pub openai_base_url: String,

    // Google Gemini
    #[serde(default)]
    pub google_api_key: String,

    // Multi-key store
    #[serde(default)]
    pub api_keys: std::collections::HashMap<String, Vec<ApiKeyEntry>>,
    #[serde(default)]
    pub active_key: std::collections::HashMap<String, String>,

    // Agent behaviour
    #[serde(default = "default_true")]
    pub auto_execute: bool,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_compact_threshold")]
    pub compact_threshold: usize,
}

fn default_compact_threshold() -> usize { 80_000 }
fn default_provider() -> String { "ollama".into() }
fn default_model() -> String { "gemma3".into() }
fn default_ollama_url() -> String { "http://localhost:11434".into() }
fn default_true() -> bool { true }
fn default_max_iterations() -> u32 { 100 }
fn default_max_tokens() -> u32 { 8192 }

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: default_provider(),
            model: default_model(),
            ollama_url: default_ollama_url(),
            anthropic_api_key: String::new(),
            openai_api_key: String::new(),
            openai_base_url: String::new(),
            google_api_key: String::new(),
            api_keys: std::collections::HashMap::new(),
            active_key: std::collections::HashMap::new(),
            auto_execute: true,
            max_iterations: default_max_iterations(),
            max_tokens: default_max_tokens(),
            compact_threshold: default_compact_threshold(),
        }
    }
}

impl Config {
    pub fn load() -> Self {
        let path = config_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(data) => match serde_json::from_str(&data) {
                    Ok(cfg) => return cfg,
                    Err(e) => eprintln!("Config parse error: {e}"),
                },
                Err(e) => eprintln!("Config read error: {e}"),
            }
        }
        Self::default()
    }

    pub fn save(&self) -> Result<()> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(config_path(), json)?;
        Ok(())
    }

    pub fn provider_display(&self) -> String {
        format!("{} / {}", self.provider, self.model)
    }

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
}

pub fn is_local_provider(name: &str) -> bool {
    matches!(name, "ollama" | "gguf")
}

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
    fn test_resolve_api_key_stale_active_label_falls_back_to_first() {
        let mut config = Config::default();
        config.api_keys.insert("anthropic".into(), vec![
            ApiKeyEntry { label: "current".into(), key: "k-current".into() },
        ]);
        config.active_key.insert("anthropic".into(), "deleted".into()); // stale label
        assert_eq!(config.resolve_api_key("anthropic"), "k-current");
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
