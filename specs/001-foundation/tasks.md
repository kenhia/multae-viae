# Tasks: Foundation — Workspace & Local Model Integration

**Input**: Design documents from `/specs/001-foundation/`
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, quickstart.md

**Tests**: Included — constitution mandates TDD (Principle III).

**Organization**: Tasks grouped by user story for independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Workspace root**: `Cargo.toml`, `justfile`
- **Crates**: `crates/mv-core/`, `crates/mv-cli/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Cargo workspace, crate scaffolding, justfile, .gitignore

- [x] T001 Create workspace root `Cargo.toml` with members `crates/mv-core` and `crates/mv-cli`
- [x] T002 [P] Create `crates/mv-core/Cargo.toml` with dependencies: thiserror, serde, serde_json
- [x] T003 [P] Create `crates/mv-cli/Cargo.toml` with dependencies: mv-core (path), rig-core 0.35, tokio, clap (derive), anyhow, tracing, tracing-subscriber (env-filter)
- [x] T004 [P] Create `justfile` with recipes: build, test, check, fmt, lint, run, ci
- [x] T005 [P] Update `.gitignore` to include `/target`, `.scratch-agent/`, `.scratch/`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and error handling that all user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T006 Define `MvError` enum in `crates/mv-core/src/lib.rs` with variants: EmptyPrompt, BackendUnreachable, ModelNotFound, CompletionFailed — derive `thiserror::Error` with actionable user messages per data-model.md
- [x] T007 [P] Define `BackendConfig` struct in `crates/mv-core/src/lib.rs` with fields: endpoint (String, default `http://localhost:11434`), model (String, default `qwen3:4b`)
- [x] T008 [P] Write unit tests for `MvError` display messages in `crates/mv-core/src/lib.rs` (inline tests module)
- [x] T009 [P] Write unit test for `BackendConfig` default values in `crates/mv-core/src/lib.rs` (inline tests module)

**Checkpoint**: Foundation ready — `cargo test -p mv-core` passes with all tests green

---

## Phase 3: User Story 1 — Send a Prompt to a Local Model (Priority: P1) 🎯 MVP

**Goal**: CLI accepts a prompt, sends it to Ollama via Rig, prints the response to stdout (plain text or JSON)

**Independent Test**: `cargo run -p mv-cli -- "What is Rust?"` prints a model response to stdout; `cargo run -p mv-cli -- --json "What is Rust?"` prints JSON

### Tests for User Story 1 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T010 [US1] Write unit test for prompt validation (empty/whitespace rejection) in `crates/mv-core/src/lib.rs`
- [x] T011 [US1] Write unit test for empty model response handling (no panic, exit 0) in `crates/mv-core/src/lib.rs`
- [x] T012 [US1] Write integration test for CLI argument parsing (valid prompt, --model, --endpoint, --json flags) in `crates/mv-cli/tests/cli_args.rs`
- [x] T013 [US1] Write integration test for CLI exit codes (0 on success, 1 on error) in `crates/mv-cli/tests/cli_args.rs`

### Implementation for User Story 1

- [x] T014 [US1] Implement prompt validation function in `crates/mv-core/src/lib.rs` — reject empty/whitespace, return `MvError::EmptyPrompt`
- [x] T015 [US1] Define CLI args struct with clap derive in `crates/mv-cli/src/main.rs` — positional PROMPT, --model, --endpoint, --json, -v verbosity
- [x] T016 [US1] Implement `run()` async function in `crates/mv-cli/src/main.rs` — create Ollama client via Rig, build agent, send prompt, print response to stdout
- [x] T017 [US1] Implement error handling in `main()` in `crates/mv-cli/src/main.rs` — map Rig errors to `MvError` variants, print to stderr (or JSON error object if --json), exit code 1
- [x] T018 [US1] Handle edge case: empty model response (print nothing or empty string, exit 0) in `crates/mv-cli/src/main.rs`
- [x] T019 [US1] Implement `--json` output mode in `crates/mv-cli/src/main.rs` — wrap response in `{"response": "..."}` JSON object; errors as `{"error": "..."}` to stdout

