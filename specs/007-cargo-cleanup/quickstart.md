# Quickstart: Post-Decomposition Build Guide

**Feature**: 007-cargo-cleanup | **Date**: 2026-08-13

## Building after decomposition

No command changes — the workspace commands all continue to work as before:

```bash
# Build the full workspace (all crates)
nix develop --command cargo build --workspace

# Build only the daemon (brings in agentix-llama transitively)
nix develop --command cargo build -p agentix-daemon

# Build the MCP server (no C++ recompilation)
nix develop --command cargo build -p agentix-mcp-server

# Build the ingest binary
nix develop --command cargo build -p agentix-indexer

# Test a specific crate (isolation demo)
nix develop --command cargo test -p agentix-search
nix develop --command cargo test -p agentix-mcp-server
nix develop --command cargo test -p agentix-infer
nix develop --command cargo test -p agentix-llama

# Full test suite
nix develop --command cargo test --workspace
```

## Verifying build isolation

After a warm build, touch only search logic and confirm no C++ recompilation:

```bash
# Touch a search source file
touch agentix-search/src/db.rs

# Rebuild — should show only agentix-search and agentix-mcp-server compiling
# (no llama.cpp, no tree-sitter)
nix develop --command cargo build --workspace 2>&1 | grep "Compiling"
```

Expected output includes only pure-Rust crates; no `llama-cpp-2`, `tree-sitter-*`,
or `fastembed` build output.

## Nix package builds

```bash
nix build .#agentix-daemon        # HTTP gateway (includes llama.cpp)
nix build .#agentix-mcp-server    # MCP server binary
nix build .#agentix-indexer       # ingest binary
nix build .#agentix-search        # search library (no binary)
nix build .#agentix-llama         # llamacpp backend library (no binary)
nix build .#ax                    # TUI agent
nix build .#claude-jail           # bubblewrap jail for claude-code
nix build .#ax-jail               # bubblewrap jail for agentix-ax
```

## Changed import paths

If you have code outside the workspace that imports from these crates:

| Before | After |
|--------|-------|
| `agentix_infer::backend::llamacpp::LlamaCppBackend` | `agentix_llama::LlamaCppBackend` |
| `mcp_server::ingest::crates::ingest_crate` | `agentix_indexer::ingest::crates::ingest_crate` |
| `mcp_server::ingest::embed::ensure_embed_model` | `agentix_indexer::ingest::embed::ensure_embed_model` |
| (search/db/rerank/fmt were private to root crate) | `agentix_search::db`, `agentix_search::rerank`, etc. |

## Feature flags

| Feature | Crate | Effect |
|---------|-------|--------|
| `cuda` | `agentix-llama` | Enables GPU inference via llama.cpp CUDA |
| `cuda` | `agentix-daemon` | Passes `agentix-llama/cuda` through |

The `llamacpp` feature flag on `agentix-infer` is removed. The core crate is
always compiled without C++ regardless of build configuration.
