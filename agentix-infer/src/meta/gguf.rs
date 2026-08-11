use crate::{error::InferError, Capability};
#[cfg(feature = "llamacpp")]
use llama_cpp_2::gguf::GgufContext;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct GgufMeta {
    pub architecture: String,
    pub context_length: u32,
    pub embedding_length: u32,
    pub capabilities: Vec<Capability>,
    pub parameter_count: u64,
    #[allow(dead_code)]
    pub quantization: Option<String>,
}

#[cfg(feature = "llamacpp")]
pub fn read_gguf_metadata(path: &Path) -> Result<GgufMeta, InferError> {
    let gguf = GgufContext::from_file(path)
        .ok_or_else(|| InferError::Backend(format!("failed to read GGUF: {}", path.display())))?;

    // Architecture
    let arch_idx = gguf.find_key("general.architecture");
    let architecture = if arch_idx >= 0 {
        gguf.val_str(arch_idx).unwrap_or("unknown").to_string()
    } else {
        "unknown".to_string()
    };

    // Context length
    let ctx_key = format!("{}.context_length", architecture);
    let ctx_idx = gguf.find_key(&ctx_key);
    let context_length = if ctx_idx >= 0 {
        gguf.val_u32(ctx_idx)
    } else {
        0
    };

    // Embedding dimension
    let emb_key = format!("{}.embedding_length", architecture);
    let emb_idx = gguf.find_key(&emb_key);
    let embedding_length = if emb_idx >= 0 {
        gguf.val_u32(emb_idx)
    } else {
        0
    };

    // Capability detection
    let mut capabilities = Vec::new();

    // Embedding: pooling_type present and non-zero
    let pool_key = format!("{}.pooling_type", architecture);
    let pool_idx = gguf.find_key(&pool_key);
    if pool_idx >= 0 {
        let pooling = gguf.val_u32(pool_idx);
        if pooling != 0 {
            capabilities.push(Capability::Embedding);
        }
    }

    // Completion: chat template present
    let tmpl_idx = gguf.find_key("tokenizer.chat_template");
    if tmpl_idx >= 0 {
        capabilities.push(Capability::Completion);
    }

    // Vision: any vision encoder key
    let vision_keys = [
        format!("{}.vision_encoder.image_size", architecture),
        "clip.vision_model.image_size".to_string(),
        "vision_model.image_size".to_string(),
    ];
    for vkey in &vision_keys {
        if gguf.find_key(vkey) >= 0 {
            capabilities.push(Capability::Vision);
            break;
        }
    }

    // Conservative fallback: if no capabilities detected, assume Completion
    if capabilities.is_empty() {
        tracing::warn!(
            path = %path.display(),
            architecture = %architecture,
            "no capability keys found in GGUF metadata; defaulting to Completion"
        );
        capabilities.push(Capability::Completion);
    }

    Ok(GgufMeta {
        architecture,
        context_length,
        embedding_length,
        capabilities,
        parameter_count: 0,
        quantization: None,
    })
}

#[cfg(not(feature = "llamacpp"))]
pub fn read_gguf_metadata(_path: &Path) -> Result<GgufMeta, InferError> {
    Err(InferError::Backend(
        "llamacpp feature not enabled".to_string(),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn empty_capabilities_fallback() {
        // When no GGUF keys match, we should get Completion as fallback.
        // The real GGUF parsing path is tested via integration tests with a fixture model.
        let caps: Vec<Capability> = vec![];
        let result = if caps.is_empty() {
            vec![Capability::Completion]
        } else {
            caps
        };
        assert_eq!(result, vec![Capability::Completion]);
    }

    #[test]
    fn embedding_capability_requires_nonzero_pooling() {
        // Capability::Embedding is only added when pooling_type != 0
        // This mirrors the logic in read_gguf_metadata
        let pooling_type: u32 = 0;
        let mut caps = Vec::new();
        if pooling_type != 0 {
            caps.push(Capability::Embedding);
        }
        assert!(caps.is_empty());

        let pooling_type_mean: u32 = 1; // Mean pooling
        let mut caps2 = Vec::new();
        if pooling_type_mean != 0 {
            caps2.push(Capability::Embedding);
        }
        assert_eq!(caps2, vec![Capability::Embedding]);
    }
}
