# Data Model: Cargo Crate Decomposition

**Feature**: 007-cargo-cleanup | **Date**: 2026-08-13

This is a pure refactor — no new runtime data models. The "entities" are Cargo crate
boundaries and their dependency edges.

---

## Workspace Crate Inventory (post-decomposition)

```
agentix/                        (workspace root)
├── src/                        jail binaries only (package: agentix-jails)
│   ├── jail/main.rs            claude-jail binary
│   ├── ax_jail/main.rs         ax-jail binary
│   └── gh_proxy/               gh proxy client/server binaries
│
├── agentix-api/                OpenAI-compatible request/response types
│                               C++: no | deps: (none)
│
├── agentix-router/             backend-selection routing (RouteTarget enum)
│                               C++: no | deps: agentix-api
│
├── agentix-infer/              inference CORE: traits, types, engine, pool, store, meta
│                               C++: no | deps: (pure Rust — async-trait, tokio, serde,
│                               sha2, hex, hf-hub, ureq, dirs, futures, tokio-stream)
│
├── agentix-llama/              NEW — LlamaCpp backend implementation
│                               C++: YES (llama-cpp-2 / llama.cpp)
│                               deps: agentix-infer, llama-cpp-2, tokio, async-trait,
│                                     thiserror, tracing
│                               features: default=[], cuda=["llama-cpp-2/cuda"]
│
├── agentix-indexer/            NEW — document ingestion pipeline
│                               C++: YES (tree-sitter-*, via multiple grammar crates)
│                               deps: sqlx, tokio, reqwest, serde, sha2, hex,
│                                     indicatif, clap, chrono, ignore, glob,
│                                     tree-sitter + grammar crates, tempfile,
│                                     flate2, tar, zip, tracing, anyhow
│                               binary: ingest
│
├── agentix-search/             NEW — BM25+vector search, DB queries, reranking
│                               C++: YES (fastembed/onnxruntime, optional via RERANK_MODEL)
│                               deps: sqlx, reqwest, fastembed, anyhow, tracing
│
├── agentix-mcp-server/         NEW — MCP tool definitions and protocol handlers
│                               C++: no
│                               deps: agentix-search, agentix-indexer, rmcp,
│                                     sqlx, tokio, serde, serde_json, reqwest,
│                                     tracing, tracing-subscriber, anyhow
│                               binary: mcp-server
│
├── agentix-daemon/             Axum HTTP gateway
│                               C++: YES (transitively via agentix-llama)
│                               deps: agentix-api, agentix-router, agentix-infer,
│                                     agentix-llama, axum, tokio, reqwest, serde,
│                                     serde_json, anyhow, tracing, tracing-subscriber,
│                                     tower-http, tokio-stream
│                               features: cuda=["agentix-llama/cuda"]
│
├── agentix-harness/            agent loop library
│                               C++: no | deps: (existing, unchanged)
│
└── agentix-ax/                 TUI agent binary
                                C++: no | deps: agentix-api, agentix-harness
```

---

## Dependency Graph (post-decomposition)

```
agentix-api
    └── agentix-router

agentix-infer
    └── agentix-llama
            └── agentix-daemon ─── agentix-api
                              └─── agentix-router
                              └─── agentix-infer

agentix-indexer ─── agentix-mcp-server
agentix-search  ───┘

agentix-harness ─── agentix-ax ─── agentix-api

root crate (agentix-jails): no library deps
```

**Invariant**: Zero edges from `agentix-search`, `agentix-indexer`, `agentix-mcp-server`,
or `agentix-router` to `agentix-llama` or any other C++ backend crate. The graph is acyclic.

---

## File Migration Map

### `agentix-infer` → `agentix-infer` (core, reduced)

| File | Action |
|------|--------|
| `src/backend/mod.rs` | Keep (contains traits only) |
| `src/backend/llamacpp.rs` | **Move** to `agentix-llama/src/lib.rs` |
| `src/engine.rs` | Keep |
| `src/error.rs` | Keep |
| `src/meta/` | Keep |
| `src/pool.rs` | Keep |
| `src/store/` | Keep |
| `build.rs` | **Move** to `agentix-llama/build.rs` |
| `Cargo.toml` | Remove `llama-cpp-2` dep; remove `llamacpp` feature; remove `build.rs` reference |

After extraction `agentix-infer` retains zero C++ dependencies and zero optional
feature flags (candle deps can be removed from Cargo.toml too since there is no
implementing code — they are dead declarations; defer to a separate cleanup commit).

### Root crate (`mcp-server`) → split to new crates + residual

| File | Destination |
|------|-------------|
| `src/db.rs` | `agentix-search/src/db.rs` |
| `src/embed.rs` | `agentix-search/src/embed.rs` |
| `src/rerank.rs` | `agentix-search/src/rerank.rs` |
| `src/fmt.rs` | `agentix-search/src/fmt.rs` |
| `src/ingest/` (all) | `agentix-indexer/src/ingest/` |
| `src/ingest/main.rs` | `agentix-indexer/src/main.rs` (binary entry) |
| `src/main.rs` | `agentix-mcp-server/src/main.rs` |
| `src/tools.rs` | `agentix-mcp-server/src/tools.rs` |
| `src/lib.rs` | **Remove** (re-export shim no longer needed) |
| `src/jail/main.rs` | Keep in root |
| `src/ax_jail/main.rs` | Keep in root |
| `src/gh_proxy/` | Keep in root |

Root `Cargo.toml` package name changes from `mcp-server` to `agentix-jails`. All
tree-sitter, fastembed, rmcp, and sqlx deps are removed from root; they move to their
owning crates.

### `agentix-daemon` (minimal changes)

| Change | Detail |
|--------|--------|
| Add dep | `agentix-llama = { path = "../agentix-llama" }` |
| Remove feature | `agentix-infer = { path = "../agentix-infer" }` (no more `features = ["llamacpp"]`) |
| Update feature | `cuda = ["agentix-llama/cuda"]` |
| Update import | `agentix_infer::backend::llamacpp::LlamaCppBackend` → `agentix_llama::LlamaCppBackend` |

---

## `agentix-llama` Public API

```rust
// agentix-llama/src/lib.rs
pub use backend::LlamaCppBackend;
pub use backend::LlamaCppLoadedModel;
```

`LlamaCppBackend` implements `agentix_infer::backend::InferBackend`.
`LlamaCppLoadedModel` implements `agentix_infer::backend::LoadedModel`.

Consumers (i.e., `agentix-daemon`) only use `LlamaCppBackend::new()` and pass the
result to `InferEngine::register_backend()`. No other symbols need to be public.

---

## `agentix-search` Public API

```rust
// agentix-search/src/lib.rs
pub mod db;     // ChunkRow, RepoSummary, search functions
pub mod embed;  // embed_query()
pub mod rerank; // rerank()
pub mod fmt;    // format_results()
```

The module structure mirrors the current root crate layout. All callers
(`agentix-mcp-server`) use unqualified paths after the crate rename.

---

## `agentix-indexer` Public API

```rust
// agentix-indexer/src/lib.rs
pub mod ingest {
    pub mod code;
    pub mod crates;
    pub mod docs;
    pub mod embed;
    pub mod hackage;
    pub mod pypi;
    pub mod repo_index;
    pub mod symbols;
}
```

Matches current `mcp_server::ingest::*` layout. The `agentix-mcp-server` import
path changes from `mcp_server::ingest::` to `agentix_indexer::ingest::`.
