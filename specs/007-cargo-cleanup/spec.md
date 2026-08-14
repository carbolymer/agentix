# Feature Specification: Cargo Crate Decomposition

**Feature Branch**: `007-cargo-cleanup`
**Created**: 2026-08-13
**Status**: Draft
**Input**: User description: "Break agentix-infer into agentix-llama and agentix-whisper crates, extract agentix-indexer, agentix-search, and agentix-mcp-server from root crate so that router, indexer, search, and mcp-server changes do not trigger C++ crate rebuilds"

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

### User Story 3 - LLM and Whisper Backend Isolation (Priority: P2)

The LLM text-generation backend and the Whisper audio-transcription backend are separate crates. A change to the LLM backend does not rebuild the Whisper backend, and vice versa.

**Why this priority**: Each backend embeds a large C++ library. Keeping them separate means a developer working on only one does not pay the compile cost of both.

**Independent Test**: Modify only the LLM backend wrapper, rebuild, and confirm the Whisper backend crate is not recompiled. Then do the inverse.

**Acceptance Scenarios**:

1. **Given** both backends have been compiled, **When** only LLM backend source changes, **Then** only the LLM backend crate and its dependents recompile — the Whisper backend crate is unchanged.
2. **Given** both backends have been compiled, **When** only Whisper backend source changes, **Then** only the Whisper backend crate and its dependents recompile — the LLM backend crate is unchanged.

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
- **FR-002**: The LLM text-generation backend and the Whisper audio-transcription backend MUST reside in separate crates with no direct dependency between them.
- **FR-003**: Search functionality MUST be reachable from the daemon without a direct compile-time dependency on inference backend crates.
- **FR-004**: Indexing functionality MUST be reachable from the daemon without a direct compile-time dependency on inference backend crates.
- **FR-005**: The MCP server implementation MUST be reachable from the daemon without a direct compile-time dependency on inference backend crates.
- **FR-006**: The public API surface shared between crates (traits, types, error types) MUST remain stable across the decomposition — callers outside the changed crates MUST NOT require modification.
- **FR-007**: All existing capabilities (text completion, embedding, transcription, search, indexing, MCP tool dispatch) MUST continue to function correctly after decomposition.
- **FR-008**: The crate dependency graph MUST remain acyclic after decomposition.
- **FR-009**: Feature flags that enable or disable inference backends MUST continue to work correctly, and builds with no inference feature enabled MUST still compile.

### Key Entities

- **LLM Backend Crate**: Encapsulates all llama.cpp-based text completion and embedding inference. Depends on the C++ llama library.
- **Whisper Backend Crate**: Encapsulates all whisper-rs-based audio transcription inference. Depends on the C++ whisper library.
- **Infer Core Crate**: Retains shared inference traits, types, context pool, and model store — backend-agnostic, no direct C++ dependency.
- **Indexer Crate**: Encapsulates document ingestion and indexing pipelines. No C++ dependency.
- **Search Crate**: Encapsulates BM25 and vector search logic. No C++ dependency.
- **MCP Server Crate**: Encapsulates Model Context Protocol tool definitions and protocol handlers. No C++ dependency.
- **Router Crate**: Encapsulates backend-selection logic. No C++ dependency.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A developer modifying only search, indexer, or MCP server source completes a rebuild in under 30 seconds on a warm cache (compared to several minutes when C++ recompilation is triggered).
- **SC-002**: The build dependency graph shows zero edges from the search, indexer, MCP server, or router crates to the LLM or Whisper backend crates.
- **SC-003**: All existing integration tests pass without modification after decomposition.
- **SC-004**: A change to only the LLM backend crate produces a rebuild that touches at most the LLM backend crate and its direct dependents — the Whisper backend crate is not rebuilt.
- **SC-005**: Nix builds for the daemon package succeed with the same feature combinations as before decomposition (default, whisper, cuda).

## Assumptions

- The existing `agentix-infer` crate's public trait and type surface will be retained in a shared core crate that both backend crates depend on, preserving the current API contract.
- The daemon crate is the only binary crate and remains responsible for wiring together all component crates at link time; the `--allow-multiple-definition` linker workaround for shared ggml symbols between the LLM and Whisper C++ libraries remains at the daemon link level.
- The root crate currently contains MCP server, indexer, and search binaries; these will be extracted to their own library crates. Binary entry points may remain in the root crate or move to the new crates.
- Nix packaging will be updated to reference the new crate structure but must continue to produce the same output binaries and NixOS services.
- The decomposition is a pure refactor — no new capabilities are added in this feature.