**Checkpoint**: `cargo run -p mv-cli -- "Hello"` returns a model response. `cargo run -p mv-cli -- --json "Hello"` returns JSON. `cargo test --workspace` passes.

---

## Phase 4: User Story 2 — Structured Logging Output (Priority: P2)

**Goal**: Structured tracing output on stderr, controllable by -v flag and RUST_LOG

**Independent Test**: `cargo run -p mv-cli -- -vv "Hello" 2>debug.log` shows structured log lines in debug.log, model response on stdout

### Tests for User Story 2 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T020 [US2] Write integration test verifying default verbosity produces no stderr output in `crates/mv-cli/tests/cli_logging.rs`
- [x] T021 [US2] Write integration test verifying -v flag produces structured log lines on stderr in `crates/mv-cli/tests/cli_logging.rs`

### Implementation for User Story 2

- [x] T022 [US2] Implement tracing subscriber initialization in `crates/mv-cli/src/main.rs` — EnvFilter, fmt layer to stderr, verbosity mapped from -v count
- [x] T023 [US2] Add tracing spans and events to `run()` in `crates/mv-cli/src/main.rs` — info for connect/request/response, debug for config details
- [x] T024 [US2] Ensure RUST_LOG environment variable overrides -v flag in `crates/mv-cli/src/main.rs`

**Checkpoint**: Default run shows only response on stdout. `-vv` shows structured logs on stderr. `cargo test --workspace` passes.

---

## Phase 5: User Story 3 — Workspace Builds and Passes Checks (Priority: P3)

**Goal**: All quality gates pass: fmt, clippy, test

**Independent Test**: `just ci` succeeds with zero warnings and zero failures

### Implementation for User Story 3

- [x] T025 [US3] Run `cargo fmt --all` and fix any formatting issues across all crates
- [x] T026 [US3] Run `cargo clippy --all-targets --all-features -- -D warnings` and fix all warnings across all crates
- [x] T027 [US3] Verify `just ci` recipe runs `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test --workspace` and exits 0
- [x] T028 [US3] Run `just ci` and confirm zero warnings, zero failures

**Checkpoint**: `just ci` passes clean. All quality gates green.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, cleanup, and quickstart validation

- [x] T029 [P] Update `README.md` with build/run instructions, prerequisites (Ollama, Rust, just), and CLI usage examples
- [x] T030 Run quickstart.md validation — follow quickstart.md steps on a clean build and verify all commands work
- [x] T031 Verify all acceptance scenarios from spec.md: happy path, Ollama down, no model, empty prompt, --json output

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - US1 (P1) should complete first — US2 builds on its code
  - US3 is a validation pass over the entire workspace
- **Polish (Phase 6)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) — MVP, no dependencies on other stories
- **User Story 2 (P2)**: Can start after US1 is complete (adds tracing to `run()` implemented in US1)
- **User Story 3 (P3)**: Can start after US1 and US2 are complete (validates the full workspace)

### Within Each User Story

- Tests MUST be written and FAIL before implementation
- Core types before CLI integration
- Error handling after happy path
- Story complete before moving to next priority

### Parallel Opportunities

- T002, T003, T004, T005 can all run in parallel (Phase 1 — different files)
- T007, T008, T009 can run in parallel within Phase 2 (independent types/tests)
- T010, T011, T012, T013 can run in parallel within US1 tests (different test files)
- T029 can run in parallel with T030, T031 in Phase 6

---

## Parallel Example: Phase 1 Setup

```text
T001 ──► T002 ─┐
         T003 ─┤
         T004 ─┤──► Phase 2
         T005 ─┘
```

## Parallel Example: User Story 1

```text
T010 ─┐
T011 ─┤──► T014 ──► T015 ──► T016 ──► T017 ──► T018 ──► T019
T012 ─┤
T013 ─┘
```

---

## Implementation Strategy

1. **MVP first**: Phase 1 → Phase 2 → Phase 3 (US1) delivers a working CLI with JSON output
2. **Incremental delivery**: Each phase is a testable increment
3. **TDD throughout**: Red → Green → Refactor per constitution Principle III
4. **Quality gates**: `just ci` after every phase to catch regressions
