# Implementation Plan: agentix-infer

**Branch**: `001-agentix-infer` | **Date**: 2026-08-08 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/001-agentix-infer/spec.md`

## Summary

Build `agentix-infer`: a pure Rust library crate that replaces the Ollama HTTP proxy in agentix-daemon with in-process inference. Phase 1 delivers GGUF embedding and completion via the llama.cpp backend, making agentix-daemon start with no Ollama dependency. Phase 2 adds a Candle backend for safetensors models (Laguna). The daemon's external OpenAI-compatible API surface is unchanged.

## Technical Context

**Language/Version**: Rust 1.80+ (edition 2021), async via Tokio  
**Primary Dependencies**: `llama-cpp-2` (GGUF inference, Phase 1), `candle-core/candle-transformers` (Phase 2), `hf-hub` (HuggingFace download), `tokio` (async runtime + spawn_blocking for C FFI)  
**Storage**: Content-addressed blob store on local filesystem (`$AGENTIX_MODELS_DIR`), Ollama-compatible layout  
**Testing**: `cargo test -p agentix-infer`; integration tests use a small (<50MB) quantized fixture GGUF pinned in Nix flake  
**Target Platform**: Linux x86_64; GPU via CUDA (optional feature flag)  
**Project Type**: Library crate — no `main`, no network surface; daemon assembles and calls it  
**Performance Goals**: Embedding latency for <512 token inputs lower than baseline Ollama-proxy path (two fewer process boundaries); no blocking of Tokio event loop (all C FFI via `spawn_blocking`)  
**Constraints**: `unsafe` blocks require `// SAFETY:` comments (Principle VIII gate 4); `clippy::unwrap_used` enforced; integration tests MUST NOT require multi-gigabyte downloads (Principle VI)  
**Scale/Scope**: Single-host; VRAM budget configurable; ContextPool holds ≤N warm models concurrently (N configurable, default 2)

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Library-First | ✅ PASS | `agentix-infer` is a library crate. No `tokio::main`, no `axum::Router`. Independently testable with `cargo test -p agentix-infer`. |
| II. Local-First | ✅ PASS | This feature IS the local inference path. Removes cloud-proxy-by-default risk (Ollama upgrade breakage). |
| III. Reproducible | ✅ PASS (action required) | `build.rs` for llama.cpp. Nix flake must add `cudaPackages.cudatoolkit` + `libcublas` to agentix-daemon derivation for CUDA builds. Non-CUDA builds remain possible. |
| IV. Isolation | ✅ PASS | Library runs inside daemon process (outside jail). No sandbox changes needed. |
| V. Layered API | ✅ PASS | `agentix-infer` sits below daemon in dependency graph. Does NOT depend on `agentix-api`, `agentix-router`, or the daemon. Routing library selects `RouteTarget::Local`; daemon translates to `InferEngine::embed/complete`. Clean layering. |
| VI. Testing | ✅ PASS (action required) | Integration tests MUST use a small pinned GGUF fixture (target <50MB) — NOT jina-code (1.5GB). Fixture must be pinned in Nix flake as FOD derivation. `spawn_blocking` required for all llama.cpp C FFI (architectural invariant per constitution). |
| VII. Agent State Machine | ✅ N/A | Inference library, not an agent loop. |
| VIII. Quality Gates | ✅ PASS | `clippy::unwrap_used` + `clippy::expect_used` enforced in `[workspace.lints]`. FFI boundary needs `// SAFETY:` comments. |

**Constitution Check result: PASS** — no violations. No Complexity Tracking entries required.

## Project Structure

### Documentation (this feature)

```text
specs/001-agentix-infer/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── contracts/
│   └── public-api.md    # Rust public API contract (traits, types)
└── tasks.md             # Phase 2 output (/speckit.tasks)
```

### Source Code (repository root)

```text
agentix-infer/
├── Cargo.toml
├── build.rs                      # llama.cpp + CUDA linking (llamacpp feature only)
└── src/
    ├── lib.rs                    # public re-exports: InferEngine, InferConfig, ModelInfo…
    ├── engine.rs                 # InferEngine coordinator
    ├── pool.rs                   # ContextPool (LRU, VRAM budget)
    ├── error.rs                  # InferError enum
    ├── backend/
    │   ├── mod.rs                # InferBackend + LoadedModel traits
    │   ├── llamacpp.rs           # LlamaCppBackend [feature=llamacpp]
    │   └── candle.rs             # CandleBackend [feature=candle]
    ├── store/
    │   ├── mod.rs                # ModelStore public API
    │   ├── manifest.rs           # Ollama-compatible manifest JSON + extensions
    │   ├── blob.rs               # content-addressed blob storage (SHA256)
    │   └── hf.rs                 # HuggingFace Hub download client
    └── meta/
        ├── mod.rs                # capability detection dispatch
        ├── gguf.rs               # GGUF KV metadata reader
        └── safetensors.rs        # config.json reader for safetensors models

# agentix-daemon changes (no new crate)
agentix-daemon/
├── Cargo.toml                    # add agentix-infer dependency
└── src/
    ├── gateway/
    │   ├── mod.rs                # AppState gains Arc<InferEngine>; local route wired
    │   ├── ollama_proxy.rs       # REMOVED (local route migrated to InferEngine)
    │   └── infer_handler.rs      # NEW: translates OpenAI req → InferEngine calls

# Nix changes
perSystem/packages.nix            # add cudaPackages.* to daemon buildInputs
perSystem/processes.nix           # remove Ollama requirement for daemon (keep for MCP)
```

