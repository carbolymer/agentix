# Tasks: Socket-Activated Backends

**Input**: `specs/010-socket-backends/plan.md`
**Prerequisites**: spec 006-whisper-integration merged (agentix-whisper binary exists with
/control/shutdown); spec 007-cargo-cleanup merged

---

## Phase 1: agentix-llama Binary

**Purpose**: Give agentix-llama its own standalone HTTP server binary, mirroring agentix-whisper.

- [ ] T001 Add `[[bin]] name = "agentix-llama" path = "src/main.rs"` to `agentix-llama/Cargo.toml`; add `axum/json`, `anyhow`, `serde_json`, `tracing-subscriber`, `tokio/full` deps
- [ ] T002 Create `agentix-llama/src/main.rs`: standalone HTTP server on `AGENTIX_LLAMA_SOCKET` (default `/run/agentix/llama.sock`); `InferEngine` + `LlamaCppBackend`; pre-loads `AGENTIX_LLAMA_MODEL` if set
- [ ] T003 Implement `POST /v1/chat/completions` handler in agentix-llama binary: non-streaming path returns JSON; streaming path returns SSE via `axum::response::sse`
- [ ] T004 Implement `POST /v1/embeddings` handler in agentix-llama binary
- [ ] T005 Implement `GET /v1/models`, `POST /api/pull`, `DELETE /api/delete` handlers (model management)
- [ ] T006 Implement `POST /control/shutdown` handler (same pattern as agentix-whisper)

**Checkpoint**: `cargo build -p agentix-llama` produces both lib and binary; binary starts and serves on Unix socket.

---

## Phase 2: Daemon Socket Proxy

**Purpose**: Strip all C++ deps from agentix-daemon; replace in-process handlers with Unix socket proxies.

- [ ] T007 Remove `agentix-llama = { path = "../agentix-llama" }` from `agentix-daemon/Cargo.toml`; remove `agentix-infer` dep
- [ ] T008 Remove `cuda` feature from `agentix-daemon/Cargo.toml` (no C++ to configure)
- [ ] T009 Add `llama_socket: PathBuf` and `whisper_socket: PathBuf` to `agentix-daemon/src/config.rs`; read from `AGENTIX_LLAMA_SOCKET` / `AGENTIX_WHISPER_SOCKET`
- [ ] T010 Replace `infer_handler.rs` with a Unix socket proxy: forward requests to `llama_socket`, relay response (streaming-aware)
- [ ] T011 Replace `transcription_handler.rs` 501 stub with a Unix socket proxy: forward multipart to `whisper_socket`, relay response
- [ ] T012 Remove `agentix_infer::InferEngine` setup from `agentix-daemon/src/main.rs` (no longer needed)
- [ ] T013 Verify `cargo build -p agentix-daemon` has zero C++ compilation; binary is small (<5 MB)

**Checkpoint**: `POST /v1/chat/completions` and `POST /v1/audio/transcriptions` route through daemon to respective backend sockets and return correct responses.

---

## Phase 3: Streaming Relay

**Purpose**: Relay SSE token streams from llama without buffering; relay whisper segments similarly.

- [ ] T014 In daemon's llama proxy: detect `"stream": true` in request body; use `reqwest` streaming response; pipe SSE chunks to client via `axum::body::Body::from_stream`
- [ ] T015 Verify first token arrives at client before generation completes (no buffering)
- [ ] T016 Handle backend unavailable (socket not yet ready): return `503 Service Unavailable` with `Retry-After: 1` header

**Checkpoint**: Streaming chat completion shows tokens arriving progressively in curl/client.

---

## Phase 4: VRAM Coordination

**Purpose**: Daemon can shed a backend to free VRAM before loading a large model.

- [ ] T017 Add `vram_budget_bytes: Option<u64>` and `backend_priority: Vec<String>` to daemon config (env vars `AGENTIX_VRAM_BUDGET` and `AGENTIX_BACKEND_PRIORITY`)
- [ ] T018 Before pulling/loading a model, check if combined VRAM usage of active backends exceeds budget; if so, call `POST /control/shutdown` on lowest-priority backend
- [ ] T019 After shutdown, wait for socket to re-activate (poll with timeout) before proceeding
- [ ] T020 `GET /v1/models` aggregates model lists from both backend sockets (with fallback if a socket is down)

**Checkpoint**: Loading a model that exceeds budget automatically shuts down the lower-priority backend.

---

## Phase 5: NixOS Modules

**Purpose**: Systemd socket-activated services for each backend.

- [ ] T021 Create `flake/nixosModules/agentix-whisper.nix`: `agentix-whisper-daemon.socket` unit (ListenStream = `/run/agentix/whisper.sock`) + `agentix-whisper-daemon.service` unit (`Restart=on-failure`, `AGENTIX_WHISPER_MODEL` from option)
- [ ] T022 Create `flake/nixosModules/agentix-llama.nix`: `agentix-llama-daemon.socket` + `agentix-llama-daemon.service` (`Restart=on-failure`, `AGENTIX_LLAMA_MODEL` from option)
- [ ] T023 Add `whisperSocket` and `llamaSocket` path options to the main `agentix-daemon` NixOS module; map to env vars
- [ ] T024 Update `flake/nixosModules/agentix.nix` to wire socket paths into daemon environment
- [ ] T025 Add `agentix-whisper-daemon` and `agentix-llama-daemon` to `flake.nixosModules`
- [ ] T026 Add `packages.agentix-llama` Nix derivation in `perSystem/packages.nix` (mirrors `packages.agentix-whisper`)
- [ ] T027 Add `src/main.rs` stub for `agentix-llama` to `depsOnlySrc` and `mkBinSrc`

**Checkpoint**: `nixos-rebuild` with both modules enabled starts both socket units; `systemctl status agentix-whisper-daemon.socket` shows active.

---

## Phase 6: Polish

- [ ] T028 `cargo fmt --check` passes
- [ ] T029 `cargo clippy --workspace -- -D warnings` clean
- [ ] T030 `cargo test -p agentix-llama -p agentix-whisper -p agentix-daemon` passes
- [ ] T031 Integration test: daemon proxies transcription through whisper socket, returns correct JSON
- [ ] T032 Integration test: daemon proxies streaming chat through llama socket, SSE chunks arrive progressively

---

## Dependencies & Execution Order

- **Phase 1** (llama binary): No deps — start immediately
- **Phase 2** (daemon proxy): Depends on Phase 1
- **Phase 3** (streaming): Depends on Phase 2
- **Phase 4** (VRAM coordination): Depends on Phase 2
- **Phase 5** (NixOS): Depends on Phase 1; can parallel with Phase 3/4
- **Phase 6** (Polish): Depends on all prior phases

---

## Notes

- Use `reqwest` with Unix socket support (`reqwest::Client` + `hyper-util` Unix connector) for daemon → backend proxying
- The daemon must forward the original `Content-Type`, `Authorization`, and other headers to backends
- Backends should bind the socket with mode `0600` (daemon user only) unless multi-user access is needed
- `Restart=on-failure` means explicit `/control/shutdown` stays shut; only crashes auto-restart
- VRAM budget polling should use exponential backoff, not a tight loop
