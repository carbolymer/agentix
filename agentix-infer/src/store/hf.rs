use crate::error::InferError;
use std::path::Path;

/// Parsed reference to a HuggingFace model file.
#[derive(Debug, Clone)]
pub struct HfRef {
    pub repo_id: String,
    pub filename: String,
    pub revision: String,
}

/// Parse model refs:
/// - "hf.co/org/repo:filename.gguf" → HfRef { repo_id: "org/repo", filename: "filename.gguf", revision: "main" }
pub fn parse_hf_ref(model_ref: &str) -> Result<HfRef, InferError> {
    let stripped = model_ref.strip_prefix("hf.co/").unwrap_or(model_ref);
    // format: org/repo:filename
    if let Some((repo_part, filename)) = stripped.rsplit_once(':') {
        // ensure repo_part has exactly one slash (i.e. it's org/repo)
        if repo_part.contains('/') {
            return Ok(HfRef {
                repo_id: repo_part.to_string(),
                filename: filename.to_string(),
                revision: "main".to_string(),
            });
        }
    }
    // no colon — treat whole thing as repo_id with no specific file
    Err(InferError::DownloadFailed(format!(
        "HF ref '{}' must specify a filename: use hf.co/org/repo:filename.gguf",
        model_ref
    )))
}

/// Download a file from HuggingFace Hub and write it to the blob store.
/// Returns (sha256_hash, size_bytes).
pub fn download_to_blob_store(
    hf_ref: &HfRef,
    models_dir: &Path,
) -> Result<(String, u64), InferError> {
    use hf_hub::api::sync::ApiBuilder;
    use hf_hub::{Repo, RepoType};

    // Use a temp dir as hf-hub cache so we control where files land
    let cache_dir = tempfile::tempdir().map_err(InferError::Io)?;

    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.path().to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|e| InferError::DownloadFailed(e.to_string()))?;

    let repo = api.repo(Repo::with_revision(
        hf_ref.repo_id.clone(),
        RepoType::Model,
        hf_ref.revision.clone(),
    ));

    let cached_path = repo
        .get(&hf_ref.filename)
        .map_err(|e| InferError::DownloadFailed(format!("HF download failed: {}", e)))?;

    // Stream from cache into our blob store
    let file = std::fs::File::open(&cached_path)?;
    let (hash, size) = super::blob::write_blob(models_dir, file)?;

    tracing::info!(
        repo = %hf_ref.repo_id,
        filename = %hf_ref.filename,
        hash = %hash,
        size = %size,
        "downloaded blob from HuggingFace"
    );

    Ok((hash, size))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_hf_ref() {
        let r = parse_hf_ref(
            "hf.co/second-state/All-MiniLM-L6-v2-Embedding-GGUF:all-MiniLM-L6-v2-Q8_0.gguf",
        )
        .unwrap();
        assert_eq!(r.repo_id, "second-state/All-MiniLM-L6-v2-Embedding-GGUF");
        assert_eq!(r.filename, "all-MiniLM-L6-v2-Q8_0.gguf");
        assert_eq!(r.revision, "main");
    }

    #[test]
    fn parse_hf_ref_without_prefix() {
        let r = parse_hf_ref("org/repo:model.gguf").unwrap();
        assert_eq!(r.repo_id, "org/repo");
        assert_eq!(r.filename, "model.gguf");
    }

    #[test]
    fn parse_hf_ref_missing_filename_returns_err() {
        assert!(parse_hf_ref("hf.co/org/repo-only").is_err());
    }
}
