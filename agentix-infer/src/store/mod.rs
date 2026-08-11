pub mod blob;
pub mod hf;
pub mod manifest;

use crate::meta;
use crate::{error::InferError, BackendHint, ModelFormat, ModelInfo};
use manifest::{AgentixExtension, Manifest, ManifestLayer};
use std::path::{Path, PathBuf};

pub struct ModelStore {
    pub models_dir: PathBuf,
}

impl ModelStore {
    pub fn new(models_dir: PathBuf) -> Self {
        Self { models_dir }
    }

    /// Pull a model from a remote source.
    /// Supported refs:
    /// - "hf.co/org/repo:filename.gguf" — HuggingFace Hub GGUF
    /// - "/local/path/to/model.gguf" — local file path
    pub fn pull(&self, model_ref: &str) -> Result<ModelInfo, InferError> {
        if model_ref.starts_with("hf.co/") {
            self.pull_hf(model_ref)
        } else if model_ref.starts_with('/') || model_ref.starts_with("./") {
            self.register_local(model_ref)
        } else {
            Err(InferError::DownloadFailed(format!(
                "unsupported model ref format: '{}'. Use hf.co/org/repo:file or /path/to/file",
                model_ref
            )))
        }
    }

    fn pull_hf(&self, model_ref: &str) -> Result<ModelInfo, InferError> {
        let hf_ref = hf::parse_hf_ref(model_ref)?;
        let format = detect_format(&hf_ref.filename);
        let (hash, size) = hf::download_to_blob_store(&hf_ref, &self.models_dir)?;

        let blob_path = blob::blob_path(&self.models_dir, &hash);
        let detected = meta::detect_capabilities(&blob_path, format)?;

        let backend = match format {
            ModelFormat::Gguf => BackendHint::LlamaCpp,
            ModelFormat::Safetensors => BackendHint::Candle,
        };

        // Use the original model ref as the canonical name so that
        // store.info(model_ref) finds the manifest after a pull.
        let name = model_ref.to_string();

        let manifest = build_manifest(&hash, size, format, backend, &detected, &name);
        let manifest_path = self.manifest_path_for(&name);
        manifest::write_manifest(&manifest_path, &manifest)?;

        Ok(manifest::manifest_to_model_info(name, &manifest))
    }

    fn register_local(&self, path_str: &str) -> Result<ModelInfo, InferError> {
        let path = Path::new(path_str);
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| InferError::DownloadFailed("invalid local path".to_string()))?;
        let format = detect_format(filename);

        let file = std::fs::File::open(path)?;
        let (hash, size) = blob::write_blob(&self.models_dir, file)?;

        let blob_path = blob::blob_path(&self.models_dir, &hash);
        let detected = meta::detect_capabilities(&blob_path, format)?;

        let backend = match format {
            ModelFormat::Gguf => BackendHint::LlamaCpp,
            ModelFormat::Safetensors => BackendHint::Candle,
        };

        // Use the original path string as the canonical name.
        let name = path_str.to_string();

        let manifest = build_manifest(&hash, size, format, backend, &detected, &name);
        let manifest_path = self.manifest_path_for(&name);
        manifest::write_manifest(&manifest_path, &manifest)?;

