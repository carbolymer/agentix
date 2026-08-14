# Tasks: Cargo Crate Decomposition

**Input**: Design documents from `/specs/007-cargo-cleanup/`
**Prerequisites**: plan.md ✓ spec.md ✓ research.md ✓ data-model.md ✓ quickstart.md ✓

**Tests**: No new test tasks — spec does not request TDD. Existing integration tests must pass at each checkpoint (they are verification criteria, not new tasks).

**Organization**: Tasks follow plan phases A–G, grouped by user story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no incomplete-task dependencies)
- **[Story]**: Which user story drives this task

---

## Phase 1: Setup

**Purpose**: Confirm the baseline is green before any files are touched.

- [X] T001 Verify `cargo build --workspace` succeeds with no errors on the current branch (baseline checkpoint — do not proceed until clean)

---

## Phase 2: User Story 1 — Fast Search/Indexer Iteration (Priority: P1) 🎯 MVP

**Goal**: `agentix-search` and `agentix-indexer` exist as standalone crates. A change to either crate does not rebuild any C++ crate.

**Independent Test**: Touch `agentix-search/src/db.rs`, run `cargo build --workspace`, confirm no `llama-cpp-2`, `tree-sitter-*`, or `fastembed` compilation output appears. Repeat with `agentix-indexer/src/ingest/code.rs`.

### agentix-search

- [X] T002 [P] [US1] Create `agentix-search/Cargo.toml` — package `agentix-search`, edition 2021, `[lints] workspace = true`; deps: `sqlx` (postgres+runtime-tokio+tls-rustls+chrono), `reqwest` (json+rustls-tls), `fastembed = "4"`, `tokio = { version = "1", features = ["full"] }`, `anyhow = "1"`, `tracing = "0.1"`
- [X] T003 [P] [US1] Copy `src/db.rs` → `agentix-search/src/db.rs`; no import changes needed (all deps are crate-external)
- [X] T004 [P] [US1] Copy `src/embed.rs` → `agentix-search/src/embed.rs`; no import changes needed
- [X] T005 [P] [US1] Copy `src/rerank.rs` → `agentix-search/src/rerank.rs`; no import changes needed
- [X] T006 [P] [US1] Copy `src/fmt.rs` → `agentix-search/src/fmt.rs`; no import changes needed
- [X] T007 [US1] Create `agentix-search/src/lib.rs` with `pub mod db; pub mod embed; pub mod rerank; pub mod fmt;`

### agentix-indexer (parallel with agentix-search)

- [X] T008 [P] [US1] Create `agentix-indexer/Cargo.toml` — package `agentix-indexer`, edition 2021, `[lints] workspace = true`; deps: same tree-sitter-* and ingest deps as the root `Cargo.toml` ingest section (sqlx, tokio, reqwest, serde, serde_json, sha2, hex, indicatif, clap derive, chrono serde, ignore, glob, tempfile, flate2, tar, zip, tree-sitter + all grammar crates, anyhow, tracing, tracing-subscriber env-filter); `[[bin]] name = "ingest" path = "src/main.rs"`
- [X] T009 [P] [US1] Copy entire `src/ingest/` directory tree → `agentix-indexer/src/ingest/` (all `.rs` files except `main.rs`); no import path changes needed (all imports are external crates or sibling modules via `super::`)
- [X] T010 [P] [US1] Create `agentix-indexer/src/main.rs` from `src/ingest/main.rs`; update the single `use mcp_server::ingest` import (if present) to `use agentix_indexer::ingest`; update any `mod` declarations that previously relied on being under `src/ingest/`
- [X] T011 [US1] Create `agentix-indexer/src/lib.rs` matching the structure in `src/lib.rs`: `pub mod ingest { pub mod code; pub mod crates; pub mod docs; pub mod embed; pub mod hackage; pub mod pypi; pub mod repo_index; pub mod symbols; }`

### Workspace integration

- [X] T012 [US1] Add `"agentix-search"` and `"agentix-indexer"` to the `members` array in the root `Cargo.toml` workspace section
- [X] T013 [US1] Verify `cargo build -p agentix-search` compiles cleanly; then touch `agentix-search/src/db.rs` and time `cargo build --workspace` on a warm cache — confirm no `llama-cpp-2` compilation appears and wall time is < 30s (SC-001)
- [X] T014 [US1] Verify `cargo build -p agentix-indexer` compiles cleanly; then touch `agentix-indexer/src/ingest/code.rs` and time `cargo build --workspace` on a warm cache — confirm no `llama-cpp-2` compilation appears and wall time is < 30s (SC-001)

