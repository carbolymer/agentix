# Research: Cargo Crate Decomposition

**Feature**: 007-cargo-cleanup | **Date**: 2026-08-13

## Decisions

### 1. Extraction boundary for `agentix-infer` → `agentix-llama`

**Decision**: The only code that crosses the C++ boundary in `agentix-infer` is
`src/backend/llamacpp.rs` and `build.rs`. Everything else — types, traits
(`InferBackend`, `LoadedModel`), engine, pool, store, meta — has zero direct C++
dependency and remains in `agentix-infer`.

**Rationale**: `llamacpp.rs` imports `llama_cpp_2::*` directly. The engine uses trait
objects (`Arc<dyn InferBackend>`) so it never touches the concrete type. The pool,
store, and meta modules are pure Rust. Moving only `llamacpp.rs` + `build.rs` to a
new `agentix-llama` crate is the minimal surgical change with the maximum build-cache
benefit.

**Alternatives considered**: Moving the full `backend/` module (traits included) to
`agentix-llama`. Rejected: traits are the shared contract — placing them in
`agentix-llama` would force every future backend crate to depend on the llama crate,
recreating the tight coupling we're eliminating.

---

### 2. `agentix-infer` candle feature handling

**Decision**: The `candle-*` optional dependencies in `agentix-infer/Cargo.toml` are
retained in `agentix-infer` for now. No `agentix-candle` crate is created in this
feature. The candle feature flag remains compilable but the backend implementation is
not yet extracted (there is no `backend/candle.rs` — the feature deps are declared but
unused at the code level).

**Rationale**: Extracting a second backend crate with no implementation is premature.
The candle feature is out-of-scope per the spec.

---

### 3. Root crate boundary split

**Decision**:
- `agentix-search` ← `src/db.rs`, `src/embed.rs`, `src/rerank.rs`, `src/fmt.rs`
- `agentix-indexer` ← `src/ingest/` (all modules) + the `ingest` binary entry point
- `agentix-mcp-server` ← `src/main.rs`, `src/tools.rs` + the `mcp-server` binary entry point
- Root crate ← jail binaries only (`src/jail/`, `src/ax_jail/`, `src/gh_proxy/`)

**Rationale**: `src/lib.rs` today re-exports `src/ingest/` for shared use by the MCP
server. After extraction, `agentix-mcp-server` will depend directly on
`agentix-indexer` and `agentix-search`. The current `mcp_server::ingest::*` imports
in `tools.rs` map to `agentix_indexer::*` after the move. The root crate `mcp-server`
package name will change to reflect its reduced scope (jail binaries only); the Cargo
package name can become `agentix-jails`.

**Alternatives considered**: Keeping binaries in root and having them call into new
library crates. Rejected: the crane Nix build already produces per-binary derivations
using `mkBinSrc` stubs — moving binaries to their owning crates simplifies the Nix
packaging and makes each crate self-contained.

---

### 4. `agentix-mcp-server` depends on `agentix-indexer`

**Decision**: `agentix-mcp-server` directly depends on `agentix-indexer`.

**Rationale**: `src/tools.rs` calls `ingest_crate`, `ingest_hackage`, `ingest_pypi`,
and `ensure_embed_model` from `src/ingest/`. These are MCP tool implementations that
trigger on-demand indexing from the MCP server. The dependency is correct by design:
the MCP server is the consumer, the indexer is the producer.

---

### 5. C++ dependencies by crate (post-decomposition)

| Crate | C++ dep | Library |
|-------|---------|---------|
| `agentix-llama` | yes | llama-cpp-2 / llama.cpp |
| `agentix-indexer` | yes | tree-sitter-* (multiple) |
| `agentix-search` | yes (optional) | fastembed / onnxruntime |
| `agentix-infer` | **no** | — |
| `agentix-mcp-server` | **no** | — |
| `agentix-router` | **no** | — |
| `agentix-api` | **no** | — |
| `agentix-harness` | **no** | — |
| `agentix-ax` | **no** | — |
| root (`agentix-jails`) | **no** | — |

Build-cache isolation: a change to MCP tool logic recompiles only `agentix-mcp-server`
— no C++ recompilation. A change to search query logic recompiles only `agentix-search`
— no llama.cpp or tree-sitter recompilation. A change to the llamacpp backend
recompiles only `agentix-llama` — `agentix-infer` core, indexer, search, and MCP server
are all unchanged.

---

### 6. `agentix-daemon` import path change

**Decision**: After extraction, `agentix-daemon` adds `agentix-llama` as a direct
dependency and changes:
```rust
// before
use agentix_infer::backend::llamacpp::LlamaCppBackend;
// after
use agentix_llama::LlamaCppBackend;
```
The `llamacpp` feature flag on the `agentix-infer` dep is removed. The `cuda` feature
flag moves to `agentix-llama/cuda`.

**Rationale**: The daemon is the only binary that wires backends at link time. This is
the minimal caller change.

---

### 7. Nix packages.nix update strategy

**Decision**: Every new crate must be added to:
1. `depsOnlySrc` file-set (manifest path) + stub generation
2. `mkBinSrc` stub generation (workspace resolution requires all member manifests)
3. `agentixSrc` file-set (full source path)

New Nix package attributes required (per constitution gate VIII.5):
- `packages.agentix-llama` — builds the llamacpp backend crate
- `packages.agentix-indexer` (replaces `packages.ingest` + provides library)
- `packages.agentix-search`
- `packages.agentix-mcp-server` (replaces `packages.mcp-server`)

The `cudaArgs` scope applies only to `agentix-llama` and `agentix-daemon` builds; the
other new crates use plain `commonArgs`.

`packages.mcp-server` and `packages.ingest` are renamed/replaced. Any downstream
references (jail bin directories, NixOS module) must be updated to the new package names.

---

### 8. `--allow-multiple-definition` linker flag

**Decision**: Not applicable in this feature. The linker workaround for shared ggml
symbols between two C++ backends is only needed when both `agentix-llama` and a future
Whisper backend crate are linked into the same binary simultaneously. With only one
backend crate, no symbol conflict exists.

**How to apply when the time comes**: The `RUSTFLAGS` or `[target.*.rustflags]` in
`.cargo/config.toml` at the daemon level is the right place, not in the library crates.
