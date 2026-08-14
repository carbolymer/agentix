# Implementation Plan: Cargo Crate Decomposition

**Branch**: `007-cargo-cleanup` | **Date**: 2026-08-13 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/007-cargo-cleanup/spec.md`

## Summary

Decompose the monolithic `agentix-infer` crate and the root `mcp-server` crate so that
changes to search, indexing, MCP server, or routing logic never trigger C++ recompilation.
The approach is purely mechanical: extract `LlamaCppBackend` into a new `agentix-llama`
crate, extract search/indexer/MCP-server code from the root crate into three new library
crates, update the workspace manifest, update caller imports, and update Nix packaging.
No new capabilities are added.

## Technical Context

**Language/Version**: Rust edition 2021, Rust 1.80+ (via fenix stable toolchain in Nix)
**Primary Dependencies**: llama-cpp-2 (C++, moves to `agentix-llama`), tree-sitter-* (C++, in `agentix-indexer`), fastembed/onnxruntime (C++, in `agentix-search`), tokio, axum, sqlx, rmcp
**Storage**: PostgreSQL 17 + pg_search (BM25) + pgvector (HNSW); unchanged by this refactor
**Testing**: `cargo test --workspace`; existing integration tests must all pass unchanged
**Target Platform**: Linux server (x86_64-linux); Nix flake is the build/package system
**Project Type**: Workspace library/binary decomposition — pure refactor, no new features
**Performance Goals**: Warm-cache rebuild for pure-Rust crate changes completes in < 30s (vs. several minutes with C++ recompilation)
**Constraints**: Public API surface of `agentix-infer` types and traits must not change; Nix package attributes must not be removed (only renamed or added); all existing binaries must continue to build and function
**Scale/Scope**: 6 workspace members → 10 workspace members after decomposition

## Constitution Check

| Principle | Status | Notes |
|-----------|--------|-------|
| I — Library-First Architecture | ✓ PASS | All 4 new crates are pure library crates with clear non-overlapping purposes. `agentix-mcp-server` and `agentix-indexer` each own one binary that aligns with their purpose. |
| II — Local-First Intelligence | ✓ N/A | No routing or inference changes. |
| III — Reproducible Environments | ✓ PASS | Every new crate gets a `nix build .#<pkg>` attribute. `Cargo.lock` must be committed after adding new workspace members. |
| IV — Isolation by Default | ✓ N/A | No sandbox changes. |
| V — Layered API and Routing | ✓ PASS | `agentix-infer` core retains traits and types. Daemon remains the only crate that assembles backends. MCP server uses spec-compliant rmcp transport — unchanged. |
| VI — Comprehensive Testing | ✓ PASS | `store_integration.rs` stays in `agentix-infer/tests/` (no C++ dep). `complete_integration.rs` and `embed_integration.rs` move to `agentix-llama/tests/` (require LlamaCpp — they belong in the crate with the C++ dep). All must pass. New crates need at least compilation tests. |
| VII — Formal Agent State Machine | ✓ N/A | `agentix-harness` not touched. |
| VIII — Code Quality Gates | ✓ PASS | All gates (fmt, clippy, test, nix build) must be green. Every new crate needs a `[lints] workspace = true` entry. |

**No constitution violations.** This refactor improves compliance with Principle I by
giving each concern its own crate with a clear, bounded purpose.

## Project Structure

### Documentation (this feature)

```text
specs/007-cargo-cleanup/
├── spec.md              # Feature specification
├── plan.md              # This file
├── research.md          # Phase 0 — completed
├── data-model.md        # Phase 1 — completed
├── quickstart.md        # Phase 1 — completed
└── tasks.md             # Phase 2 output (from /speckit.tasks — generated)
```

### Source Code (post-decomposition workspace layout)

