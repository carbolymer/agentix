# Tasks: agentix-infer — Native Rust Inference Engine

**Input**: Design documents from `/specs/001-agentix-infer/`
**Branch**: `001-agentix-infer`
**Plan**: plan.md | **Spec**: spec.md | **Research**: research.md

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no shared dependencies)
- **[US#]**: Maps to user story from spec.md
- Tests included for all phases (integration tests required by Principle VI)

---

## Phase 1: Setup (Crate Scaffold)

**Purpose**: Wire the new crate into the workspace; establish feature flags and module skeleton.

- [ ] T001 Add `agentix-infer` to workspace `Cargo.toml` members list in `/home/sam/home/agentix/Cargo.toml`
- [ ] T002 Create `agentix-infer/Cargo.toml` with features `llamacpp` (default), `candle`, `cuda`; add deps: `llama-cpp-2` (llamacpp), `hf-hub`, `tokio`, `async-trait`, `thiserror`, `serde`/`serde_json`, `sha2`, `hex`, `tempfile`
- [ ] T003 [P] Create `agentix-infer/src/lib.rs` with empty `pub mod` declarations for `engine`, `pool`, `error`, `backend`, `store`, `meta`
- [ ] T004 [P] Create `agentix-infer/build.rs` skeleton — compile llama.cpp (delegates to llama-cpp-sys-2 via `llama-cpp-2`; add CUDA env var handling for `cuda` feature)
- [ ] T005 [P] Add `[workspace.lints]` entry enabling `clippy::unwrap_used` and `clippy::expect_used` for `agentix-infer` if not already in workspace root `Cargo.toml`

**Checkpoint**: `cargo check -p agentix-infer` passes with no errors.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core data types, error handling, blob storage, manifest parsing, and GGUF metadata reading. All user stories depend on this phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

- [ ] T006 Implement `InferError` enum in `agentix-infer/src/error.rs` covering all variants from `contracts/public-api.md`: `ModelNotFound`, `CapabilityMissing`, `NoBackend`, `ChecksumMismatch`, `DownloadFailed`, `VramExhausted`, `Backend`, `Manifest`, `Io`
- [ ] T007 [P] Implement `ModelFormat`, `BackendHint`, `Capability`, `FinishReason` enums in `agentix-infer/src/lib.rs`; derive `Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize`
- [ ] T008 [P] Implement `ModelInfo`, `InferConfig`, `CompletionRequest`, `CompletionMessage`, `CompletionChunk` structs in `agentix-infer/src/lib.rs` annotated `#[non_exhaustive]`
- [ ] T009 Implement `agentix-infer/src/store/blob.rs`: `write_blob(reader: impl Read) -> Result<(String, u64), InferError>` that streams data to a temp file, computes SHA256 inline, renames to `blobs/sha256-<hex>`; `blob_path(models_dir, hash) -> PathBuf`; `verify_blob(path, expected_hash) -> Result<(), InferError>`
- [ ] T010 [P] Implement `agentix-infer/src/store/manifest.rs`: `Manifest`, `ManifestLayer`, `ManifestConfig` structs matching Ollama JSON schema plus `_agentix` extension block; `read_manifest(path) -> Result<Manifest>` and `write_manifest(path, manifest) -> Result<()>`; digest conversion helpers `digest_to_filename(digest: &str) -> String` (`sha256:hex` → `sha256-hex`) and back
- [ ] T011 Implement `agentix-infer/src/meta/gguf.rs`: `read_gguf_metadata(path: &Path) -> Result<GgufMeta, InferError>` using `GgufContext::from_file()`; extract `general.architecture`, `{arch}.pooling_type`, `tokenizer.chat_template`, `{arch}.n_embd`, `{arch}.context_length`, `{arch}.block_count`; detect `Capability` set using rules from data-model.md; return `GgufMeta { architecture, context_length, embedding_length, capabilities, parameter_count }`
- [ ] T012 [P] Implement `agentix-infer/src/meta/safetensors.rs`: `read_safetensors_metadata(config_json: &[u8]) -> Result<SafetensorsMeta, InferError>`; parse `model_type` field and map to `Vec<Capability>` using table from data-model.md
- [ ] T013 [P] Implement `agentix-infer/src/meta/mod.rs`: `detect_capabilities(path: &Path, format: ModelFormat) -> Result<Vec<Capability>, InferError>` dispatching to gguf.rs or safetensors.rs
- [ ] T014 Write unit tests in `agentix-infer/src/store/blob.rs` (cfg test): write blob, verify hash, detect checksum mismatch
- [ ] T015 [P] Write unit tests in `agentix-infer/src/store/manifest.rs` (cfg test): round-trip serialize/deserialize Ollama manifest JSON; verify `_agentix` extension fields survive; verify digest conversion
- [ ] T016 [P] Write unit tests in `agentix-infer/src/meta/gguf.rs` (cfg test): parse sample GGUF KV bytes; verify pooling_type present → Embedding capability; verify missing pooling_type → conservative fallback + warning

**Checkpoint**: `cargo test -p agentix-infer` passes for all unit tests; `cargo clippy -p agentix-infer -- -D warnings` clean.

---

## Phase 3: User Story 2 — Pull a Model from HuggingFace (Priority: P1)

**Goal**: Operators can pull GGUF and safetensors models from HuggingFace Hub into the blob store with SHA256 verification.

**Independent Test**: Pull `second-state/All-MiniLM-L6-v2-Embedding-GGUF` Q8_0 via `ModelStore::pull("hf.co/second-state/All-MiniLM-L6-v2-Embedding-GGUF:all-MiniLM-L6-v2-Q8_0.gguf")`. Verify blob appears at correct path, SHA256 matches, manifest lists the model. Then call `ModelStore::list()` and confirm the model appears.

- [ ] T017 [US2] Implement `agentix-infer/src/store/hf.rs`: `HfClient` struct wrapping `hf_hub::api::sync::ApiBuilder`; `parse_hf_ref(model_ref: &str) -> Result<HfRef>` parsing `hf.co/{org}/{repo}:{file}` and `hf.co/{org}/{repo}` formats; `HfClient::download_to_blob_store(hf_ref: &HfRef, store_dir: &Path) -> Result<(String, u64), InferError>` that downloads via `repo.get()`, reads from hf-hub cache, streams through `write_blob()`, returns (sha256_hash, size_bytes)
- [ ] T018 [US2] Implement `agentix-infer/src/store/mod.rs` `ModelStore` struct: `ModelStore::new(models_dir: PathBuf) -> Self`; `pull(model_ref: &str) -> Result<ModelInfo>` that (a) detects source type from ref, (b) downloads to blob store, (c) reads GGUF/safetensors metadata via `meta::detect_capabilities`, (d) writes manifest with `_agentix` extension block, (e) returns `ModelInfo`
- [ ] T019 [US2] Implement `ModelStore::list() -> Vec<ModelInfo>` by walking `manifests/` directory tree and deserializing each manifest
- [ ] T020 [P] [US2] Implement `ModelStore::info(name: &str) -> Option<ModelInfo>` — look up manifest by name/alias
- [ ] T021 [P] [US2] Implement `ModelStore::remove(name: &str) -> Result<(), InferError>` — remove manifest; if blob SHA is not referenced by any other manifest, delete blob file
- [ ] T022 [US2] Implement alias resolution: `ModelStore::resolve(name: &str) -> Option<PathBuf>` returning the primary model blob path; alias table stored in `manifests/_aliases.json`
- [ ] T023 [US2] Integration test in `agentix-infer/tests/store_integration.rs`: use fixture GGUF from `AGENTIX_TEST_MODEL_PATH` env var; call `ModelStore::pull()` with local file path ref; verify blob written, manifest readable, `list()` returns the model, `remove()` cleans up

**Checkpoint**: `cargo test -p agentix-infer -- store_integration` passes. ModelStore can pull, list, and remove models.

---

## Phase 4: User Story 1 — Embeddings Without Ollama (Priority: P1)

**Goal**: agentix-daemon generates embeddings in-process for any GGUF embedding model. Ollama does not need to be running.

**Independent Test**: Start agentix-daemon (or call `InferEngine::embed()` directly in a test) with AGENTIX_MODELS_DIR pointing at a directory containing the fixture GGUF. Receive a non-empty `Vec<f32>` with correct dimension. Verify with Ollama not installed.

- [ ] T024 [US1] Define `InferBackend` and `LoadedModel` traits in `agentix-infer/src/backend/mod.rs` exactly as in `contracts/public-api.md`; add `#[async_trait]`; include `ModelGuard` struct (wraps `Arc<dyn LoadedModel>`, releases to pool on drop)
- [ ] T025 [US1] Implement `LlamaCppLoadedModel` in `agentix-infer/src/backend/llamacpp.rs` for embeddings: holds `Arc<LlamaModel>` + `Mutex<LlamaContext>`; `embed(&self, input: &str) -> Result<Vec<f32>>` — inside `spawn_blocking`: tokenize input, build `LlamaBatch`, call `ctx.encode()`, read `ctx.embeddings_seq_ith(0)`, return owned `Vec<f32>`; `vram_bytes()` returns context size estimate
- [ ] T026 [US1] Implement `LlamaCppBackend` struct in `agentix-infer/src/backend/llamacpp.rs`: holds a singleton `Arc<LlamaBackend>` (init once); `load(blob_path, info) -> Arc<dyn LoadedModel>` via `spawn_blocking` — calls `LlamaModel::load_from_file()` with GPU layers from config, then `model.new_context()` with `with_embeddings(true)` and `pooling_type` from manifest or fallback to `Unspecified`; `supports_format(Gguf) -> true`
- [ ] T027 [US1] Implement `ContextPool` in `agentix-infer/src/pool.rs`: `HashMap<String, Vec<PoolSlot>>` under `Mutex`; `acquire(name, backend, blob_path, info) -> Result<ModelGuard>` — returns idle slot or loads new one, evicts LRU when `total_vram > limit`, serializes concurrent loads of the same model via per-model `Arc<Notify>`; `release(name, slot)` — returns slot to idle list, updates `last_used`
- [ ] T028 [US1] Implement `InferEngine` in `agentix-infer/src/engine.rs`: holds `RwLock<ModelStore>`, `ContextPool`, `Vec<Arc<dyn InferBackend>>`; `new(config)` initializes store and pool; `register_backend(backend)`; `embed(model, input) -> Result<Vec<f32>>` — resolves model → blob path via store, selects backend from manifest hint or `supports_format()`, acquires from pool, calls `loaded.embed(input)`, releases guard
- [ ] T029 [US1] Expose `InferEngine::embed_batch(model, inputs) -> Result<Vec<Vec<f32>>>` in `agentix-infer/src/engine.rs` — calls `loaded.embed_batch(inputs)` which tokenizes all inputs and calls `embed_batch` on the model
- [ ] T030 [US1] Implement `LlamaCppLoadedModel::embed_batch` — batch tokenize, build multi-sequence `LlamaBatch`, call `ctx.encode()`, read `ctx.embeddings_seq_ith(i)` for each sequence
- [ ] T031 [US1] Wire daemon: add `agentix-infer = { path = "../agentix-infer", features = ["llamacpp"] }` to `agentix-daemon/Cargo.toml`; add `infer: Arc<InferEngine>` to `AppState` in `agentix-daemon/src/gateway/mod.rs`; initialize in `agentix-daemon/src/main.rs` via `InferEngine::new(InferConfig { models_dir, .. })` and `register_backend(Arc::new(LlamaCppBackend::new()))`
- [ ] T032 [US1] Implement `agentix-daemon/src/gateway/infer_handler.rs` embeddings handler: parse `/v1/embeddings` OpenAI request body → call `state.infer.embed_batch(model, inputs)` → format as OpenAI embeddings response `{ object: "list", data: [{ object: "embedding", index, embedding }], model, usage }`
- [ ] T033 [US1] Route `/v1/embeddings` in `agentix-daemon/src/gateway/mod.rs` to `infer_handler` when the model name is present in the InferEngine's store; fall back to Ollama proxy if model is not locally registered (bridge period — Ollama proxy kept for non-local models)
- [ ] T034 [US1] Integration test in `agentix-infer/tests/embed_integration.rs`: load fixture GGUF from `AGENTIX_TEST_MODEL_PATH`, create `InferEngine` with `LlamaCppBackend`, call `embed("test-model", "hello world")`, assert `vec.len() == 384` (all-MiniLM embedding dim), assert no element is NaN
- [ ] T035 [US1] Add Nix flake fixture model pin: add `agentix-test-model` derivation to `perSystem/packages.nix` using `fetchurl` with SHA256 for `all-MiniLM-L6-v2-Q8_0.gguf`; set `AGENTIX_TEST_MODEL_PATH` in the dev shell and CI environment

**Checkpoint**: `cargo test -p agentix-infer -- embed_integration` passes. POST to `/v1/embeddings` on a running daemon with the fixture model returns a valid embedding vector.

---

## Phase 5: User Story 3 — Local Completion Without Ollama (Priority: P2)

**Goal**: agentix-daemon streams chat completions from local GGUF models. Ollama does not need to be running.

**Independent Test**: POST streaming request to `/v1/chat/completions` with a locally registered GGUF completion model. Receive a valid SSE stream ending in `data: [DONE]`. Ollama not installed.

- [ ] T036 [US3] Implement `LlamaCppLoadedModel::complete(req: CompletionRequest) -> Result<Pin<Box<dyn Stream<...>>>>` in `agentix-infer/src/backend/llamacpp.rs`: apply chat template via model token formatter, tokenize, then spawn a `tokio::sync::mpsc::channel` + `spawn_blocking` thread that runs the sampling loop (`ctx.decode() → sampler.sample() → sampler.accept() → token_to_str()` → send chunk); convert `mpsc::Receiver` to `async_stream::stream!` or `tokio_stream::wrappers::ReceiverStream`; send `FinishReason::Stop` on EOS or `FinishReason::Length` on `max_tokens`
- [ ] T037 [US3] Handle `Capability::Completion` guard in `LlamaCppLoadedModel::complete`: if manifest `capabilities` does not include `Completion`, return `InferError::CapabilityMissing` immediately before any work
- [ ] T038 [US3] Implement `InferEngine::complete(model, req) -> Result<impl Stream<...>>` in `agentix-infer/src/engine.rs`: same acquire/release pattern as embed; delegate to `loaded.complete(req)`
- [ ] T039 [US3] Implement `agentix-daemon/src/gateway/infer_handler.rs` completion handler: parse `/v1/chat/completions` body; if model is local (InferEngine has it), call `state.infer.complete(model, req)`; for streaming: convert chunk stream to SSE (`data: {json}\n\n`) ending with `data: [DONE]\n\n`; for non-streaming: collect all chunks, return OpenAI chat completion JSON
- [ ] T040 [US3] Route local model `/v1/chat/completions` in `agentix-daemon/src/gateway/mod.rs` to `infer_handler`; Anthropic/OpenAI/OpenRouter routes continue to go through existing proxy code (no change to `RouteTarget::Anthropic` etc.)
- [ ] T041 [US3] Remove `agentix-daemon/src/gateway/ollama_proxy.rs` local chat-completion route; keep only the Ollama embeddings proxy fallback (for models not registered in InferEngine) until Phase 3 step T033 bridge is ready
- [ ] T042 [US3] Integration test in `agentix-infer/tests/complete_integration.rs`: load fixture GGUF (all-MiniLM will not pass since it's embedding-only — use a small completion GGUF if available, otherwise test `CapabilityMissing` error path); mock a `CompletionRequest`, call `engine.complete()`, collect stream, assert at least one non-empty chunk received; assert `FinishReason::Stop` in final chunk

**Checkpoint**: Streaming completions work end-to-end through the daemon for locally registered GGUF models. Non-local models (Anthropic, OpenAI, OpenRouter) continue to work unchanged.

---

## Phase 6: User Story 4 — Candle Backend for Safetensors (Priority: P3)

**Goal**: Safetensors models (e.g. Laguna) load and serve completions without GGUF conversion.

**Independent Test**: Pull Laguna XS.2 safetensors from HuggingFace. POST to `/v1/chat/completions`. Receive valid completion. Candle feature flag enabled.

- [ ] T043 [US4] Enable `candle` Cargo feature in `agentix-infer/Cargo.toml`: add `candle-core`, `candle-nn`, `candle-transformers`, `hf-hub` under the `candle` feature
- [ ] T044 [US4] Implement `agentix-infer/src/meta/safetensors.rs` fully: read `config.json` from HF repo (download separately alongside safetensors shards); parse `model_type` and `num_hidden_layers`, `hidden_size` etc. for `SafetensorsMeta`
- [ ] T045 [US4] Implement multi-file safetensors pull in `agentix-infer/src/store/hf.rs`: detect `model.safetensors.index.json` sharding; download each shard; store each as a separate blob; write manifest with multiple layers, one per shard
- [ ] T046 [US4] Implement `CandleBackend` in `agentix-infer/src/backend/candle.rs` (feature=candle): `load(blob_paths, info) -> Arc<dyn LoadedModel>` — load safetensors weights via `candle_core::safetensors::load()`; `supports_format(Safetensors) -> true`
- [ ] T047 [US4] Implement `CandleLoadedModel::complete(req)` for Laguna architecture via `candle_transformers`; use `spawn_blocking` for forward pass
- [ ] T048 [US4] Update `agentix-daemon/Cargo.toml` to enable `candle` feature when targeting Laguna; update startup backend registration to include `CandleBackend` when `candle` feature is active

**Checkpoint**: Laguna XS.2 loads and serves completions without any GGUF conversion.

---

## Phase 7: Polish & Cross-Cutting Concerns

- [ ] T049 [P] Update `perSystem/packages.nix` to add `cudaPackages.cudatoolkit`, `cudaPackages.libcublas`, `clang`, `libclang.lib`, `cmake` to `agentix-daemon` build inputs for the `cuda` feature build
- [ ] T050 [P] Update `perSystem/processes.nix` to remove the Ollama start requirement for `agentix-daemon` (Ollama still needed for MCP server embeddings until MCP server is also migrated)
- [ ] T051 [P] Update `agentix-daemon/src/gateway/health.rs` to include inference engine status: list loaded backends, model count, and whether InferEngine is active
- [ ] T052 [P] Update `README.md` environment variables table: add `AGENTIX_MODELS_DIR` (default `/var/lib/agentix/models`), `AGENTIX_VRAM_LIMIT_BYTES`, `AGENTIX_MAX_LOADED_MODELS`; remove Ollama as a daemon hard dependency
- [ ] T053 Run `cargo fmt --check --workspace` and fix any formatting issues across agentix-infer and agentix-daemon changes
- [ ] T054 Run `cargo clippy --workspace -- -D warnings` and resolve all warnings; add `// SAFETY:` comments to any `unsafe` blocks in llama.cpp FFI boundary code
- [ ] T055 Commit `agentix-infer-prd.md` to repo (currently untracked): `git add agentix-infer-prd.md`
- [ ] T056 [P] Update `ARCHITECTURE.md` crate inventory to include `agentix-infer` with its dependency edges and purpose

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — start immediately
- **Foundational (Phase 2)**: Requires Phase 1 completion — blocks all user stories
- **US2 Pull (Phase 3)**: Requires Phase 2 — ModelStore::pull needs blob.rs + manifest.rs + meta/gguf.rs
- **US1 Embeddings (Phase 4)**: Requires Phase 2 (blob store, manifest, GGUF meta); T031–T033 require Phase 3 (ModelStore for engine to use); T024–T030 and T034 can start after Phase 2
- **US3 Completion (Phase 5)**: Requires Phase 4 completion (pool, engine, daemon wiring in place)
- **US4 Candle (Phase 6)**: Requires Phase 2; independent of Phases 3–5 (can run in parallel once foundational is done)
- **Polish (Phase 7)**: Requires Phases 4 and 5; T049/T050 can run earlier

### User Story Dependencies

- **US2 (Pull)**: Can start after Phase 2 — no dependency on US1, US3, US4
- **US1 (Embeddings)**: Backend/pool/engine work (T024–T030) can start after Phase 2; daemon wiring (T031–T035) requires Phase 3 ModelStore
- **US3 (Completion)**: Requires US1 (engine and daemon wiring in place)
- **US4 (Candle)**: Independent of US1–US3; runs in parallel with US2 once Phase 2 is done

### Parallel Opportunities

Within Phase 2: T006, T007, T008, T010, T012, T013, T014, T015, T016 are all different files.
Within Phase 4: T024 (traits), T025–T026 (LlamaCpp impl), T035 (Nix fixture) can proceed in parallel.
T049 (Nix CUDA packages) can be done any time after Phase 1.

---

## Parallel Example: Phase 4 (US1 Embeddings)

```
# After Phase 2 completes, these can start in parallel:
Task T024: Define InferBackend + LoadedModel traits in agentix-infer/src/backend/mod.rs
Task T026: LlamaCppBackend::load() in agentix-infer/src/backend/llamacpp.rs
Task T027: ContextPool in agentix-infer/src/pool.rs
Task T035: Nix fixture model pin in perSystem/packages.nix

# Then sequentially:
Task T025: LlamaCppLoadedModel::embed()   (needs T024 trait + T026 backend struct)
Task T028: InferEngine                    (needs T025, T027)
Task T029: InferEngine::embed_batch       (needs T028)
Task T031: Daemon AppState wiring         (needs T028)
Task T032: infer_handler embeddings       (needs T031)
Task T033: Route /v1/embeddings           (needs T032)
Task T034: Integration test               (needs T028, T035)
```

---

## Implementation Strategy

### MVP (User Story 2 + User Story 1 only)

1. Complete Phase 1: Setup
2. Complete Phase 2: Foundational (CRITICAL — blocks all stories)
3. Complete Phase 3: US2 — model pull from HuggingFace
4. Complete Phase 4: US1 — embeddings end-to-end through daemon
5. **STOP and VALIDATE**: `POST /v1/embeddings` works with Ollama not installed
6. Ship: agentix-daemon can run with no Ollama dependency for the embedding path

### Full Phase 1 Delivery (adds US3)

7. Complete Phase 5: US3 — streaming completions
8. **VALIDATE**: `POST /v1/chat/completions` (local model) works with Ollama not installed
9. Remove `ollama_proxy.rs` entirely

### Phase 2 Delivery (adds US4, independent)

10. Complete Phase 6: US4 — Candle backend, Laguna support
11. **VALIDATE**: Laguna XS.2 serves completions

---

## Notes

- All blocking C FFI calls (llama.cpp) MUST use `tokio::task::spawn_blocking` — architectural invariant (Principle VI)
- `LlamaContext` is `!Send`; hold it behind `Mutex<LlamaContext>` inside `LlamaCppLoadedModel`
- `bert.pooling_type` KV key may be absent in older GGUF conversions — always fall back to `LlamaPoolingType::Unspecified` with a warning (never panic)
- Blob digest in JSON uses `sha256:hex` (colon); filename on disk uses `sha256-hex` (hyphen) — Ollama convention
- Integration tests read fixture model from `AGENTIX_TEST_MODEL_PATH` env var; set by Nix dev shell
- Commit `agentix-infer-prd.md` (T055) before or alongside the first code commits
