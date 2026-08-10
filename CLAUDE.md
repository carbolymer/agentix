# agentix Development Guidelines

Auto-generated from feature plans and updated manually. Last updated: 2026-08-10

## Active Technologies

- Rust (edition 2021, Rust 1.80+)
- Tokio async runtime
- Axum HTTP framework (agentix-daemon)
- llama-cpp-2 (GGUF inference, agentix-infer Phase 1)
- candle (safetensors inference, agentix-infer Phase 2)
- PostgreSQL 17 + pg_search (BM25) + pgvector (HNSW)
- Nix flakes (build + services)

## Project Structure

```text
agentix-api/         # OpenAI-compatible request/response types (no deps)
agentix-router/      # Backend-selection routing (RouteTarget enum)
agentix-infer/       # In-process inference library (GGUF + safetensors)
agentix-daemon/      # HTTP gateway (Axum); assembles api + router + infer
agentix-harness/     # Agent loop library (state machine, tool dispatch)
agentix-ax/          # TUI agent binary (Ratatui, links harness)
src/                 # mcp-server + ingest binaries (root crate)
```

Dependency flow: `agentix-api` → `agentix-router`, `agentix-infer` → `agentix-daemon`. No circular deps. The daemon is the only crate that binds a port.

## Commands

```bash
# Build everything
nix develop --command cargo build --workspace

# Test a specific crate
nix develop --command cargo test -p agentix-infer

# Run all tests
nix develop --command cargo test --workspace

# Format check
nix develop --command cargo fmt --check

# Clippy (fails on warnings; unwrap_used + expect_used are enabled)
nix develop --command cargo clippy -- -D warnings

# Start dev services (PostgreSQL + Ollama)
nix run .#dev

# Run gateway
nix develop --command agentix-daemon
```

## Code Style

- Conventional Commits: `feat:`, `fix:`, `feat!:`, `chore:`, `docs:`, `test:`
- `clippy::unwrap_used` and `clippy::expect_used` are denied workspace-wide — use `?` or explicit error handling
- `unsafe` blocks require a `// SAFETY:` comment explaining the invariant
- No blocking calls on the Tokio runtime — use `tokio::task::spawn_blocking` for C FFI (llama.cpp)
- Comments only when the WHY is non-obvious; no docstring blocks for obvious functions

## Active Feature: 001-agentix-infer

**Branch**: `001-agentix-infer`
**Plan**: `specs/001-agentix-infer/plan.md`

Building `agentix-infer`: in-process GGUF inference replacing the Ollama HTTP proxy.

Key constraints:
- Pure library crate — no network surface
- Integration tests MUST use a small (<50MB) fixture GGUF pinned in Nix, not jina-code (1.5GB)
- All llama.cpp C FFI via `spawn_blocking`
- Ollama-compatible blob layout so existing model dirs are usable

<!-- MANUAL ADDITIONS START -->
<!-- MANUAL ADDITIONS END -->
