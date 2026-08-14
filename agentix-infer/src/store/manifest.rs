use crate::{error::InferError, BackendHint, Capability, ModelFormat, ModelInfo};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Ollama-compatible manifest with agentix extension fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub media_type: String,
    pub config: ManifestLayer,
    pub layers: Vec<ManifestLayer>,
    /// agentix extension block (ignored by Ollama)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub _agentix: Option<AgentixExtension>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestLayer {
    pub media_type: String,
    pub digest: String, // "sha256:<hex>"
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentixExtension {
    pub backend: BackendHint,
    pub capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_length: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantization: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameter_count: Option<u64>,
}

/// Convert JSON digest ("sha256:hex") to on-disk filename ("sha256-hex")
pub fn digest_to_filename(digest: &str) -> String {
    digest.replacen(':', "-", 1)
}

/// Convert on-disk filename ("sha256-hex") back to JSON digest ("sha256:hex")
pub fn filename_to_digest(name: &str) -> String {
    if let Some(rest) = name.strip_prefix("sha256-") {
        format!("sha256:{}", rest)
    } else {
        name.to_string()
    }
}

pub fn manifest_path(
    models_dir: &Path,
    registry: &str,
    namespace: &str,
    name: &str,
    tag: &str,
) -> PathBuf {
    models_dir
        .join("manifests")
        .join(registry)
        .join(namespace)
        .join(name)
        .join(tag)
}

pub fn read_manifest(path: &Path) -> Result<Manifest, InferError> {
    let data = std::fs::read(path)?;
    serde_json::from_slice(&data).map_err(|e| InferError::Manifest(e.to_string()))
}

pub fn write_manifest(path: &Path, manifest: &Manifest) -> Result<(), InferError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let data =
        serde_json::to_vec_pretty(manifest).map_err(|e| InferError::Manifest(e.to_string()))?;
    std::fs::write(path, data)?;
    Ok(())
}

/// Build ModelInfo from a manifest path + parsed manifest.
pub fn manifest_to_model_info(name: String, manifest: &Manifest) -> ModelInfo {
    let ext = manifest._agentix.as_ref();
    let format = ext
        .map(|e| match e.backend {
            BackendHint::LlamaCpp => ModelFormat::Gguf,
            BackendHint::Candle => ModelFormat::Safetensors,
            BackendHint::Whisper => ModelFormat::WhisperBin,
        })
        .unwrap_or(ModelFormat::Gguf);

    let model_layer = manifest.layers.iter().find(|l| {
        l.media_type == "application/vnd.ollama.image.model"
            || l.media_type == "application/vnd.ollama.image.tensor"
    });

    ModelInfo {
        name,
        architecture: ext.and_then(|e| e.architecture.clone()).unwrap_or_default(),
        format,
        backend: ext.map(|e| e.backend).unwrap_or(BackendHint::LlamaCpp),
        context_length: ext.and_then(|e| e.context_length).unwrap_or(0),
        embedding_length: ext.and_then(|e| e.embedding_length).unwrap_or(0),
        capabilities: ext.map(|e| e.capabilities.clone()).unwrap_or_default(),
        quantization: ext.and_then(|e| e.quantization.clone()),
        parameter_count: ext.and_then(|e| e.parameter_count).unwrap_or(0),
        size_bytes: model_layer.map(|l| l.size).unwrap_or(0),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn digest_conversion() {
        assert_eq!(digest_to_filename("sha256:abc123"), "sha256-abc123");
        assert_eq!(filename_to_digest("sha256-abc123"), "sha256:abc123");
    }

    #[test]
    fn round_trip_manifest() {
        let m = Manifest {
            schema_version: 2,
            media_type: "application/vnd.docker.distribution.manifest.v2+json".to_string(),
            config: ManifestLayer {
                media_type: "application/vnd.docker.container.image.v1+json".to_string(),
                digest: "sha256:deadbeef".to_string(),
                size: 100,
                from: None,
            },
            layers: vec![ManifestLayer {
                media_type: "application/vnd.ollama.image.model".to_string(),
                digest: "sha256:cafebabe".to_string(),
                size: 25_000_000,
                from: None,
            }],
            _agentix: Some(AgentixExtension {
                backend: BackendHint::LlamaCpp,
                capabilities: vec![Capability::Embedding],
                architecture: Some("bert".to_string()),
                context_length: Some(256),
                embedding_length: Some(384),
                quantization: Some("Q8_0".to_string()),
                parameter_count: Some(22_600_000),
            }),
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test_manifest");
        write_manifest(&path, &m).unwrap();
        let m2 = read_manifest(&path).unwrap();
        assert_eq!(m2.schema_version, 2);
        assert_eq!(
            m2._agentix.unwrap().capabilities,
            vec![Capability::Embedding]
        );
    }
}