**Checkpoint**: `agentix-search` and `agentix-indexer` are independent crates in the workspace. User Story 1 acceptance test can now be run.

---

## Phase 3: User Story 3 — Independent Backend Crate Architecture (Priority: P2)

**Goal**: `agentix-infer` has zero C++ dependencies. A new `agentix-llama` crate owns the llamacpp backend. The daemon wires them together.

**Independent Test**: Run `cargo build -p agentix-infer`; confirm no `llama`, `cmake`, or C++ compiler invocations appear. Run `cargo build -p agentix-daemon`; confirm it still succeeds.

- [X] T015 [P] [US3] Create `agentix-llama/Cargo.toml` — package `agentix-llama`, edition 2021, `[lints] workspace = true`; `build = "build.rs"`; `[features] cuda = ["llama-cpp-2/cuda"]` (no default features — llama-cpp-2 is always compiled, cuda is optional on top); deps: `agentix-infer = { path = "../agentix-infer" }`, `llama-cpp-2 = "0.1"` **(non-optional — this crate exists solely to provide the llamacpp backend)**, `tokio = { version = "1", features = ["full"] }`, `async-trait = "0.1"`, `thiserror = "2"`, `tracing = "0.1"`; `[dev-dependencies]` same as `agentix-infer` dev-deps
- [X] T016 [P] [US3] Move `agentix-infer/build.rs` → `agentix-llama/build.rs` (CUDA link-search logic; file content unchanged)
- [X] T017 [P] [US3] Move inference backend integration tests that require LlamaCpp: copy `agentix-infer/tests/complete_integration.rs` → `agentix-llama/tests/complete_integration.rs` and `agentix-infer/tests/embed_integration.rs` → `agentix-llama/tests/embed_integration.rs`; update crate path references (`agentix_infer::` for types, `agentix_llama::LlamaCppBackend` for the backend); keep `agentix-infer/tests/store_integration.rs` in place (no C++ dep)
- [X] T018 [US3] Create `agentix-llama/src/lib.rs` by copying `agentix-infer/src/backend/llamacpp.rs`; rewrite every `crate::` import to `agentix_infer::` (e.g. `agentix_infer::backend::{CompletionStream, InferBackend, LoadedModel}`, `agentix_infer::{Capability, CompletionChunk, ...}`); add `pub use` re-exports for `LlamaCppBackend` and `LlamaCppLoadedModel` at the crate root
- [X] T019 [US3] Add `"agentix-llama"` to the `members` array in root `Cargo.toml`; verify `cargo build -p agentix-llama` compiles (this confirms the new crate and the CUDA build.rs work before touching agentix-infer)
- [X] T020 [US3] Update `agentix-infer/src/backend/mod.rs`: remove the `#[cfg(feature = "llamacpp")] pub mod llamacpp;` line and any associated `pub use llamacpp::...` re-exports
- [X] T021 [US3] Update `agentix-infer/Cargo.toml`: remove `llama-cpp-2` optional dep; remove `[features]` section entirely (there is no explicit `build` field — Cargo auto-detects `build.rs`, so no field needs removing; `build.rs` itself was deleted by T016); remove the dead `candle-core`, `candle-nn`, `candle-transformers` optional deps (no implementing code exists for them); remove `llama-cpp-2` from `[dev-dependencies]` if present; optionally add a `[features] whisper = []` no-op stub per SC-005 so the flag is reservable without breaking builds
- [X] T022 [US3] Update `agentix-daemon/Cargo.toml`: add `agentix-llama = { path = "../agentix-llama" }` dep; change `cuda` feature to `["agentix-llama/cuda"]`; remove `features = ["llamacpp"]` from the `agentix-infer` dep entry
- [X] T023 [US3] Update `agentix-daemon/src/main.rs`: change `use agentix_infer::backend::llamacpp::LlamaCppBackend;` to `use agentix_llama::LlamaCppBackend;`
- [X] T024 [US3] Verify `cargo build -p agentix-infer` (confirm no C++ compiler output); verify `cargo build -p agentix-daemon` (confirm it links via agentix-llama)

**Checkpoint**: `agentix-infer` is a pure-Rust library. `agentix-llama` owns all llamacpp code. Daemon compiles and links correctly.

---

## Phase 4: User Story 2 — Fast MCP Server Iteration (Priority: P2)

**Goal**: `agentix-mcp-server` is its own crate. A change to MCP tool definitions does not rebuild `agentix-search`, `agentix-indexer`, or any C++ crate.

**Independent Test**: Touch `agentix-mcp-server/src/tools.rs`, run `cargo build -p agentix-mcp-server`; confirm only `agentix-mcp-server` recompiles. No tree-sitter or fastembed build output.

