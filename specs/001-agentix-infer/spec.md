# Feature Specification: agentix-infer — Native Rust Inference Engine

**Feature Branch**: `001-agentix-infer`
**Created**: 2026-08-07
**Status**: Draft
**Input**: Build agentix-infer: a native Rust inference library that replaces the Ollama HTTP proxy in agentix-daemon.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Embeddings Without Ollama (Priority: P1)

An operator running agentix-daemon wants to generate code embeddings for the MCP search index without Ollama installed. Today, the daemon proxies embedding requests over HTTP to a local Ollama process, which must be installed, running, and on a compatible version. After this feature, the daemon generates embeddings in-process: no separate Ollama process, no HTTP round-trip, no version compatibility risk.

**Why this priority**: This is the immediate unblock. The jina-code embedding model broke when Ollama upgraded from 0.22 to 0.32 due to capability detection changes outside our control. Embedding is the simpler of the two inference paths (no streaming, fixed output size) and proves out the architecture with low risk.

**Independent Test**: Start agentix-daemon with Ollama not installed or not running. Issue a POST to `/v1/embeddings` with the jina-code model. Receive a valid embedding vector response. The value delivered: the MCP search pipeline works without any Ollama dependency.

**Acceptance Scenarios**:

1. **Given** Ollama is not installed on the host, **When** agentix-daemon starts with an AGENTIX_MODELS_DIR pointing to a directory containing jina-code GGUF blobs, **Then** the daemon starts successfully and reports the model as available.
2. **Given** agentix-daemon is running with jina-code loaded, **When** a client sends POST `/v1/embeddings` with `{"model":"jina-code","input":"func tokenize(s string) []string"}`, **Then** the response is a valid OpenAI-format embedding response with a non-empty float vector.
3. **Given** no model is loaded for the requested name, **When** a client requests embeddings, **Then** the daemon returns a 404 with a clear error indicating the model is not found.

---

### User Story 2 - Pull a Model from HuggingFace (Priority: P1)

An operator wants to download and register a GGUF model from HuggingFace Hub so the inference engine can use it. They issue a pull command (via CLI or daemon API) specifying the HF repo and file. The model is downloaded, its checksum is verified, and it becomes immediately available for inference.

**Why this priority**: Model acquisition is a prerequisite for any inference. Operators need a reliable, auditable way to get models onto the host without depending on Ollama's registry.

**Independent Test**: Issue a pull request for `hf.co/jinaai/jina-code-embeddings-1.5b-GGUF:Q8_0`. Verify the blob appears in AGENTIX_MODELS_DIR with correct SHA256, and that a subsequent `/v1/models` listing includes the model. Value delivered: operators can provision models without Ollama.

**Acceptance Scenarios**:

1. **Given** an empty models directory, **When** an operator pulls `hf.co/jinaai/jina-code-embeddings-1.5b-GGUF:Q8_0`, **Then** the GGUF file is downloaded, stored content-addressed by SHA256, and a manifest is written mapping the model name to the blob.
2. **Given** a model is already downloaded, **When** the same pull is issued again, **Then** the existing blob is reused (no re-download) and the manifest is updated if needed.
3. **Given** a download completes with a checksum mismatch, **When** the daemon attempts to register the blob, **Then** the blob is deleted and the pull fails with an error indicating checksum failure.
4. **Given** a model GGUF is already present in an Ollama-compatible blob store, **When** the daemon starts with AGENTIX_MODELS_DIR pointing to that store, **Then** the existing blobs are readable without re-downloading.

---

### User Story 3 - Local Completion Without Ollama (Priority: P2)

An operator running local code-completion workflows wants agentix-daemon to serve chat completions via a locally loaded GGUF model (e.g., qwen2.5-coder:14b or DeepSeek-R1) without Ollama. The OpenAI-compatible `/v1/chat/completions` endpoint should route local model requests to the in-process backend, with streaming support.

**Why this priority**: Chat completion is the other major inference path. Once embeddings are proven (P1), completion completes the Ollama removal. Streaming is required because agent loops consume token-by-token output.

**Independent Test**: Start agentix-daemon with a qwen2.5-coder GGUF loaded. Issue a streaming POST to `/v1/chat/completions`. Receive a server-sent-events stream with valid completion chunks. Value delivered: local agent sessions work end-to-end without Ollama.

**Acceptance Scenarios**:

1. **Given** a qwen2.5-coder GGUF is loaded, **When** a client sends a non-streaming POST `/v1/chat/completions` request, **Then** the response is a valid OpenAI chat completion object.
2. **Given** a GGUF completion model is loaded, **When** a client sends a streaming POST with `"stream":true`, **Then** the response is a valid SSE stream with `data: {...}` chunks ending in `data: [DONE]`.
3. **Given** a model that only supports embeddings (no chat template), **When** a completion request is issued for that model, **Then** the daemon returns a 400 error indicating the model does not support completion.

---

### User Story 4 - Candle Backend for Safetensors Models (Priority: P3)

An operator wants to load a Laguna model (safetensors format) that has no GGUF release. The inference engine detects the format, routes to the Candle backend automatically, and serves completions.

**Why this priority**: Laguna uses a custom MoE architecture not yet supported by llama.cpp. The Candle backend unlocks frontier models that will never have GGUF releases. This is Phase 2 and depends on Phase 1 being stable.

**Independent Test**: Pull Laguna XS.2 from HuggingFace in safetensors format. Issue a completion request. Receive a valid response. Laguna completions are served without any GGUF conversion step.

**Acceptance Scenarios**:

1. **Given** Laguna XS.2 safetensors blobs are present in the model store, **When** a completion request is issued naming that model, **Then** the Candle backend is selected automatically based on the manifest format field.
2. **Given** a safetensors model is requested and the Candle feature is not compiled in, **When** the daemon starts, **Then** it logs a warning that the model requires the Candle backend but refuses to start with a clear error rather than silently falling back.

