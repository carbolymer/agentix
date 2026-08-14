# Tasks: Whisper Integration

**Input**: Design documents from `/specs/006-whisper-integration/`
**Architecture note**: `agentix-whisper` is a new crate that is BOTH a library (WhisperBackend,
audio decoding) AND a standalone binary (Unix socket HTTP server). The daemon does NOT link
whisper — it returns 501 on the transcription endpoint until spec 010 wires up the socket proxy.

**Organization**: Tasks are grouped by phase.

---

## Phase 1: Setup (Shared Infrastructure — agentix-infer, pure Rust only)

- [x] T001 Add `Capability::Transcription` variant to `Capability` enum in `agentix-infer/src/lib.rs`
- [x] T002 Add `BackendHint::Whisper` variant to `BackendHint` enum in `agentix-infer/src/lib.rs`
- [x] T003 Add `ModelFormat::WhisperBin` variant to `ModelFormat` enum in `agentix-infer/src/lib.rs`
- [x] T004 Add `InferError::Transcription(String)` variant to `InferError` in `agentix-infer/src/error.rs`
- [x] T005 Add `async fn transcribe(&self, pcm: &[f32]) -> Result<String, InferError>` default method to `LoadedModel` trait (returns `CapabilityMissing`; existing backends need no changes)
- [x] T006 Add `TranscriptionResponse { text: String }` struct to `agentix-api/src/lib.rs`
- [x] T007 Update all exhaustive match arms on `Capability`, `BackendHint`, and `ModelFormat` to handle new variants

**Checkpoint**: `cargo build --workspace` green.

---

## Phase 2: Foundational (GGUF + .bin detection — agentix-infer, pure Rust only)

- [x] T008 Add `backend_hint: Option<BackendHint>` field to `GgufMeta` in `agentix-infer/src/meta/gguf.rs`
- [x] T009 In `read_gguf_metadata()`: add early-return for `architecture == "whisper"` → `[Capability::Transcription], BackendHint::Whisper`
- [x] T010 Add unit test for whisper GGUF early-return logic
- [x] T011 In `detect_format()`: map `.bin` → `ModelFormat::WhisperBin`; in store: when format is `WhisperBin` or GGUF with `BackendHint::Whisper`, use `BackendHint::Whisper` in manifest
- [x] T012 In `engine.rs` `load_model()`: select backend by hint name first, falling back to `supports_format()` — prevents LlamaCpp from claiming whisper GGUF files
- [x] T013 Add `pub async fn transcribe_pcm(&self, model: &str, pcm: &[f32]) -> Result<String, InferError>` to `InferEngine`
- [x] T014 Add `pub async fn warmup(&self, model: &str) -> Result<(), InferError>` to `InferEngine`

**Checkpoint**: `cargo test -p agentix-infer` green.

---

## Phase 3: agentix-whisper Crate (C++ backend + standalone binary)

**Purpose**: New crate that mirrors `agentix-llama`. Implements `InferBackend` (library) AND
runs a standalone HTTP server on a Unix socket (binary). No C++ in the daemon.

