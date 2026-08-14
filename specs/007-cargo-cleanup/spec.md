# Feature Specification: Cargo Crate Decomposition

**Feature Branch**: `007-cargo-cleanup`
**Created**: 2026-08-13
**Status**: Draft
**Input**: User description: "Break agentix-infer into agentix-llama and agentix-whisper crates, extract agentix-indexer, agentix-search, and agentix-mcp-server from root crate so that router, indexer, search, and mcp-server changes do not trigger C++ crate rebuilds"

**Scope note**: The `agentix-whisper` crate (audio transcription) is not yet in the repo — that feature is on an unmerged branch. This decomposition creates `agentix-llama` and designs the crate boundary to support additional backends (including Whisper when it lands) without requiring further architectural change.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Fast Search/Indexer Iteration (Priority: P1)

A developer modifying search ranking, indexing pipelines, or BM25/vector logic can rebuild and test without triggering any C++ compilation. Their change compiles in seconds, not minutes.

**Why this priority**: Search and indexing are frequently modified. Any friction here multiplies across the team's daily workflow.

**Independent Test**: Modify a source file in the search or indexer component, run a build, and observe that no C++ compilation occurs and the build completes in under 30 seconds on a warm cache.

**Acceptance Scenarios**:

1. **Given** a clean incremental build cache for all C++ dependencies, **When** a developer modifies only search logic and rebuilds, **Then** no C++ compiler invocations occur and inference backend crates are not recompiled.
2. **Given** a developer adds a new indexing pipeline feature, **When** they rebuild, **Then** only the indexer and its downstream consumers recompile — inference backend crates are unchanged.

---

### User Story 2 - Fast MCP Server Iteration (Priority: P2)

A developer adding or modifying MCP tool definitions, protocol handlers, or agent tool dispatch can iterate without waiting for C++ compilation.

**Why this priority**: The MCP server surface changes as new agent capabilities are added. It should have no build-time dependency on inference backends.

**Independent Test**: Modify the MCP server handler code, rebuild, and confirm no C++ compilation occurs.

**Acceptance Scenarios**:

1. **Given** a warm build cache, **When** a developer modifies an MCP tool definition and rebuilds, **Then** only the MCP server crate and its downstream consumers recompile.
2. **Given** a new tool is added to the MCP server, **When** the project is built, **Then** inference backends are not recompiled.

---

### User Story 3 - Independent Backend Crate Architecture (Priority: P2)

The crate structure is designed so each inference backend (LLM, and future backends such as Whisper audio transcription) lives in its own crate. Adding a second C++ backend crate in the future does not require architectural rework, and when multiple backends coexist, a change to one does not trigger recompilation of another.

**Why this priority**: Each backend embeds a large C++ library. The decomposition must produce a crate boundary that scales naturally to multiple backends — not one that requires revisiting when the Whisper feature eventually merges.

**Independent Test**: Verify that `agentix-llama` has no compile-time dependency on any future Whisper crate, and that the shared inference core (`agentix-infer`) carries no C++ dependency of its own. When a second backend crate is later added following the same pattern, no changes to existing crates should be required.

**Acceptance Scenarios**:

1. **Given** `agentix-llama` has been compiled, **When** only LLM backend source changes, **Then** only `agentix-llama` and its direct dependents recompile — the inference core and non-inference crates are unchanged.
2. **Given** the decomposed workspace, **When** a new backend crate is added following the established pattern, **Then** no existing crate requires modification to accommodate it.

---

### User Story 4 - Router Changes Do Not Trigger C++ (Priority: P3)

A developer modifying backend-selection routing logic can iterate without any C++ compilation.

**Why this priority**: Routing logic evolves as new backends and model formats are added. It must remain cheap to change.

**Independent Test**: Modify routing logic, rebuild, and observe that neither inference backend is recompiled.

**Acceptance Scenarios**:

1. **Given** all crates have been compiled, **When** only routing logic is modified and the project is rebuilt, **Then** inference backend crates are not recompiled.

---

### Edge Cases

