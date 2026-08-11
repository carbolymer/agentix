# Research: agentix-infer

**Date**: 2026-08-08
**Status**: Complete — all NEEDS CLARIFICATION items resolved

---

## Decision 1: llama-cpp-2 crate API

**Decision**: Use `llama-cpp-2` (v0.1.154) wrapping `llama-cpp-sys-2`, which compiles llama.cpp from a bundled submodule at build time.

**Rationale**: Pure-Rust bindings with `build.rs`-managed llama.cpp compilation. No system-installed llama.cpp required. CUDA support via Cargo feature flag. The crate's API maps closely to the llama.cpp C API but is safe Rust.

**Key API signatures:**

*Model loading:*
```rust
let backend = LlamaBackend::init()?;
let params = LlamaModelParams::default().with_n_gpu_layers(32).with_use_mmap(true);
let model = LlamaModel::load_from_file(&backend, path, &params)?;
```

*Embedding context:*
```rust
let ctx_params = LlamaContextParams::default()
    .with_embeddings(true)
    .with_pooling_type(LlamaPoolingType::Mean)  // Mean | Cls | Last | Unspecified
    .with_n_ctx(NonZeroU32::new(512))
    .with_n_batch(512);
let ctx = model.new_context(&backend, ctx_params)?;
```

*Running embeddings (no single `embed()` call — must batch manually):*
```rust
// 1. Build LlamaBatch, add tokens with sequence IDs
// 2. ctx.encode(&mut batch)?   — embedding forward pass
// 3. ctx.embeddings_seq_ith(seq_id)  → Result<&[f32]>  (pooled)
//    ctx.embeddings_ith(token_idx)   → Result<&[f32]>  (per-token)
// Embedding dimension: model.n_embd() → i32
```

*Streaming completion (sampling loop):*
```rust
let sampler = LlamaSampler::chain_simple([
    LlamaSampler::top_k(40),
    LlamaSampler::top_p(0.9, 1),
    LlamaSampler::temp(0.8),
]);
// per token:
ctx.decode(&mut batch)?;
let token = sampler.sample(&mut ctx, -1);  // -1 = last position
sampler.accept(token);
let piece = model.token_to_str(token, Special::Tokenize)?;
// EOS: model.token_eos() == token
```

*GGUF KV metadata:*
```rust
let gguf = GgufContext::from_file(path)?;  // Option<GgufContext>
let idx = gguf.find_key("general.architecture");  // i64, -1 if absent
let arch = gguf.val_str(idx)?;
let pool_idx = gguf.find_key("bert.pooling_type");  // key is "{arch}.pooling_type"
let pooling = gguf.val_u32(pool_idx);
let tmpl_idx = gguf.find_key("tokenizer.chat_template");
// tmpl_idx != -1 → Completion capable
```

**build.rs requirements**: `cmake` binary, C/C++ compiler, `clang`/`libclang` for bindgen. The `cuda` Cargo feature sets `GGML_CUDA=ON`; links `cudart`, `cublas`, `cuda`. Uses `CUDA_PATH` env var. Nix flake must add `cudaPackages.cudatoolkit`, `cudaPackages.libcublas`, and `clang` to daemon build inputs.

**Alternatives considered**: `candle` alone (no GGUF support for llama-family models at this time), raw llama.cpp C bindings (more work, less safe). Neither replaces `llama-cpp-2` for Phase 1.

---

## Decision 2: HuggingFace Hub download

**Decision**: Use `hf-hub` crate (v0.3.2) sync API to download to its cache, then copy to our content-addressed blob store.