**Prerequisite**: Phases 2 (US1) must be complete — `agentix-search` and `agentix-indexer` must exist in the workspace.

- [X] T025 [P] [US2] Create `agentix-mcp-server/Cargo.toml` — package `agentix-mcp-server`, edition 2021, `[lints] workspace = true`; deps: `agentix-search = { path = "../agentix-search" }`, `agentix-indexer = { path = "../agentix-indexer" }`, `rmcp = { version = "0.1", features = ["server", "transport-io", "macros"] }`, `sqlx = { version = "0.8", features = ["postgres", "runtime-tokio", "tls-rustls", "chrono"] }`, `tokio = { version = "1", features = ["full"] }`, `reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls"] }`, `serde = { version = "1", features = ["derive"] }`, `serde_json = "1"`, `anyhow = "1"`, `tracing = "0.1"`, `tracing-subscriber = { version = "0.3", features = ["env-filter"] }`; `[[bin]] name = "mcp-server" path = "src/main.rs"`
- [X] T026 [P] [US2] Copy `src/tools.rs` → `agentix-mcp-server/src/tools.rs`; change every `mcp_server::ingest::` import to `agentix_indexer::ingest::`; change `db::`, `embed::`, `fmt::`, `rerank::` bare module references to `agentix_search::db::`, `agentix_search::embed::`, etc.
- [X] T027 [US2] Copy `src/main.rs` → `agentix-mcp-server/src/main.rs`; remove the `mod db; mod embed; mod fmt; mod rerank;` declarations (those modules are now deps, not inline); add `use agentix_search::{db, embed, fmt, rerank};` or access via full paths; keep `mod tools;` declaration since `tools.rs` is in the same crate
- [X] T028 [US2] Add `"agentix-mcp-server"` to the `members` array in root `Cargo.toml`
- [X] T029 [US2] Verify `cargo build -p agentix-mcp-server` compiles cleanly; then touch `agentix-mcp-server/src/tools.rs` and time `cargo build -p agentix-mcp-server` on a warm cache — confirm no `llama-cpp-2`, `tree-sitter-*`, or `fastembed` compilation appears and wall time is < 30s (SC-001)

**Checkpoint**: All four new crates (`agentix-llama`, `agentix-search`, `agentix-indexer`, `agentix-mcp-server`) are in the workspace and build independently.

---

## Phase 5: User Story 4 — Router Changes Do Not Trigger C++ (Priority: P3)

**Goal**: Document and verify the existing router isolation invariant. `agentix-router` has never depended on any C++ crate; this phase confirms and records that fact.

**Independent Test**: `cargo metadata --format-version 1 | jq '[.resolve.nodes[] | select(.id | startswith("agentix-router")) | .deps[].name]'` — output should contain only `agentix-api` and standard-library crates.

- [X] T030 [US4] Run `cargo metadata --format-version 1` and inspect the resolved dependency graph for `agentix-router`; confirm zero transitive deps on `llama-cpp-2`, any `tree-sitter-*` crate, or `fastembed`
- [ ] T031 [US4] Update `ARCHITECTURE.md` dependency rules section: add an explicit invariant line stating "agentix-router MUST NOT depend on any C++ crate; this is verified by inspecting `cargo metadata`"

**Checkpoint**: Router isolation is documented and verified. All four user stories are now satisfied.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Remove old duplicated code from the root crate, update Nix packaging, clean up docs and lints.

### E: Thin the root crate

- [ ] T032 [P] Delete `src/db.rs`, `src/embed.rs`, `src/rerank.rs`, `src/fmt.rs` from root (owned by `agentix-search` now)
- [ ] T033 [P] Delete `src/ingest/` directory from root entirely (owned by `agentix-indexer` now); also delete `src/ingest/main.rs` (was the entry point, now in agentix-indexer)
- [ ] T034 [P] Delete `src/main.rs`, `src/tools.rs`, `src/lib.rs` from root (owned by `agentix-mcp-server` now)
- [ ] T035 Update root `Cargo.toml`: rename `[package] name` from `mcp-server` to `agentix-jails`; remove all deps that moved to new crates (`rmcp`, `sqlx`, `reqwest`, `fastembed`, `serde`, `serde_json`, `anyhow`, all `tree-sitter-*` crates, `indicatif`, `chrono`, `ignore`, `glob`, `tempfile`, `flate2`, `tar`, `zip`, `clap`, `sha2`, `hex`, `tracing`, `tracing-subscriber`); remove `[[bin]]` entries for `mcp-server` and `ingest`; keep `tokio = { version = "1", features = ["full"] }` only if any remaining jail binary requires it
- [ ] T036 Verify `cargo build --workspace` succeeds with the thinned root crate

