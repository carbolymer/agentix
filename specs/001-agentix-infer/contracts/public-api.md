# Public API Contract: agentix-infer

This document defines the Rust public API surface of the `agentix-infer` library crate.
It is the contract that `agentix-daemon` (and any future consumer) depends on.
Breaking changes to any signature or trait method require a semver-major bump and a
documented migration note.

---

## Core Types

### InferConfig

Configuration passed to `InferEngine::new()` at startup.

```rust
pub struct InferConfig {
    /// Root directory for blob store and manifests.
    /// Default: /var/lib/agentix/models
    pub models_dir: PathBuf,

    /// Maximum VRAM bytes the pool may use across all loaded models.
    /// None = unlimited (use all available VRAM).
    pub vram_limit_bytes: Option<u64>,

    /// Maximum number of concurrently warm model instances.
    /// Default: 2
    pub max_loaded_models: usize,
}
```

---

### ModelInfo (read-only, cloneable)

```rust
#[derive(Debug, Clone)]
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
```

---

### Enums

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ModelFormat { Gguf, Safetensors }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BackendHint { LlamaCpp, Candle }

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Capability { Completion, Embedding, Vision }
```

---

### CompletionRequest / CompletionChunk

```rust
#[derive(Debug, Clone)]
pub struct CompletionRequest {
    pub messages: Vec<CompletionMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub stop: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CompletionMessage {
    pub role: String,   // "system" | "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct CompletionChunk {
    pub delta: String,
    pub finish_reason: Option<FinishReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishReason { Stop, Length, Error }
```

---

## Traits

### InferBackend

Implemented by each backend. `InferEngine` dispatches through this trait.

```rust
#[async_trait::async_trait]
pub trait InferBackend: Send + Sync {
    /// Load a model blob into memory. Returns a handle to the loaded model.
    async fn load(&self, blob_path: &Path, info: &ModelInfo) -> Result<Arc<dyn LoadedModel>, InferError>;

    /// Return true if this backend can handle the given model format.
    fn supports_format(&self, format: ModelFormat) -> bool;

    /// Human-readable backend name for logging and error messages.
    fn name(&self) -> &'static str;
}
```

**Contract:**
- `load()` is called at most once per `(blob_path, model_name)` pair concurrently (pool serializes concurrent loads for the same model).
- `load()` must not block the Tokio runtime. Implementations MUST use `tokio::task::spawn_blocking` for any synchronous C FFI.
- The returned `Arc<dyn LoadedModel>` is thread-safe; concurrent calls on the same instance MUST be safe.

---

### LoadedModel

A warm, in-memory model instance. Acquired from and released back to the ContextPool.

```rust
#[async_trait::async_trait]
pub trait LoadedModel: Send + Sync {
    /// Embed a single string. Panics if model does not support Embedding.
    async fn embed(&self, input: &str) -> Result<Vec<f32>, InferError>;

    /// Embed a batch of strings. More efficient than calling embed() repeatedly.
    async fn embed_batch(&self, inputs: &[&str]) -> Result<Vec<Vec<f32>>, InferError>;

    /// Stream completion tokens. Panics if model does not support Completion.
    async fn complete(
        &self,
        req: CompletionRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<CompletionChunk, InferError>> + Send>>, InferError>;

    /// Tokenize text without inference.
    async fn tokenize(&self, text: &str) -> Result<Vec<i32>, InferError>;

    /// Bytes of GPU/CPU memory held by this instance (used by pool for eviction).
    fn vram_bytes(&self) -> u64;
}
```

**Contract:**
- All `async` methods must not block the Tokio runtime. C FFI uses `spawn_blocking`.
- `embed()` on a model without `Capability::Embedding` MUST return `InferError::CapabilityMissing`.
- `complete()` on a model without `Capability::Completion` MUST return `InferError::CapabilityMissing`.
- `vram_bytes()` is synchronous and infallible; it MUST return a stable value for the lifetime of the instance.

---

## InferEngine (top-level handle)

`Clone + Send + Sync`. Cheap to clone (Arc internals). Lives in `AppState`.

```rust
impl InferEngine {
    /// Create a new engine. Registers no backends — call register_backend() after construction.
    pub async fn new(config: InferConfig) -> Result<Self, InferError>;

    /// Register an inference backend. Called at startup before any inference requests.
    pub fn register_backend(&self, backend: Arc<dyn InferBackend>);

    // ── Model management ──────────────────────────────────────────────────────

    /// Pull a model from a remote source (HuggingFace Hub, URL, or Ollama registry).
    /// model_ref examples: "hf.co/jinaai/jina-code-embeddings-1.5b-GGUF:Q8_0"
    ///                     "library/qwen2.5-coder:14b"
    pub async fn pull(&self, model_ref: &str) -> Result<ModelInfo, InferError>;

    /// List all registered models.
    pub async fn list(&self) -> Vec<ModelInfo>;

    /// Remove a model (manifests + blobs, if not referenced by another manifest).
    pub async fn remove(&self, name: &str) -> Result<(), InferError>;

    /// Look up a model by name without acquiring it.
    pub fn info(&self, name: &str) -> Option<ModelInfo>;

    // ── Inference ─────────────────────────────────────────────────────────────

    /// Embed a single input string.
    pub async fn embed(&self, model: &str, input: &str) -> Result<Vec<f32>, InferError>;

    /// Embed multiple inputs (single batch call to backend).
    pub async fn embed_batch(
        &self,
        model: &str,
        inputs: &[&str],
    ) -> Result<Vec<Vec<f32>>, InferError>;

    /// Stream a completion. Returns an async stream of token chunks.
    pub async fn complete(
        &self,
        model: &str,
        req: CompletionRequest,
    ) -> Result<impl Stream<Item = Result<CompletionChunk, InferError>> + Send, InferError>;

    /// Tokenize text using the model's tokenizer (no inference).
    pub async fn tokenize(&self, model: &str, text: &str) -> Result<Vec<i32>, InferError>;
}
```

---

## Error Type

```rust
#[derive(Debug, thiserror::Error)]
pub enum InferError {
    #[error("model not found: {0}")]
    ModelNotFound(String),

    #[error("model {0} does not support capability {1:?}")]
    CapabilityMissing(String, Capability),

    #[error("no registered backend supports format {0:?}")]
    NoBackend(ModelFormat),

    #[error("checksum mismatch for {path}: expected {expected}, got {actual}")]
    ChecksumMismatch { path: PathBuf, expected: String, actual: String },

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
```

---

## Stability Guarantees

- `InferBackend` and `LoadedModel` traits are sealed to this crate in Phase 1. External implementations are not supported until the API stabilizes.
- `InferEngine` public methods are stable from Phase 3 onward.
- `ModelInfo`, `InferConfig`, `CompletionRequest`, `CompletionChunk` are non-exhaustive (`#[non_exhaustive]`) — new fields may be added in minor releases.
- `InferError` variants may be added in minor releases; callers should use a catch-all `_` arm.
