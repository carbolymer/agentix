pub mod backend;
pub mod engine;
pub mod error;
mod meta;
pub mod pool;
pub mod store;

pub use engine::InferEngine;
pub use error::InferError;

use std::path::PathBuf;

// ── Enums ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelFormat {
    Gguf,
    Safetensors,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BackendHint {
    LlamaCpp,
    Candle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Capability {
    Completion,
    Embedding,
    Vision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FinishReason {
    Stop,
    Length,
    Error,
}

// ── Structs ──────────────────────────────────────────────────────────────────

#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InferConfig {
    pub models_dir: PathBuf,
    pub vram_limit_bytes: Option<u64>,
    pub max_loaded_models: usize,
}

impl InferConfig {
    pub fn new(
        models_dir: PathBuf,
        vram_limit_bytes: Option<u64>,
        max_loaded_models: usize,
    ) -> Self {
        Self {
            models_dir,
            vram_limit_bytes,
            max_loaded_models,
        }
    }
}

impl Default for InferConfig {
    fn default() -> Self {
        Self {
            models_dir: PathBuf::from("/var/lib/agentix/models"),
            vram_limit_bytes: None,
            max_loaded_models: 2,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub architecture: String,
    pub format: ModelFormat,
    pub backend: BackendHint,
    pub context_length: u32,
    pub embedding_length: u32,
    pub capabilities: Vec<Capability>,
    pub quantization: Option<String>,
    pub parameter_count: u64,
    pub size_bytes: u64,
}

#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionRequest {
    pub messages: Vec<CompletionMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub stop: Vec<String>,
}

#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionMessage {
    pub role: String,
    pub content: String,
}

#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionChunk {
    pub delta: String,
    pub finish_reason: Option<FinishReason>,
}