- [x] T015 Add `agentix-whisper` to workspace `members` in root `Cargo.toml`
- [x] T016 Create `agentix-whisper/Cargo.toml`: lib + `[[bin]] name = "agentix-whisper"`; deps include `agentix-api`, `agentix-infer`, `whisper-rs`, `symphonia`, `rubato`, `tokio/full`, `axum/multipart`, `anyhow`, `serde_json`, `tracing-subscriber`; optional `cuda` feature
- [x] T017 Create `agentix-whisper/build.rs`: CUDA env passthrough (mirrors agentix-llama)
- [x] T018 Create `agentix-whisper/src/audio.rs`: `decode_audio_to_pcm` — symphonia decode + downmix + rubato resample to 16 kHz mono f32, all in `spawn_blocking`
- [x] T019 Audio unit tests: 44100 Hz sine → 16 kHz (verify length), 16 kHz passthrough (no resample)
- [x] T020 Create `agentix-whisper/src/lib.rs`: `WhisperBackend` implementing `InferBackend`; `WhisperLoadedModel` with `Arc<WhisperContext>`; `transcribe()` creates `WhisperState` in `spawn_blocking`; `unsafe impl Send/Sync` with SAFETY comments
- [x] T021 Create `agentix-whisper/src/main.rs`: standalone HTTP server on `AGENTIX_WHISPER_SOCKET` (default `/run/agentix/whisper.sock`); serves `POST /v1/audio/transcriptions` (real implementation) and `POST /control/shutdown` (graceful drain-and-exit); pre-loads `AGENTIX_WHISPER_MODEL` at startup
- [x] T021a Add model management endpoints to `agentix-whisper/src/main.rs`: `POST /api/pull` (register local path or pull from HuggingFace), `GET /v1/models` (list registered whisper models), `DELETE /api/delete` (remove a model from the store)
- [x] T022 Add integration test in `agentix-whisper/tests/`: `#[ignore]` test using `AGENTIX_TEST_WHISPER_MODEL_PATH`

**Checkpoint**: `cargo build -p agentix-whisper` succeeds (lib + binary); audio unit tests pass; integration test skipped without fixture.

---

## Phase 4: Daemon Wiring (stub only)

**Purpose**: Register the route but return 501 — no C++ in the daemon, no ggml collision.
Real proxy wired in spec 010-socket-backends.

- [x] T023 Ensure `agentix-daemon/Cargo.toml` does NOT depend on `agentix-whisper`; axum has only `json` feature (no `multipart`)
- [x] T024 Delete `agentix-daemon/build.rs` — `--allow-multiple-definition` no longer needed
- [x] T025 `agentix-daemon/src/gateway/transcription_handler.rs` returns `501 Not Implemented` with message pointing to spec 010
- [x] T026 Route `POST /v1/audio/transcriptions` registered in `mod.rs` (keeps the endpoint visible in API surface)

**Checkpoint**: `cargo build -p agentix-daemon` succeeds with no C++ whisper symbols; route exists and returns 501.

---

## Phase 5: Nix Packaging

- [x] T027 Add `src/main.rs` stub for `agentix-whisper` in both `depsOnlySrc` and `mkBinSrc` shell scripts in `perSystem/packages.nix`
- [x] T028 Remove `daemonCargoArtifacts` (whisper no longer in daemon build)
- [x] T029 Add `agentixWhisperPkg = craneLib.buildPackage ... --package agentix-whisper` and export as `packages.agentix-whisper`
- [x] T030 Update `agentixDaemonPkg`: remove `--features whisper`, remove `whisperCudaEnv`, use `cargoArtifacts` (not `daemonCargoArtifacts`) for CPU builds
- [x] T031 Remove `whisperAlwaysOn` option from `flake/nixosModules/agentix.nix` daemon module (moves to agentix-whisper service in spec 010)
- [x] T032 Retain pinned `agentixTestWhisperModel` FOD for use in spec 010 integration tests

**Checkpoint**: `nix build .#agentix-daemon` and `nix build .#agentix-whisper` both succeed.

---

## Phase 6: Polish

- [x] T033 `cargo fmt --check` passes
- [x] T034 `cargo clippy -p agentix-infer -p agentix-whisper -p agentix-daemon -- -D warnings` clean
- [x] T035 `cargo test -p agentix-infer -p agentix-whisper` passes (integration test ignored without fixture)

---

## Notes

- `agentix-infer` stays **pure Rust** — no whisper-rs, no symphonia.
- The daemon links only `agentix-llama` for C++. No `--allow-multiple-definition` needed.
- The whisper binary shuts down via `POST /control/shutdown` — no systemctl privileges required.
- Audio decoding (symphonia + rubato) lives in `agentix-whisper/src/audio.rs`.
- Spec 010-socket-backends wires the daemon proxy and adds the NixOS socket-activated service.
