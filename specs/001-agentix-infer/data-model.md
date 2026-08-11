# Data Model: agentix-infer

## Entities

### ModelBlob

Represents a single raw model file stored content-addressed on disk.

| Field | Type | Notes |
|-------|------|-------|
| `hash` | `String` | SHA256 hex digest (64 chars) |
| `path` | `PathBuf` | `$AGENTIX_MODELS_DIR/blobs/sha256-{hash}` |
| `size_bytes` | `u64` | File size |
| `format` | `ModelFormat` | `Gguf` or `Safetensors` |

**Invariants:**
- `hash` is the SHA256 of the file at `path`. Checked on write; verified on load.
- A blob with a given hash is immutable — once written it is never modified in place.
- Partial downloads are written to a temp path and renamed atomically; a partial temp file is never registered as a blob.

---

### ModelManifest

JSON document stored at `$AGENTIX_MODELS_DIR/manifests/{registry}/{path}/{tag}`. Structure is a superset of the Ollama manifest format — all Ollama-standard fields are preserved so existing manifests round-trip correctly.

| Field | Type | Required | Notes |
|-------|------|----------|-------|
| `schema_version` | `u32` | yes | `2` (Ollama compat) |
| `media_type` | `String` | yes | `"application/vnd.ollama.image.model"` for Ollama models |
| `config` | `ManifestConfig` | yes | Points to config blob |
| `layers` | `Vec<ManifestLayer>` | yes | One or more model file blobs |
| `backend` | `Option<BackendHint>` | no | `"llamacpp"` or `"candle"`; if absent, inferred from format |
| `capabilities` | `Vec<Capability>` | no | Detected at pull time from KV/config |
| `architecture` | `Option<String>` | no | e.g. `"bert"`, `"qwen2"`, `"laguna"` |
| `context_length` | `Option<u32>` | no | Max context tokens |
| `embedding_length` | `Option<u32>` | no | Embedding vector dimension |
| `quantization` | `Option<String>` | no | e.g. `"Q8_0"`, `"Q4_K_M"` |
| `parameter_count` | `Option<u64>` | no | Total parameter count |

#### ManifestLayer

| Field | Type | Notes |
|-------|------|-------|
| `media_type` | `String` | |
| `digest` | `String` | `"sha256:{hex}"` |
| `size` | `u64` | |

---

### ModelInfo

Runtime view of a registered model. Returned by `InferEngine::info()` and `list()`.

| Field | Type | Notes |
|-------|------|-------|
| `name` | `String` | Canonical model name (e.g. `"jina-code"`) |
| `architecture` | `String` | From GGUF KV or config.json |
| `format` | `ModelFormat` | `Gguf` or `Safetensors` |
| `backend` | `BackendHint` | Which backend will be used |
| `context_length` | `u32` | Max tokens |
| `embedding_length` | `u32` | Vector dimension; `0` if not an embedding model |
| `capabilities` | `Vec<Capability>` | What the model can do |
| `quantization` | `Option<String>` | Quant scheme if applicable |
| `parameter_count` | `u64` | |
| `size_bytes` | `u64` | Total blob size |

---

### PoolEntry (internal)

Tracks a warm loaded-model instance inside the ContextPool.

| Field | Type | Notes |
|-------|------|-------|
| `model_name` | `String` | |
| `loaded` | `Arc<dyn LoadedModel>` | |
| `last_used` | `Instant` | Updated on every acquire/release |
| `vram_bytes` | `u64` | From `LoadedModel::vram_bytes()` |
| `in_use` | `bool` | True while a `ModelGuard` is live |

**Pool invariants:**
- `total_vram_bytes = Σ entry.vram_bytes` for all entries (in-use or not).
- Before loading a new model, if `total_vram_bytes + new_model_vram > limit`, evict LRU entries that are NOT in-use until budget allows.
- If all candidates are in-use and budget is exceeded, `acquire()` returns `InferError::VramExhausted`.
- Two concurrent `acquire()` calls for the same unloaded model serialize — only one load proceeds; the second waits on the same `Arc<Notify>`.

---

## Enums

```
ModelFormat:   Gguf | Safetensors
BackendHint:   LlamaCpp | Candle
Capability:    Completion | Embedding | Vision
```

## Capability Detection Rules

### GGUF (read from GGUF KV metadata at pull time)

| Rule | Key pattern | Result |
|------|-------------|--------|
| Embedding | `{arch}.pooling_type` present and value ≠ 0 | adds `Capability::Embedding` |
| Completion | `tokenizer.chat_template` present | adds `Capability::Completion` |
| Vision | `{arch}.vision_encoder.*` any key present | adds `Capability::Vision` |

If no capability keys are found, fall back to `Capability::Completion` and log a warning. This is conservative — a model without a chat template can still complete, just without structured prompt formatting.

### safetensors (read from `config.json` at pull time)

| `model_type` value | Capabilities |
|-------------------|-------------|
| `bert`, `roberta`, `nomic_bert` | `Embedding` |
| `qwen2`, `deepseek_v3`, `laguna` | `Completion` |
| `llava`, `qwen2_vl` | `Completion`, `Vision` |
| *unknown* | `Completion` (conservative default, warning logged) |

## State Transitions

### Model Lifecycle (in ModelStore)

```
[not registered]
     │ pull() / register_local()
     ▼
[registered: manifest + blob on disk]
     │ remove()
     ▼
[not registered]          (blob deleted, manifest deleted)
```

### PoolEntry Lifecycle (in ContextPool)

```
[not in pool]
     │ acquire() → load via backend
     ▼
[warm: in_use=true]
     │ ModelGuard dropped
     ▼
[warm: in_use=false, last_used=now]
     │ evict() called due to VRAM pressure
     ▼
[not in pool]             (LoadedModel dropped → VRAM freed)
```