```text
agentix/                        (workspace root)
├── Cargo.toml                  workspace manifest — adds 4 new members
├── src/                        agentix-jails (renamed from mcp-server package)
│   ├── jail/main.rs            claude-jail
│   ├── ax_jail/main.rs         ax-jail
│   └── gh_proxy/               gh proxy
│
├── agentix-api/                (unchanged)
├── agentix-router/             (unchanged)
│
├── agentix-infer/              REDUCED — core traits, types, engine, pool, store, meta
│   ├── src/backend/mod.rs      InferBackend + LoadedModel traits (keep)
│   ├── src/engine.rs           InferEngine (keep)
│   ├── src/pool.rs             ContextPool (keep)
│   ├── src/store/              ModelStore (keep)
│   ├── src/meta/               GGUF + safetensors metadata (keep)
│   └── src/error.rs            InferError (keep)
│   [REMOVED: src/backend/llamacpp.rs, build.rs, llama-cpp-2 dep]
│
├── agentix-llama/              NEW — LlamaCppBackend implementation
│   ├── src/lib.rs              LlamaCppBackend, LlamaCppLoadedModel
│   ├── build.rs                CUDA link flags (moved from agentix-infer)
│   └── Cargo.toml              deps: agentix-infer, llama-cpp-2; features: cuda
│
├── agentix-indexer/            NEW — document ingestion pipeline + ingest binary
│   ├── src/ingest/             (moved from root src/ingest/)
│   ├── src/main.rs             ingest binary entry point
│   ├── src/lib.rs              pub mod ingest { ... }
│   └── Cargo.toml              deps: sqlx, tree-sitter-*, clap, sha2, ...
│
├── agentix-search/             NEW — search queries, embedding, reranking
│   ├── src/db.rs               hybrid BM25+vector search (moved from root)
│   ├── src/embed.rs            query embedding via Ollama (moved from root)
│   ├── src/rerank.rs           fastembed reranking (moved from root)
│   ├── src/fmt.rs              result formatting (moved from root)
│   ├── src/lib.rs              pub mod db, embed, rerank, fmt
│   └── Cargo.toml              deps: sqlx, reqwest, fastembed, anyhow, tracing
│
├── agentix-mcp-server/         NEW — MCP tools + mcp-server binary
│   ├── src/main.rs             MCP server entry point (moved from root)
│   ├── src/tools.rs            MCP tool definitions (moved from root)
│   └── Cargo.toml              deps: agentix-search, agentix-indexer, rmcp, sqlx, ...
│
├── agentix-daemon/             UPDATED — adds agentix-llama dep; changes LlamaCppBackend import
├── agentix-harness/            (unchanged)
└── agentix-ax/                 (unchanged)
```

## Complexity Tracking

No constitution violations requiring justification.

## Implementation Phases

### Phase A — Extract `agentix-llama` from `agentix-infer`

**Goal**: `agentix-infer` compiles with zero C++ dependencies.

1. Create `agentix-llama/` directory with `Cargo.toml`, `src/lib.rs`, `build.rs`
2. Move `agentix-infer/src/backend/llamacpp.rs` → `agentix-llama/src/lib.rs`; update
   `crate::` imports to `agentix_infer::` (all types/traits already pub)
3. Move `agentix-infer/build.rs` → `agentix-llama/build.rs`
4. Update `agentix-infer/Cargo.toml`: remove `llama-cpp-2` dep, remove `llamacpp`
   feature, remove `build.rs` reference, remove dead candle deps
5. Update `agentix-infer/src/backend/mod.rs`: remove `#[cfg(feature = "llamacpp")] pub mod llamacpp`
6. Update `agentix-daemon/Cargo.toml`: add `agentix-llama` dep, update `cuda` feature,
   remove `features = ["llamacpp"]` from `agentix-infer` dep
7. Update `agentix-daemon/src/main.rs`: change import path
8. Add `agentix-llama` to workspace `Cargo.toml` members
9. Verify: `cargo build -p agentix-infer` produces no C++ compilation

### Phase B — Extract `agentix-search` from root crate

**Goal**: Search logic is in its own crate, cleanly importable.

1. Create `agentix-search/` with `Cargo.toml`, `src/lib.rs`
2. Move `src/db.rs`, `src/embed.rs`, `src/rerank.rs`, `src/fmt.rs` to `agentix-search/src/`
3. Write `agentix-search/src/lib.rs` exporting all four modules
4. Update `agentix-search/Cargo.toml` with all deps currently in root for those files
5. Add `agentix-search` to workspace `Cargo.toml` members
6. Verify: `cargo build -p agentix-search`

