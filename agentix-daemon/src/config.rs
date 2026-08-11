use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    #[serde(default = "default_gateway_port")]
    pub gateway_port: u16,

    #[serde(default = "default_ollama_base_url")]
    pub ollama_base_url: String,

    #[serde(default = "default_models_dir")]
    pub models_dir: PathBuf,

    pub vram_limit_bytes: Option<u64>,

    #[serde(default = "default_max_loaded_models")]
    pub max_loaded_models: usize,

    #[allow(dead_code)] // reserved for future gateway auth
    pub agentix_api_key: Option<String>,
    pub anthropic_api_key: Option<String>,
    pub openai_api_key: Option<String>,
    pub openrouter_api_key: Option<String>,
    pub anthropic_base_url: Option<String>,
    pub openai_base_url: Option<String>,
}

fn default_gateway_port() -> u16 {
    11430
}

fn default_ollama_base_url() -> String {
    "http://localhost:11434".into()
}

fn default_models_dir() -> PathBuf {
    PathBuf::from("/var/lib/agentix/models")
}

fn default_max_loaded_models() -> usize {
    2
}

impl Config {
    pub fn from_env() -> Self {
        let gateway_port = std::env::var("AGENTIX_GATEWAY_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_gateway_port);

        let ollama_base_url =
            std::env::var("OLLAMA_BASE_URL").unwrap_or_else(|_| default_ollama_base_url());

        let models_dir = std::env::var("AGENTIX_MODELS_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_models_dir());

        let vram_limit_bytes = std::env::var("AGENTIX_VRAM_LIMIT_BYTES")
            .ok()
            .and_then(|v| v.parse().ok());

        let max_loaded_models = std::env::var("AGENTIX_MAX_LOADED_MODELS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(default_max_loaded_models);

        Self {
            gateway_port,
            ollama_base_url,
            models_dir,
            vram_limit_bytes,
            max_loaded_models,
            agentix_api_key: std::env::var("AGENTIX_API_KEY").ok(),
            anthropic_api_key: std::env::var("ANTHROPIC_API_KEY").ok(),
            openai_api_key: std::env::var("OPENAI_API_KEY").ok(),
            openrouter_api_key: std::env::var("OPENROUTER_API_KEY").ok(),
            anthropic_base_url: std::env::var("ANTHROPIC_BASE_URL_UPSTREAM").ok(),
            openai_base_url: std::env::var("OPENAI_BASE_URL_UPSTREAM").ok(),
        }
    }
}