### F: Nix packaging

- [ ] T037 Update `depsOnlySrc` letblock in `perSystem/packages.nix`: add manifest paths for the 4 new crates to the `lib.fileset.unions` list (`../agentix-llama/Cargo.toml`, `../agentix-indexer/Cargo.toml`, `../agentix-search/Cargo.toml`, `../agentix-mcp-server/Cargo.toml`)
- [ ] T038 Update the `pkgs.runCommand "crane-deps-src"` shell block in `depsOnlySrc`: add stub creation lines for each new crate entry point — `agentix-llama/src/lib.rs` (stubLib), `agentix-indexer/src/lib.rs` + `agentix-indexer/src/main.rs` (stubLib + stubMain), `agentix-search/src/lib.rs` (stubLib), `agentix-mcp-server/src/main.rs` (stubMain); also remove the now-obsolete `src/main.rs` and `src/ingest/main.rs` stub lines (those entry points moved)
- [ ] T039 Update `agentixSrc` `lib.fileset.unions` in `perSystem/packages.nix`: add `../agentix-llama`, `../agentix-indexer`, `../agentix-search`, `../agentix-mcp-server` source directories
- [ ] T040 Update the `mkBinSrc` helper's `lib.fileset.unions` base set (the Cargo manifest list): add the 4 new crate manifests; update the stub-generation block to create stubs for the 4 new crate entry points; remove stubs for `src/main.rs` and `src/ingest/main.rs`
- [ ] T041 Add 4 new Nix package derivations in `perSystem/packages.nix`: `agentix-llama` using `commonArgs // cudaArgs` (same pattern as `agentixDaemonPkg`, `--package agentix-llama`); `agentix-search` using `commonArgs` (`--package agentix-search`, no binary); `agentix-indexer` replacing the old `ingestPkg` (`--bin ingest`); `agentix-mcp-server` replacing old `mcpServerPkg` (`--bin mcp-server`); expose as `packages.agentix-llama`, `packages.agentix-search`, `packages.agentix-indexer`, `packages.agentix-mcp-server`
- [ ] T042 Update `claudeJailBinDir` paths array in `perSystem/packages.nix`: replace `ingestPkg` with the new `agentix-indexer` derivation variable; replace `mcpServerPkg` with the new `agentix-mcp-server` derivation variable; do the same in `axJailBinDir`
- [ ] T043 [P] Verify `nix build .#agentix-daemon`
- [ ] T044 [P] Verify `nix build .#agentix-mcp-server` and `nix build .#agentix-indexer`

### G: Documentation and quality gates

- [ ] T045 [P] Update `ARCHITECTURE.md`: crate inventory (add 4 new crates with their C++ dep status and dependency edges); dependency graph (new edges to agentix-llama, agentix-search, agentix-indexer, agentix-mcp-server); add a Decisions Log entry for the decomposition rationale
- [ ] T046 [P] Update `CLAUDE.md` Active Technologies list to reference `agentix-llama` instead of raw `llama-cpp-2`; update Project Structure block to include all 10 workspace members
- [ ] T047 Run `cargo fmt --check --workspace`; fix any formatting issues in new crate files
- [ ] T048 Run `cargo clippy -- -D warnings`; fix any warnings (confirm each new crate has `[lints] workspace = true` and no `unwrap_used`/`expect_used` violations)
- [ ] T049 Run `cargo test --workspace`; confirm `agentix-infer/tests/store_integration.rs` still passes and all other existing tests are green

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start here
- **Phase 2 (US1)**: Depends on Phase 1 — `agentix-search` and `agentix-indexer` streams are independent of each other and can run in parallel (T002–T011 are all [P])
- **Phase 3 (US3)**: Depends on Phase 1 only — fully independent of Phase 2; can run in parallel with Phase 2 if staffed
- **Phase 4 (US2)**: **Depends on Phase 2 completion** — `agentix-mcp-server` deps on `agentix-search` and `agentix-indexer` which must be in the workspace first
- **Phase 5 (US4)**: Depends on Phase 3 (workspace must have all 4 new crates for a complete graph check); light-weight, 2 tasks
- **Phase 6 (Polish)**: Depends on Phases 2, 3, 4 complete — root thinning and Nix updates happen last

### User Story Dependencies