        Ok(manifest::manifest_to_model_info(name, &manifest))
    }

    pub fn list(&self) -> Vec<ModelInfo> {
        let manifests_dir = self.models_dir.join("manifests");
        if !manifests_dir.exists() {
            return vec![];
        }
        let mut result = Vec::new();
        self.walk_manifests(&manifests_dir, &mut result);
        result
    }

    fn walk_manifests(&self, dir: &Path, out: &mut Vec<ModelInfo>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.walk_manifests(&path, out);
            } else if path.is_file()
                && !path
                    .file_name()
                    .map(|n| n == "_aliases.json")
                    .unwrap_or(false)
            {
                if let Ok(m) = manifest::read_manifest(&path) {
                    // Reconstruct name from path relative to manifests/
                    let manifests_dir = self.models_dir.join("manifests");
                    let name = path
                        .strip_prefix(&manifests_dir)
                        .ok()
                        .and_then(|p| p.to_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(manifest::manifest_to_model_info(name, &m));
                }
            }
        }
    }

    pub fn info(&self, name: &str) -> Option<ModelInfo> {
        let path = self.find_manifest(name)?;
        let m = manifest::read_manifest(&path).ok()?;
        Some(manifest::manifest_to_model_info(name.to_string(), &m))
    }

    pub fn remove(&self, name: &str) -> Result<(), InferError> {
        let manifest_path = self.manifest_path_for(name);
        let manifest = manifest::read_manifest(&manifest_path)?;

        // Remove manifest file
        std::fs::remove_file(&manifest_path)?;

        // Remove blobs if not referenced by other manifests
        for layer in &manifest.layers {
            let hash = layer
                .digest
                .strip_prefix("sha256:")
                .unwrap_or(&layer.digest);
            if !self.blob_referenced_elsewhere(hash, name) {
                let blob = blob::blob_path(&self.models_dir, hash);
                if blob.exists() {
                    std::fs::remove_file(&blob)?;
                }
            }
        }
        Ok(())
    }

    /// Resolve a model name to its primary GGUF/safetensors blob path.
    pub fn resolve(&self, name: &str) -> Option<PathBuf> {
        let manifest_path = self.find_manifest(name)?;
        let manifest = manifest::read_manifest(&manifest_path).ok()?;
        let layer = manifest.layers.iter().find(|l| {
            l.media_type == "application/vnd.ollama.image.model"
                || l.media_type == "application/vnd.ollama.image.tensor"
        })?;
        let hash = layer.digest.strip_prefix("sha256:")?;
        Some(blob::blob_path(&self.models_dir, hash))
    }

    fn find_manifest(&self, name: &str) -> Option<PathBuf> {
        let p = self.manifest_path_for(name);
        if p.exists() { Some(p) } else { None }
    }

    fn manifest_path_for(&self, name: &str) -> PathBuf {
        // Agentix write layout: manifests/agentix/<name>/latest
        self.models_dir
            .join("manifests")
            .join("agentix")
            .join(name)
            .join("latest")
    }

    fn blob_referenced_elsewhere(&self, hash: &str, exclude_name: &str) -> bool {
        let manifests_dir = self.models_dir.join("manifests");
        if !manifests_dir.exists() {
            return false;
        }
        let mut referenced = false;
        self.check_blob_refs(&manifests_dir, hash, exclude_name, &mut referenced);
        referenced
    }

    fn check_blob_refs(&self, dir: &Path, hash: &str, exclude_name: &str, found: &mut bool) {
        if *found {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.check_blob_refs(&path, hash, exclude_name, found);
            } else if path.is_file() {
                // Skip the manifest we're removing
                let manifest_base = self.manifest_path_for(exclude_name);
                if path == manifest_base {
                    continue;
                }
                if let Ok(m) = manifest::read_manifest(&path) {
                    for layer in &m.layers {
                        let h = layer
                            .digest
                            .strip_prefix("sha256:")
                            .unwrap_or(&layer.digest);
                        if h == hash {
                            *found = true;
                            return;
                        }
                    }
                }
            }
        }
    }
}

fn detect_format(filename: &str) -> ModelFormat {
    if filename.ends_with(".gguf") {
        ModelFormat::Gguf
    } else if filename.ends_with(".safetensors") || filename.ends_with(".bin") {
        ModelFormat::Safetensors
    } else {
        ModelFormat::Gguf // default
    }
}

fn build_manifest(
    hash: &str,
    size: u64,
    _format: ModelFormat,
    backend: BackendHint,
    meta: &meta::DetectedMeta,
    _name: &str,
) -> Manifest {
    Manifest {
        schema_version: 2,
        media_type: "application/vnd.docker.distribution.manifest.v2+json".to_string(),
        config: ManifestLayer {
            media_type: "application/vnd.docker.container.image.v1+json".to_string(),
            digest: format!("sha256:{}", hash),
            size: 0,
            from: None,
        },
        layers: vec![ManifestLayer {
            media_type: "application/vnd.ollama.image.model".to_string(),
            digest: format!("sha256:{}", hash),
            size,
            from: None,
        }],
        _agentix: Some(AgentixExtension {
            backend,
            capabilities: meta.capabilities.clone(),
            architecture: if meta.architecture.is_empty() {
                None
            } else {
                Some(meta.architecture.clone())
            },
            context_length: if meta.context_length > 0 {
                Some(meta.context_length)
            } else {
                None
            },
            embedding_length: if meta.embedding_length > 0 {
                Some(meta.embedding_length)
            } else {
                None
            },
            quantization: None,
            parameter_count: if meta.parameter_count > 0 {
                Some(meta.parameter_count)
            } else {
                None
            },
        }),
    }
}
