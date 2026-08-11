use crate::{error::InferError, Capability};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct SafetensorsConfig {
    model_type: Option<String>,
    #[allow(dead_code)]
    num_hidden_layers: Option<u32>,
    hidden_size: Option<u32>,
    max_position_embeddings: Option<u32>,
}

#[derive(Debug)]
pub struct SafetensorsMeta {
    pub architecture: String,
    pub context_length: u32,
    pub embedding_length: u32,
    pub capabilities: Vec<Capability>,
}

pub fn read_safetensors_metadata(config_json: &[u8]) -> Result<SafetensorsMeta, InferError> {
    let cfg: SafetensorsConfig = serde_json::from_slice(config_json)
        .map_err(|e| InferError::Manifest(format!("invalid config.json: {}", e)))?;

    let model_type = cfg.model_type.as_deref().unwrap_or("unknown");

    let capabilities = match model_type {
        "bert" | "roberta" | "nomic_bert" | "xlm-roberta" => vec![Capability::Embedding],
        "qwen2" | "deepseek_v3" | "laguna" | "llama" | "mistral" | "phi" | "gemma" => {
            vec![Capability::Completion]
        }
        "llava" | "qwen2_vl" | "idefics" => vec![Capability::Completion, Capability::Vision],
        _ => {
            tracing::warn!(
                model_type = %model_type,
                "unknown model_type in config.json; defaulting to Completion"
            );
            vec![Capability::Completion]
        }
    };

    Ok(SafetensorsMeta {
        architecture: model_type.to_string(),
        context_length: cfg.max_position_embeddings.unwrap_or(0),
        embedding_length: cfg.hidden_size.unwrap_or(0),
        capabilities,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn bert_maps_to_embedding() {
        let config =
            br#"{"model_type": "bert", "hidden_size": 384, "max_position_embeddings": 512}"#;
        let meta = read_safetensors_metadata(config).unwrap();
        assert_eq!(meta.architecture, "bert");
        assert_eq!(meta.capabilities, vec![Capability::Embedding]);
        assert_eq!(meta.embedding_length, 384);
        assert_eq!(meta.context_length, 512);
    }

    #[test]
    fn qwen2_maps_to_completion() {
        let config =
            br#"{"model_type": "qwen2", "hidden_size": 2048, "max_position_embeddings": 4096}"#;
        let meta = read_safetensors_metadata(config).unwrap();
        assert_eq!(meta.capabilities, vec![Capability::Completion]);
    }

    #[test]
    fn llava_maps_to_completion_and_vision() {
        let config = br#"{"model_type": "llava"}"#;
        let meta = read_safetensors_metadata(config).unwrap();
        assert!(meta.capabilities.contains(&Capability::Completion));
        assert!(meta.capabilities.contains(&Capability::Vision));
    }

    #[test]
    fn unknown_model_type_defaults_to_completion() {
        let config = br#"{"model_type": "some_unknown_arch"}"#;
        let meta = read_safetensors_metadata(config).unwrap();
        assert_eq!(meta.capabilities, vec![Capability::Completion]);
    }

    #[test]
    fn invalid_json_returns_error() {
        assert!(read_safetensors_metadata(b"not json").is_err());
    }
}