- What happens when a shared trait or type used by both inference backends changes? The change propagates to both backends, which is expected and correct.
- How does the system handle a build where only one inference backend feature is enabled? Crates that do not require C++ must compile correctly regardless of which inference features are active.
- What happens when only the Nix packaging configuration is changed without any Rust source changes? No Rust crates should recompile.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The build system MUST NOT recompile C++ inference backend crates when only search, indexing, MCP server, or routing source files are modified.
- **FR-002**: Each inference backend MUST reside in its own crate with no direct dependency on other backend crates. The LLM backend (`agentix-llama`) is the first such crate; the architecture MUST accommodate additional backends (e.g. a future Whisper crate) without modification to existing crates.
- **FR-003**: The `agentix-search` crate MUST NOT have a transitive compile-time dependency on any inference backend crate (e.g. `agentix-llama`).
- **FR-004**: The `agentix-indexer` crate MUST NOT have a transitive compile-time dependency on any inference backend crate.
- **FR-005**: The `agentix-mcp-server` crate MUST NOT have a transitive compile-time dependency on any inference backend crate.
- **FR-006**: The public API surface shared between crates (traits, types, error types) MUST remain stable across the decomposition — callers outside the changed crates MUST NOT require modification.
- **FR-007**: All existing capabilities (text completion, embedding, search, indexing, MCP tool dispatch) MUST continue to function correctly after decomposition.
- **FR-008**: The crate dependency graph MUST remain acyclic after decomposition.
- **FR-009**: Feature flags that enable or disable inference backends MUST continue to work correctly, and builds with no inference feature enabled MUST still compile.

### Key Entities

- **LLM Backend Crate** (`agentix-llama`): Encapsulates all llama.cpp-based text completion and embedding inference. Depends on the C++ llama library.
- **Infer Core Crate** (`agentix-infer`): Retains shared inference traits, types, context pool, and model store — backend-agnostic, no direct C++ dependency. Future backend crates (e.g. a Whisper audio transcription crate) will follow the same pattern: depend on `agentix-infer`, introduce their own C++ dependency, and require no changes to existing crates.

- **Indexer Crate** (`agentix-indexer`): Encapsulates document ingestion and indexing pipelines. Has C++ dependencies (tree-sitter grammar crates for code parsing) but no dependency on inference backend crates — modifying indexer logic does not trigger llama.cpp recompilation.
- **Search Crate** (`agentix-search`): Encapsulates BM25 and vector search logic plus optional reranking. Has a C++ dependency (fastembed/onnxruntime for reranking, enabled via `RERANK_MODEL`) but no dependency on inference backend crates.
- **MCP Server Crate** (`agentix-mcp-server`): Encapsulates Model Context Protocol tool definitions and protocol handlers. No C++ dependency.
- **Router Crate** (`agentix-router`): Encapsulates backend-selection logic. No C++ dependency.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer modifying only search, indexer, or MCP server source completes a rebuild in under 30 seconds on a warm cache (compared to several minutes when C++ recompilation is triggered).
- **SC-002**: The build dependency graph shows zero edges from the search, indexer, MCP server, or router crates to `agentix-llama` or any other backend crate.
- **SC-003**: All existing integration tests pass without modification after decomposition.
- **SC-004**: A change to only `agentix-llama` produces a rebuild that touches at most `agentix-llama` and its direct dependents — the inference core and all non-inference crates are not rebuilt.
- **SC-005**: Nix builds for the daemon package succeed with the same feature combinations as before decomposition (default, cuda). The `whisper` feature flag may be reserved for the future backend but must not fail the build when unused.

## Assumptions

- The existing `agentix-infer` crate's public trait and type surface will be retained in a shared core crate that both backend crates depend on, preserving the current API contract.
- The daemon crate is the only binary crate and remains responsible for wiring together all component crates at link time. When multiple C++ backend crates coexist (e.g. a future Whisper crate sharing ggml symbols with `agentix-llama`), the `--allow-multiple-definition` linker workaround will be applied at the daemon link level — this is not needed until a second backend lands.
- The root crate currently contains MCP server, indexer, and search binaries; these are extracted to their own crates (`agentix-mcp-server`, `agentix-indexer`). Binary entry points (`mcp-server`, `ingest`) move to their owning crates; the root crate retains only the jail binaries (`claude-jail`, `ax-jail`, `gh-jail-*`).
- Nix packaging will be updated to reference the new crate structure but must continue to produce the same output binaries and NixOS services.
- The decomposition is a pure refactor — no new capabilities are added in this feature.
