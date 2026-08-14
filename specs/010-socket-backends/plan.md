# Spec 010: Socket-Activated Backends

**Branch**: `010-socket-backends`
**Depends on**: 006-whisper-integration (merged), 007-cargo-cleanup (merged)

## Problem

Each C++ backend (llama.cpp, whisper.cpp) holds GPU VRAM for as long as its process lives.
With both linked into the daemon there is no way to release one backend's VRAM without
restarting the entire gateway. Independent crash recovery is also impossible — a bad model
load that segfaults whisper takes down the whole inference stack.

## Solution

Each C++ backend runs as its own process listening on a Unix socket. The daemon becomes
pure Rust — it proxies requests to the appropriate backend socket and knows nothing about
llama.cpp or whisper.cpp at compile time.

```
client
  │  HTTP
  ▼
agentix-daemon  (pure Rust, no C++)
  │  Unix socket proxy
  ├──► agentix-whisper  /run/agentix/whisper.sock
  │      POST /v1/audio/transcriptions
  │      POST /control/shutdown
  │
  └──► agentix-llama    /run/agentix/llama.sock
         POST /v1/chat/completions
         POST /v1/embeddings
         POST /control/shutdown
```

## Key Design Decisions

**Unix sockets, HTTP protocol**: The daemon already uses reqwest; proxying to a Unix socket
is a small change (`reqwest` + `hyper` both support Unix sockets). HTTP keeps the protocol
consistent across the stack and makes backends independently testable with `curl --unix-socket`.

**Shutdown via socket**: `POST /control/shutdown` on each backend's socket triggers graceful
drain-and-exit. The daemon calls this before loading a model that needs the VRAM budget.
No systemctl privileges required — just an HTTP call to a local socket.

**Socket activation (systemd)**: Each backend has a `.socket` unit so systemd restarts it
on the next request after a shutdown. `Restart=on-failure` (not `always`) so an explicit
`/control/shutdown` stays shut until the socket activation wakes it.

**Streaming**: Backends return SSE streams for token-by-token generation (llama) and
segment-by-segment transcription (whisper). The daemon relays the stream without buffering
via `reqwest`'s streaming response API.

**VRAM budget**: The daemon tracks a configured VRAM budget. Before loading a large model
it may call `/control/shutdown` on the other backend to reclaim its VRAM, then waits for
the socket to re-activate when needed again.

## Project Structure Changes

```text
agentix-daemon/      # Pure Rust; proxies to sockets; no C++ deps
agentix-llama/       # Gains src/main.rs: HTTP server on llama.sock
agentix-whisper/     # Already has src/main.rs from spec 006
```

The `agentix-infer` engine (InferEngine, ModelStore) moves into each backend binary.
The daemon no longer links `agentix-infer`, `agentix-llama`, or `agentix-whisper`.

## Wire Protocol

Both backends speak a subset of the OpenAI HTTP API over their Unix socket:

**agentix-llama** (`/run/agentix/llama.sock`):
- `POST /v1/chat/completions` — streaming (SSE) and non-streaming
- `POST /v1/embeddings`
- `GET  /v1/models`
- `POST /api/pull` — register a model in the local store
- `DELETE /api/delete`
- `POST /control/shutdown`

**agentix-whisper** (`/run/agentix/whisper.sock`):
- `POST /v1/audio/transcriptions` — already implemented
- `POST /control/shutdown` — already implemented

## NixOS Module Changes

- `agentix-daemon` NixOS module: add `whisperSocket` and `llamaSocket` path options
- New `agentix-whisper-daemon` NixOS module: socket unit + service unit with `Restart=on-failure`
- New `agentix-llama-daemon` NixOS module: socket unit + service unit with `Restart=on-failure`
- `whisperAlwaysOn` option moves to `agentix-whisper-daemon` module as `model` option

## VRAM Coordination

The daemon's config gains `vramBudgetBytes`. Before loading a model:
1. Check if the model fits within budget alongside currently-loaded backends
2. If not, call `/control/shutdown` on the lowest-priority backend
3. Wait for the socket to become ready again (systemd re-activates on first connection)

Priority: llama > whisper (configurable via NixOS option).

## Streaming Architecture

For `chat/completions` with `stream: true`:
- Daemon opens HTTP connection to llama socket, reads SSE chunks
- Relays each chunk to the client as it arrives (no buffering)
- Uses `reqwest` streaming + axum `Body::from_stream`

## Out of Scope

- TTS backend (spec 011-tts)
- WebSocket audio streaming for real-time voice (spec 012-voice-pipeline)
- Multi-GPU distribution
