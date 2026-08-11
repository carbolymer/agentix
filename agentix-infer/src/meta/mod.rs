pub mod gguf;
pub mod safetensors;

use crate::{error::InferError, Capability, ModelFormat};
use std::path::Path;

pub struct DetectedMeta {
    pub architecture: String,
    pub context_length: u32,
    pub embedding_length: u32,
    pub capabilities: Vec<Capability>,
    pub parameter_count: u64,
}

pub fn detect_capabilities(path: &Path, format: ModelFormat) -> Result<DetectedMeta, InferError> {
    match format {
        ModelFormat::Gguf => {
            let m = gguf::read_gguf_metadata(path)?;
            Ok(DetectedMeta {
                architecture: m.architecture,
                context_length: m.context_length,
                embedding_length: m.embedding_length,
                capabilities: m.capabilities,
                parameter_count: m.parameter_count,
            })
        }
        ModelFormat::Safetensors => {
            // For safetensors, path should point to config.json
            let data = std::fs::read(path)?;
            let m = safetensors::read_safetensors_metadata(&data)?;
            Ok(DetectedMeta {
                architecture: m.architecture,
                context_length: m.context_length,
                embedding_length: m.embedding_length,
                capabilities: m.capabilities,
                parameter_count: 0,
            })
        }
    }
}