### Phase C — Extract `agentix-indexer` from root crate

**Goal**: Ingest pipeline and `ingest` binary move to their own crate.

1. Create `agentix-indexer/` with `Cargo.toml`, `src/lib.rs`
2. Move `src/ingest/` to `agentix-indexer/src/ingest/`
3. Move `src/ingest/main.rs` → `agentix-indexer/src/main.rs` (binary entry point)
4. Write `agentix-indexer/src/lib.rs` with the same `pub mod ingest { ... }` structure
5. Update `agentix-indexer/Cargo.toml` with tree-sitter + ingest deps
6. Add `agentix-indexer` to workspace `Cargo.toml` members
7. Verify: `cargo build -p agentix-indexer`

### Phase D — Extract `agentix-mcp-server` from root crate

**Goal**: MCP server and `mcp-server` binary move to their own crate.

1. Create `agentix-mcp-server/` with `Cargo.toml`
2. Move `src/main.rs` → `agentix-mcp-server/src/main.rs`
3. Move `src/tools.rs` → `agentix-mcp-server/src/tools.rs`
4. Update imports in `tools.rs`: `mcp_server::ingest::` → `agentix_indexer::ingest::`
5. Add `agentix-mcp-server/Cargo.toml` with deps: `agentix-search`, `agentix-indexer`,
   `rmcp`, `sqlx`, `tokio`, `serde`, `serde_json`, `reqwest`, `tracing`, etc.
6. Add `agentix-mcp-server` to workspace `Cargo.toml` members
7. Verify: `cargo build -p agentix-mcp-server`

### Phase E — Thin the root crate

**Goal**: Root crate contains only jail binaries.

1. Remove `src/lib.rs` (no longer exports anything)
2. Remove moved files from root `src/`
3. Update root `Cargo.toml`: rename package to `agentix-jails`, remove all moved deps
   (rmcp, sqlx, fastembed, tree-sitter-*, reqwest, etc.)
4. Verify: `cargo build -p agentix-jails`
5. Verify: full `cargo build --workspace` succeeds

### Phase F — Update Nix packaging

**Goal**: Every new crate has a `nix build .#<pkg>` attribute; existing attributes
continue to work.

1. Update `perSystem/packages.nix`:
   - Add all 4 new crate manifests to `depsOnlySrc` file-set and stub generation
   - Add all 4 new crate manifests to `mkBinSrc` stub generation
   - Add all 4 new crate source directories to `agentixSrc`
   - Add `agentix-llama` stub (`src/lib.rs`) in `depsOnlySrc` and `mkBinSrc`
   - Add `agentix-indexer` stub (`src/lib.rs`, `src/main.rs`) equivalents
   - Add `agentix-search` stub (`src/lib.rs`)
   - Add `agentix-mcp-server` stub (`src/main.rs`)
   - Remove old root-crate binary stubs that moved (`src/main.rs`, `src/ingest/main.rs`)
   - Add new Nix package derivations: `agentix-llama`, `agentix-search`, `agentix-indexer` (replaces `ingest`), `agentix-mcp-server` (replaces `mcp-server`)
   - Update `claudeJailBinDir` and `axJailBinDir` to reference renamed packages
2. Verify: `nix build .#agentix-daemon` and `nix build .#agentix-mcp-server`

### Phase G — Update documentation

1. Update `ARCHITECTURE.md` crate inventory and dependency graph
2. Update `CLAUDE.md` active technologies to reflect new crate names
3. Run `cargo fmt --check` and `cargo clippy -- -D warnings` across workspace

## Key Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Import path renames break compilation | Phases are sequential — each phase ends with a build check before proceeding |
| Nix stub generation misses a new entry point | Every new `[[bin]]` and `[lib]` must have a stub line in `depsOnlySrc` and `mkBinSrc` |
| Root crate rename breaks NixOS module service | Check `flake/nixosModules/agentix.nix` for references to `mcp-server` package |
| `Cargo.lock` divergence | Commit `Cargo.lock` after each phase's workspace manifest change |
| candle dead deps in `agentix-infer` | Remove them in Phase A to keep the crate clean; they have no implementing code |