- **US1 (P1)**: Start immediately after Phase 1 — no story dependencies
- **US3 (P2)**: Start immediately after Phase 1 — no story dependencies; fully parallel with US1
- **US2 (P2)**: Depends on US1 completion (needs agentix-search + agentix-indexer in workspace)
- **US4 (P3)**: Depends on US3 completion (needs complete workspace for metadata check); two tasks only

### Key Ordering Constraint

```
Phase 1 (T001)
├── Phase 2 US1 (T002–T014) ─────────────────────────────────────────┐
│   [agentix-search + agentix-indexer]                                 │
│                                                                       ▼
└── Phase 3 US3 (T015–T024) ──────────── Phase 4 US2 (T025–T029) ── Phase 5 US4 (T030–T031)
    [agentix-llama + daemon update]                                         │
                                                                            ▼
                                                                     Phase 6 Polish (T032–T049)
```

---

## Parallel Opportunities

### Within Phase 2 (US1)

All agentix-search and agentix-indexer creation tasks touch different files:

```
Run simultaneously:
  T002: Create agentix-search/Cargo.toml
  T008: Create agentix-indexer/Cargo.toml

Run simultaneously:
  T003: Copy db.rs          T009: Copy src/ingest/ tree
  T004: Copy embed.rs
  T005: Copy rerank.rs
  T006: Copy fmt.rs
  T010: Create agentix-indexer/src/main.rs

Then:
  T007: Create agentix-search/src/lib.rs
  T011: Create agentix-indexer/src/lib.rs

Then:
  T012: Update workspace Cargo.toml (single file, do once)
  T013: Verify agentix-search build
  T014: Verify agentix-indexer build
```

### Within Phase 3 (US3)

```
Run simultaneously:
  T015: Create agentix-llama/Cargo.toml
  T016: Move build.rs
  T017: Move integration tests

Then sequentially (logical order):
  T018: Create agentix-llama/src/lib.rs  (copy + adapt llamacpp.rs)
  T019: Add to workspace + verify agentix-llama build
  T020: Remove llamacpp from agentix-infer/src/backend/mod.rs
  T021: Update agentix-infer/Cargo.toml
  T022: Update agentix-daemon/Cargo.toml
  T023: Update agentix-daemon/src/main.rs
  T024: Verify builds
```

### Within Phase 6 Polish

```
Run simultaneously (different files):
  T032: Delete search files from root
  T033: Delete indexer files from root
  T034: Delete mcp-server files from root

Then: T035 (Cargo.toml), T036 (verify)

Then simultaneously:
  T037-T044: Nix packaging tasks (sequential within Nix, but T043+T044 are parallel verify)

Then simultaneously:
  T045: Update ARCHITECTURE.md
  T046: Update CLAUDE.md
  T047: cargo fmt
  T048: cargo clippy (after T047)
  T049: cargo test
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001)
2. Complete Phase 2: User Story 1 (T002–T014)
3. **STOP and VALIDATE**: Verify search/indexer rebuild isolation per US1 independent test
4. Proceed to Phase 3 (US3) and Phase 4 (US2)

### Incremental Delivery

1. Phase 1 + Phase 2 → two new crates in workspace, US1 satisfied
2. Phase 3 → agentix-llama extracted, agentix-infer pure Rust, US3 satisfied
3. Phase 4 → agentix-mcp-server extracted, US2 satisfied
4. Phase 5 → router isolation documented, US4 satisfied
5. Phase 6 → root thinned, Nix updated, docs clean, all gates green

### Solo Strategy (single developer)

Since Phases 2 and 3 are independent, the recommended order for a single developer:
1. T001 (baseline)
2. T002–T014 (US1 — search + indexer, the highest-priority deliverable)
3. T015–T024 (US3 — llama extraction, independent, can be done back-to-back)
4. T025–T029 (US2 — MCP server, now unblocked)
5. T030–T031 (US4 — 2 tasks)
6. T032–T049 (Polish — thinning, Nix, docs)

---

## Notes

- Tasks T003–T011 are copies, not moves — the old files stay in root until Phase 6 (Polish). This keeps `cargo build --workspace` green throughout Phases 2–5.
- T016 is a true move (build.rs must not exist in both agentix-infer and agentix-llama simultaneously), so T016 and T021 must complete before verifying agentix-infer with no build.rs.
- The root crate (`mcp-server` package) still compiles its old binaries during Phases 2–5. The duplicate code is intentional during the transition. Phase 6 removes the old copies.
- Each `Cargo.toml` created for a new crate must include `[lints] workspace = true` — clippy `unwrap_used`/`expect_used` denials are workspace-wide.
- Commit `Cargo.lock` after each phase that adds new workspace members (T012, T019, T028).
