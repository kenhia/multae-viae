# Tasks: Tool Calling & Agentic Loop

**Input**: Design documents from `/specs/003-tool-calling/`  
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add `derive` feature to rig-core, create tools module structure, add shared constants and helpers

- [x] T001 Add `rig-core = { version = "0.35", features = ["derive"] }` dependency to crates/mv-core/Cargo.toml
- [x] T002 Add `reqwest` dependency to crates/mv-core/Cargo.toml for HTTP tool
- [x] T003 Add `tokio = { version = "1", features = ["process", "time"] }` dependency to crates/mv-core/Cargo.toml for shell_exec
- [x] T004 Create tools module structure with mod.rs in crates/mv-core/src/tools/mod.rs
- [x] T005 Add `pub mod tools;` to crates/mv-core/src/lib.rs
- [x] T006 [P] Define shared constants (MAX_TOOL_OUTPUT_CHARS, SHELL_TIMEOUT_SECS, HTTP_TIMEOUT_SECS) and truncation helper in crates/mv-core/src/tools/mod.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Refactor CLI from direct `Prompt::prompt()` to tool-aware agent with preamble and `default_max_turns`

**⚠️ CRITICAL**: The agent wiring must be in place before any tool can be exercised end-to-end

- [x] T007 Refactor `call_ollama` in crates/mv-cli/src/main.rs to build agent with `.preamble()` and `.default_max_turns(10)` instead of bare `client.agent(model).build()`
- [x] T008 Refactor `call_openai` in crates/mv-cli/src/main.rs to build agent with `.preamble()` and `.default_max_turns(10)` instead of bare `client.agent(model).build()`
- [x] T009 Add system preamble constant in crates/mv-cli/src/main.rs describing available tools and agent behavior
- [x] T010 Verify existing tests still pass — no-tool prompts must produce same behavior as before (SC-006 regression check)

**Checkpoint**: Agent builder ready with preamble and max turns — tool wiring can now begin

---

## Phase 3: User Story 1 — Ask a Question That Requires a Tool (Priority: P1) 🎯 MVP

**Goal**: User asks about local files, agent uses `file_list` or `file_read` tool and returns accurate answer

**Independent Test**: `cargo run -p mv-cli -- "What files are in the current directory?"` returns actual directory contents

### Implementation for User Story 1

- [x] T011 [P] [US1] Implement `file_list` tool with `#[rig_tool]` macro in crates/mv-core/src/tools/file_list.rs
- [x] T012 [P] [US1] Implement `file_read` tool with `#[rig_tool]` macro in crates/mv-core/src/tools/file_read.rs
- [x] T013 [US1] Wire `FileList` and `FileRead` tools into agent builder via `.tool()` in crates/mv-cli/src/main.rs
- [x] T014 [US1] Add unit tests for `file_list` tool (valid dir, missing dir, empty dir) in crates/mv-core/src/tools/file_list.rs
- [x] T015 [US1] Add unit tests for `file_read` tool (valid file, missing file, truncation) in crates/mv-core/src/tools/file_read.rs
- [x] T016 [US1] Add telemetry instrumentation (`#[tracing::instrument]`) to `file_list` and `file_read` tool functions in crates/mv-core/src/tools/file_list.rs and crates/mv-core/src/tools/file_read.rs

**Checkpoint**: Agent can list and read files — `"What files are in the current directory?"` returns accurate results

---

## Phase 4: User Story 2 — Execute a Shell Command via Tool (Priority: P2)

**Goal**: User asks a question answerable by a shell command, agent uses `shell_exec` tool with timeout and output truncation

**Independent Test**: `cargo run -p mv-cli -- "What git branch am I on?"` returns actual branch name

### Implementation for User Story 2

- [x] T017 [P] [US2] Implement `shell_exec` tool with `#[rig_tool]` macro, `tokio::process::Command`, and `tokio::time::timeout` in crates/mv-core/src/tools/shell_exec.rs
- [x] T018 [US2] Wire `ShellExec` tool into agent builder via `.tool()` in crates/mv-cli/src/main.rs
- [x] T019 [US2] Add unit tests for `shell_exec` tool (successful command, failed command, output truncation) in crates/mv-core/src/tools/shell_exec.rs
- [x] T020 [US2] Add telemetry instrumentation (`#[tracing::instrument]`) to `shell_exec` tool function in crates/mv-core/src/tools/shell_exec.rs

**Checkpoint**: Agent can execute shell commands — `"What git branch am I on?"` returns correct branch

---

## Phase 5: User Story 3 — Fetch Information from a URL (Priority: P3)

**Goal**: User asks about a URL, agent uses `http_get` tool with timeout and response truncation

**Independent Test**: `cargo run -p mv-cli -- "What is the title of https://example.com?"` references the page title

### Implementation for User Story 3

- [x] T021 [P] [US3] Implement `http_get` tool with `#[rig_tool]` macro and `reqwest` GET with timeout in crates/mv-core/src/tools/http_get.rs
- [x] T022 [US3] Wire `HttpGet` tool into agent builder via `.tool()` in crates/mv-cli/src/main.rs
- [x] T023 [US3] Add unit tests for `http_get` tool (invalid URL, response truncation) in crates/mv-core/src/tools/http_get.rs
- [x] T024 [US3] Add telemetry instrumentation (`#[tracing::instrument]`) to `http_get` tool function in crates/mv-core/src/tools/http_get.rs

**Checkpoint**: Agent can fetch URLs — `"What is the title of https://example.com?"` works

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, architecture updates, and validation

- [x] T025 [P] Update README.md with tool-calling usage examples and available tools section
- [x] T026 [P] Update docs/01-architecture-design.md with tool system architecture
- [x] T027 Add integration tests in crates/mv-cli/tests/cli_tools.rs covering edge cases: hallucinated tool name handling, tool error recovery, and no-tool-needed queries
- [x] T028 Run `just ci` (fmt, clippy -D warnings, test) and fix any issues
- [x] T029 Run quickstart.md validation — manually test each example command
- [x] T030 Update docs/09-roadmap.md — mark Phase 2 tasks as complete

> **Note**: FR-006 (arg validation), FR-007 (result formatting), FR-009 (error handling), and FR-012 (unknown tool rejection) are handled internally by Rig's agentic loop and serde deserialization. T027 verifies a subset of these behaviors end-to-end.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 — BLOCKS all user stories
- **User Stories (Phase 3–5)**: All depend on Phase 2 completion
  - Stories can proceed sequentially in priority order (P1 → P2 → P3)
  - Tools within each story can be built in parallel where marked [P]
- **Polish (Phase 6)**: Depends on all user stories being complete

### Within Each User Story

- Tool implementation before wiring into agent builder
- Unit tests alongside or after tool implementation
- Telemetry instrumentation after tool is functional

### Parallel Opportunities

- T001, T002, and T003: Independent dependency additions, can run in parallel
- T011 and T012: `file_list` and `file_read` tools in separate files, can run in parallel
- T014 and T015: Unit tests for different tools, can run in parallel
- T025 and T026: README and architecture doc updates, can run in parallel

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T006)
2. Complete Phase 2: Foundational agent refactor (T007–T010)
3. Complete Phase 3: File tools — US1 (T011–T016)
4. **STOP and VALIDATE**: `"What files are in the current directory?"` works
5. Deploy/demo if ready

### Incremental Delivery

1. Setup + Foundational → Agent ready with preamble
2. Add file tools (US1) → Test → Agent can read files (MVP!)
3. Add shell_exec (US2) → Test → Agent can run commands
4. Add http_get (US3) → Test → Agent can fetch URLs
5. Polish → Docs updated, CI green, roadmap marked