**Structure Decision**: Single new library crate `agentix-infer` added to workspace; daemon gains one new source file and loses `ollama_proxy.rs`. No new workspace-level crates beyond `agentix-infer`.

## Implementation Phases

### Phase 1 — Store and Metadata (unblocks all other work)

Deliver `ModelStore` with Ollama-compatible blob layout, SHA256 content-addressing, HuggingFace download, and GGUF KV capability detection. No inference yet — just model lifecycle.

**Deliverables:**
- Crate scaffold (`Cargo.toml`, features: `llamacpp`, `candle`, `cuda`)
- `error.rs` — `InferError` enum covering IO, download, checksum, format, backend, pool errors
- `meta/gguf.rs` — read GGUF KV section; extract `{arch}.pooling_type`, `tokenizer.chat_template`, `{arch}.vision_encoder.*`, architecture name, context length, embedding dimension
- `meta/safetensors.rs` — read `config.json` from HF repo; map `model_type` to `Capability`
- `store/blob.rs` — write blob from stream (SHA256 computed inline), lookup by hash, verify existing
- `store/manifest.rs` — parse and write Ollama manifest JSON + agentix extension fields
- `store/hf.rs` — download specific file from HF Hub to blob store (streaming, progress, checksum)
- `store/mod.rs` — `ModelStore::pull()`, `list()`, `info()`, `remove()`, alias resolution
- Unit tests: GGUF KV parsing (sample bytes), manifest round-trip, blob checksum, HF URL construction

### Phase 2 — Backend and Pool (Phase 1 must be done)

Deliver `LlamaCppBackend`, `LoadedModel` impl, and `ContextPool`. Embedding works end-to-end.

**Deliverables:**
- `backend/mod.rs` — `InferBackend` and `LoadedModel` traits (see contracts/public-api.md)
- `backend/llamacpp.rs` — `LlamaCppBackend`: load GGUF via llama-cpp-2, embed via pooling context, stream completions via sampling loop; all blocking calls wrapped in `spawn_blocking`
- `pool.rs` — `ContextPool`: `Arc<Mutex<HashMap<String, VecDeque<Arc<dyn LoadedModel>>>>>`, LRU eviction when `total_vram_bytes > limit`, `acquire()` returns `ModelGuard` (drop releases back to pool)
- Integration test: load fixture GGUF, call `embed()`, verify non-zero vector; call `complete()`, verify token stream
- `build.rs`: link llama.cpp; CUDA via `nvcc` when `cuda` feature enabled

### Phase 3 — Engine and Daemon Integration (Phase 2 must be done)

Wire `InferEngine` into agentix-daemon. Remove Ollama proxy for local routes.

**Deliverables:**
- `engine.rs` — `InferEngine`: `Arc<RwLock<ModelStore>>` + `ContextPool` + registered backends; routes embed/complete via manifest → backend hint → pool
- `lib.rs` — public API re-exports; `InferEngine::new(InferConfig)`, `pull`, `list`, `embed`, `embed_batch`, `complete`, `tokenize`
- `agentix-daemon/Cargo.toml` — add `agentix-infer = { path = "../agentix-infer", features = ["llamacpp"] }`
- `agentix-daemon/src/gateway/mod.rs` — `AppState` gains `infer: Arc<InferEngine>`; startup calls `InferEngine::new()`
- `agentix-daemon/src/gateway/infer_handler.rs` — translate `/v1/embeddings` → `InferEngine::embed_batch`; translate local `/v1/chat/completions` → `InferEngine::complete` (streaming SSE)
- Remove `ollama_proxy.rs` local route; keep Ollama proxy only for embeddings endpoint if Ollama URL is configured (bridge period)
- Integration test: start daemon with fixture model, POST to `/v1/embeddings`, verify OpenAI-format response

### Phase 4 — Candle Backend and Laguna (independent, after Phase 3)

**Deliverables:**
- `backend/candle.rs` — `CandleBackend`: load safetensors via candle-core, support Laguna architecture
- `meta/safetensors.rs` — extended for Laguna `config.json` structure
- `store/hf.rs` — multi-file safetensors pull (model.safetensors.index.json sharding)
- Update `agentix-daemon/Cargo.toml` to add `candle` feature when targeting Laguna

## Complexity Tracking

No constitution violations. No entries required.