**Rationale**: `hf-hub` handles auth tokens, progress, concurrent locking, and HF CDN URL construction. We use it for the download step only; final storage is our Ollama-compatible blob layout (not hf-hub's cache layout).

**Key API:**
```rust
let api = ApiBuilder::new()
    .with_cache_dir(tmp_dir)           // redirect to a scratch dir we control
    .with_token(Some(token))           // optional HF auth token
    .with_progress(false)
    .build()?;
let repo = api.repo(Repo::with_revision(
    "jinaai/jina-embeddings-v2-base-code-GGUF".to_string(),
    RepoType::Model,
    "main".to_string(),
));
let cached_path: PathBuf = repo.get("jina-embeddings-v2-base-code-f32.gguf")?;
// Then: read cached_path, compute SHA256, write to blob store
```

`repo.get()` checks cache first, downloads if missing. `repo.download()` always hits network. Files in hf-hub cache are content-addressed by HF's own scheme — we do not rely on that; we re-hash on our end.

**Alternatives considered**: raw `reqwest` for HTTP download (doable but reinvents auth, redirect, and progress handling that hf-hub provides).

---

## Decision 3: CI fixture GGUF

**Decision**: `second-state/All-MiniLM-L6-v2-Embedding-GGUF`, file `all-MiniLM-L6-v2-Q8_0.gguf` (25 MB).

**Rationale**: 25 MB is well under the 50 MB target. BERT architecture, 384-dim output, 256-token context. Widely used embedding model with reliable GGUF conversion. Small enough to pin as a Nix FOD derivation.

**Important caveat**: The `bert.pooling_type` KV key in the GGUF metadata may be absent on older conversions (this key was added to llama.cpp later). When `find_key("bert.pooling_type")` returns -1:
- Fall back to `LlamaPoolingType::Unspecified` — llama.cpp then infers pooling from architecture
- Log a warning so operators can re-convert models with newer llama.cpp if needed
- This fallback MUST be tested as a code path; do not assume the key exists

**Nix pinning approach**: Use `fetchurl` in the Nix flake with the file's HF direct download URL and a `sha256` hash. Set `AGENTIX_TEST_MODEL_PATH` env var pointing at the unpacked file; integration tests read this env var.

**Alternatives considered**: `nomic-ai/nomic-embed-text-v1.5-GGUF` Q2_K at 48 MB (right at the limit, and Nomic conversions are more likely to have proper `pooling_type` metadata). Keep as a backup if the all-MiniLM conversion proves unreliable for capability detection tests.

---

## Decision 4: Ollama manifest JSON format

**Decision**: Our manifest format is a strict superset of Ollama's. We preserve all Ollama fields verbatim and add agentix extension fields so round-tripping an Ollama manifest back to disk preserves Ollama compatibility.

**Ollama manifest structure:**
```json
{
  "schemaVersion": 2,
  "mediaType": "application/vnd.docker.distribution.manifest.v2+json",
  "config": {
    "mediaType": "application/vnd.docker.container.image.v1+json",
    "digest": "sha256:<hex>",
    "size": 407
  },
  "layers": [
    {
      "mediaType": "application/vnd.ollama.image.model",
      "digest": "sha256:<hex>",
      "size": 45949216
    },
    {
      "mediaType": "application/vnd.ollama.image.template",
      "digest": "sha256:<hex>",
      "size": 1429
    }
  ]
}
```

**Critical detail**: Blob filenames on disk use `sha256-<hex>` (hyphen). The `digest` field in JSON uses `sha256:<hex>` (colon). The Rust types must handle this conversion.

**Agentix extension fields** (added at the root level, ignored by Ollama):
```json
{
  "_agentix": {
    "backend": "llamacpp",
    "capabilities": ["Embedding"],
    "architecture": "bert",
    "context_length": 256,
    "embedding_length": 384,
    "quantization": "Q8_0",
    "parameter_count": 22600000
  }
}
```

Using a namespaced `_agentix` key ensures Ollama ignores the extension fields when it reads our manifests. We never write agentix extension fields into Ollama's registry-format manifests for Ollama-registry models.

**Layer mediaType values** (for reference):

| mediaType | Purpose |
|-----------|---------|
| `application/vnd.ollama.image.model` | GGUF model weights |
| `application/vnd.ollama.image.template` | Chat template text |
| `application/vnd.ollama.image.params` | Inference params JSON |
| `application/vnd.ollama.image.license` | License text |
| `application/vnd.ollama.image.projector` | Vision encoder GGUF |
| `application/vnd.ollama.image.adapter` | LoRA adapter GGUF |

---

## Resolved: `spawn_blocking` pattern for C FFI

All llama.cpp calls (`ctx.encode()`, `ctx.decode()`, `LlamaModel::load_from_file()`) are blocking C FFI. The pattern:

```rust
let result = tokio::task::spawn_blocking(move || {
    // blocking llama.cpp call here
    ctx.encode(&mut batch)
}).await??;
```

The outer `?` handles `JoinError` (task panicked); the inner `?` handles the llama.cpp error. `LlamaContext` is `!Send` (it holds a raw pointer), so the context must stay inside the `spawn_blocking` closure for its entire lifetime. This means `LlamaCppLoadedModel` will hold the context inside a `Mutex` and the embed/complete methods will move a clone of the relevant data into `spawn_blocking`.

**Implication for ContextPool**: The pool stores `Arc<dyn LoadedModel>`. The llama.cpp context cannot be shared across threads without a lock. `LlamaCppLoadedModel` will hold `Mutex<LlamaContext>` internally; `LlamaModel` is `Send + Sync` and can be shared.
