use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum InferError {
    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("model {0} does not support capability {1:?}")]
    CapabilityMissing(String, crate::Capability),

    #[error("no registered backend supports format {0:?}")]
    NoBackend(crate::ModelFormat),

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        path: PathBuf,
        expected: String,
        actual: String,
    },

    #[error("download failed: {0}")]
    DownloadFailed(String),

    #[error("VRAM exhausted: cannot load {model} ({required_bytes} bytes required)")]
    VramExhausted { model: String, required_bytes: u64 },

    #[error("backend error: {0}")]
    Backend(String),

    #[error("manifest error: {0}")]
    Manifest(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