---

### Edge Cases

- What happens when VRAM is exhausted and a new model load is requested? The pool must evict the least-recently-used loaded model to free space before loading the new one, and must report the eviction in logs.
- What happens if the download of a large model is interrupted mid-stream? The partial blob must not be registered; on retry the download resumes or restarts cleanly.
- What happens if two concurrent requests arrive for a model that is not yet loaded? The pool must serialize the load — only one load attempt proceeds; the second request waits on the same future.
- What happens when the requested model name is an alias? The alias must be resolved to the canonical manifest before backend selection.
- What happens if a GGUF KV section is malformed or missing expected capability keys? The system must fall back to conservative capability assumptions (Completion-only) and log a warning rather than failing the pull.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The inference library MUST be a pure Rust library crate with no network listening surface of its own; all HTTP is owned by agentix-daemon.
- **FR-002**: The model store MUST use an Ollama-compatible content-addressed blob layout (blobs/sha256-\<hash\>, manifests/\<registry\>/\<path\>) so existing Ollama-managed models are usable without re-downloading.
- **FR-003**: The model store MUST pull GGUF and safetensors models from HuggingFace Hub, verifying SHA256 after download.
- **FR-004**: The library MUST detect model capabilities (Completion, Embedding, Vision) by reading GGUF KV metadata or safetensors config.json; no hardcoded architecture lists are permitted.
- **FR-005**: The library MUST expose a single `InferBackend` trait so new backends can be added without modifying the engine or model store.
- **FR-006**: Phase 1 MUST include a GGUF backend using the llama.cpp library, gated behind a `llamacpp` Cargo feature (default on).
- **FR-007**: Phase 2 MUST include a safetensors backend using the Candle library, gated behind a `candle` Cargo feature (off by default until Phase 2).
- **FR-008**: A `cuda` Cargo feature MUST enable GPU acceleration for both backends without changing the public API.
- **FR-009**: The context pool MUST maintain warm loaded-model instances and evict by LRU when VRAM usage exceeds a configurable threshold.
- **FR-010**: agentix-daemon MUST route local model requests to the in-process engine rather than proxying to Ollama; the external OpenAI-compatible API surface MUST remain unchanged.
- **FR-011**: The `/v1/embeddings`, `/v1/chat/completions` (local route), and model management endpoints MUST continue to operate correctly after the Ollama proxy is removed.
- **FR-012**: agentix-daemon MUST start successfully when Ollama is not installed.

### Key Entities

- **ModelBlob**: A raw model file (GGUF or safetensors) stored content-addressed by SHA256 under `blobs/`.
- **ModelManifest**: A JSON file mapping a model name (registry/name:tag) to one or more blobs, with metadata fields including format, backend hint, and detected capabilities.
- **ModelInfo**: The runtime view of a registered model — name, architecture, format, detected capabilities, parameter count, context length, embedding dimension, quantization, blob size.
- **InferBackend**: The trait interface a backend must implement — load a blob into a `LoadedModel`, and report which formats it supports.
- **LoadedModel**: A trait representing a warm in-memory model instance — supports embed, embed_batch, complete (streaming), tokenize, and VRAM usage reporting.
- **ContextPool**: Tracks active `LoadedModel` instances per model name; grants access via acquire/release; enforces VRAM budget.
- **InferEngine**: Top-level coordinator holding the model store, pool, and registered backends; exposes the public API consumed by agentix-daemon.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: agentix-daemon starts successfully on a host with Ollama not installed and serves requests for models present in AGENTIX_MODELS_DIR.
- **SC-002**: jina-code embedding requests complete end-to-end with no Ollama process running, and the returned vectors are numerically identical (within floating-point tolerance) to those previously produced via Ollama for the same inputs.
- **SC-003**: Embedding latency for inputs under 512 tokens is lower than the baseline Ollama-proxy path (measured: prior path adds two HTTP round-trips and two process boundaries; target is measurably lower p50).
- **SC-004**: A GGUF model pull from HuggingFace Hub completes with correct SHA256 verification and the model is available for inference within the same daemon session.
- **SC-005**: DeepSeek-R1 and qwen2.5-coder GGUF models load and serve streaming completions via the llama.cpp backend.
- **SC-006** *(Phase 2)*: Laguna XS.2 loads and serves completions via the Candle backend with no GGUF conversion step.
- **SC-007**: Adding support for a new model architecture requires no changes to InferEngine, ModelStore, or ContextPool — only a backend implementation or capability detection extension.
- **SC-008**: All existing OpenAI-compatible API clients that previously targeted Ollama through the daemon continue to work without configuration changes.

## Assumptions

- Operators have sufficient VRAM or RAM to load the models they request; the engine reports load failures clearly but does not dynamically split layers across devices in Phase 1.
- The jina-code GGUF on HuggingFace Hub contains the KV metadata fields (`{arch}.pooling_type`) needed for capability detection; if not, Phase 1 will include a fallback manifest override mechanism.
- Phase 1 does not include migration tooling for moving existing Ollama blob stores — operators point AGENTIX_MODELS_DIR at the existing Ollama models directory directly.
- The Candle backend (Phase 2) is sequenced independently and does not block Phase 1 delivery; Phase 1 is complete when SC-001 through SC-005 and SC-007 through SC-008 pass.
- llama.cpp blocking C FFI calls are wrapped with `tokio::task::spawn_blocking`; callers treat the engine as fully async.
- The Nix flake build gains CUDA toolkit inputs for the daemon derivation; non-CUDA builds remain possible by omitting the `cuda` feature.
