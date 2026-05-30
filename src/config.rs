use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}
