# Tasks: MCP Integration

**Input**: Design documents from `specs/004-mcp-integration/`  
**Prerequisites**: plan.md (required), spec.md (required for user stories), research.md, data-model.md, contracts/cli.md

**Tests**: TDD is mandatory per Constitution Principle III. Tests are written first and must fail before implementation.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add rmcp dependency, create MCP module structure, add CLI flag

- [x] T001 Add `rmcp` dependency (client feature) to `crates/mv-core/Cargo.toml` and create `crates/mv-core/src/mcp/mod.rs` module with placeholder submodules
- [x] T002 Add `--mcp-config` CLI flag to `crates/mv-cli/src/main.rs` per contracts/cli.md

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: MCP server configuration parsing and validation — MUST complete before any user story

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

### Tests

- [x] T003 [P] Write unit tests for McpServerConfig YAML parsing (valid stdio, valid http, missing fields, duplicates) in `crates/mv-core/src/mcp/config.rs` (tests module)

### Implementation

- [x] T004 Implement `McpTransportType`, `McpServerConfig`, `McpServersConfig` structs with serde deserialization and validation in `crates/mv-core/src/mcp/config.rs`
- [x] T005 Implement `McpServersConfig::load()` and `McpServersConfig::resolve()` (explicit path vs. default file discovery) in `crates/mv-core/src/mcp/config.rs`
- [x] T006 Add MCP config error variants to the project error type in `crates/mv-core/src/lib.rs`

**Checkpoint**: Config parsing works — `McpServersConfig::load("mcp-servers.yaml")` parses and validates correctly. All T003 tests pass.

---

## Phase 3: User Story 1 — Connect to a Local MCP Server and Use Its Tools (Priority: P1) 🎯 MVP

**Goal**: Spawn stdio-based MCP server processes, perform protocol handshake, discover tools, invoke tools via the agentic loop

**Independent Test**: Configure the reference filesystem MCP server, run `mv-cli "List the files in /tmp"`, verify the agent uses the MCP tool

### Tests for User Story 1

- [x] T007 [P] [US1] Write unit tests for `McpClient::connect_stdio()` — successful handshake mock, spawn failure, handshake timeout — in `crates/mv-core/src/mcp/client.rs` (tests module)
- [x] T008 [P] [US1] Write unit tests for `McpToolAdapter` — implements Rig `Tool` trait, converts call args, converts results, handles errors — in `crates/mv-core/src/mcp/registry.rs` (tests module)

### Implementation

- [x] T009 [US1] Implement `McpClient` struct with `connect_stdio()` method — spawn child process via `TokioChildProcess` with env vars from `McpServerConfig.env` passed via `Command::envs()`, perform MCP handshake with configurable timeout (default 5s), discover tools via `list_all_tools()`, skip tools with invalid or empty parameter schemas (log warning) — in `crates/mv-core/src/mcp/client.rs`
- [x] T010 [US1] Implement `McpToolAdapter` struct that implements Rig's `Tool` trait — bridge `call()` to rmcp `call_tool()`, convert MCP tool results to Rig `ToolResult`, apply output truncation, skip tools with malformed input schemas during adapter creation (log warning) — in `crates/mv-core/src/mcp/registry.rs`
- [x] T011 [US1] Implement `connect_all_servers()` function — iterate McpServerConfig list, connect each stdio server, collect McpToolAdapters, handle startup failures gracefully (log + skip) — in `crates/mv-core/src/mcp/client.rs`
- [x] T012 [US1] Wire MCP into CLI startup — load MCP config, call `connect_all_servers()`, register McpToolAdapters on the Rig agent builder alongside built-in tools — in `crates/mv-cli/src/main.rs`
- [x] T013 [US1] Implement graceful shutdown — send MCP shutdown to connected servers, terminate child processes on CLI exit — in `crates/mv-core/src/mcp/client.rs`

**Checkpoint**: Stdio MCP servers work end-to-end. `mv-cli "List the files in /tmp"` with a filesystem MCP server configured returns accurate results.

---

## Phase 4: User Story 2 — MCP Tools Appear Alongside Built-in Tools (Priority: P2)

**Goal**: MCP tools merge seamlessly into the unified tool set; name collisions are handled deterministically

**Independent Test**: Configure an MCP server with a unique tool, verify the model selects and invokes it alongside built-in tools

### Tests for User Story 2

- [x] T014 [P] [US2] Write unit tests for tool merging — no collisions, built-in takes precedence over MCP, cross-server namespace collisions resolved — in `crates/mv-core/src/mcp/registry.rs` (tests module)

### Implementation

- [x] T015 [US2] Implement tool name collision detection and resolution in `crates/mv-core/src/mcp/registry.rs` — built-in tools take precedence with warning log, cross-server duplicates namespaced as `server_name.tool_name`
- [x] T016 [US2] Implement `merge_tools()` function that takes built-in tool names and MCP tools, returns the deduplicated set with collision warnings — in `crates/mv-core/src/mcp/registry.rs`

**Checkpoint**: With both built-in and MCP tools configured, the model sees a single unified tool list. Collisions produce warnings, not errors.

---

## Phase 5: User Story 3 — Connect to a Remote MCP Server over HTTP (Priority: P3)

**Goal**: Connect to HTTP-based MCP servers, discover and invoke their tools identically to stdio servers

**Independent Test**: Start a reference MCP server on a local HTTP endpoint, configure it, verify tool discovery and invocation works

### Tests for User Story 3

- [x] T017 [P] [US3] Write unit tests for `McpClient::connect_http()` — successful connection, unreachable server, mid-session disconnection — in `crates/mv-core/src/mcp/client.rs` (tests module)

### Implementation

- [x] T018 [US3] Implement `McpClient::connect_http()` method — connect via `StreamableHttpClientTransport`, perform MCP handshake, discover tools — in `crates/mv-core/src/mcp/client.rs`
- [x] T019 [US3] Update `connect_all_servers()` to dispatch on transport type — call `connect_stdio()` for stdio, `connect_http()` for http — in `crates/mv-core/src/mcp/client.rs`
- [x] T020 [US3] Handle mid-session HTTP disconnection — detect connection errors on tool invocation, return error to model, log warning — in `crates/mv-core/src/mcp/client.rs`

**Checkpoint**: Both stdio and HTTP MCP servers connect, discover tools, and serve tool calls. Unreachable HTTP servers are skipped gracefully.

---

## Phase 6: User Story 4 — MCP Tool Calls Appear in Telemetry (Priority: P4)

**Goal**: Every MCP tool invocation emits an OpenTelemetry span with server name, tool name, transport type, result status, and duration

**Independent Test**: Run a query triggering an MCP tool call with `--otlp`, verify span in Jaeger has MCP-specific attributes

### Tests for User Story 4

- [x] T021 [P] [US4] Write unit tests verifying `McpToolAdapter::call()` creates a tracing span with expected attributes — in `crates/mv-core/src/mcp/registry.rs` (tests module)

### Implementation

- [x] T022 [US4] Add `#[tracing::instrument]` to `McpToolAdapter::call()` with span attributes `tool.name`, `mcp.server.name`, `mcp.transport`, `tool.result.status` — in `crates/mv-core/src/mcp/registry.rs`
- [x] T023 [US4] Record error status and message on span when MCP tool call fails — in `crates/mv-core/src/mcp/registry.rs`

**Checkpoint**: MCP tool calls appear in Jaeger traces with all MCP-specific attributes. Failed calls show error status.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, validation, cleanup

- [x] T024 [P] Add MCP integration section to `docs/04-mcp-integration.md` reflecting implemented design
- [x] T025 [P] Update `README.md` with MCP server configuration instructions
- [x] T026 Run `specs/004-mcp-integration/quickstart.md` validation end-to-end
- [x] T027 Verify built-in tools (file_list, file_read, shell_exec, http_get) still work correctly when MCP servers are configured — regression check for SC-006
- [x] T028 Integration test with reference filesystem MCP server — configure, connect, invoke `list_directory`, verify correct results — validates SC-007 in `crates/mv-cli/tests/cli_tools.rs`
- [x] T029 Run pre-commit checks: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Story 1 (Phase 3)**: Depends on Foundational (Phase 2) — No dependencies on other stories
- **User Story 2 (Phase 4)**: Depends on Phase 3 (US1) — needs MCP tools in registry to test merging
- **User Story 3 (Phase 5)**: Depends on Phase 3 (US1) — extends McpClient with HTTP transport
- **User Story 4 (Phase 6)**: Depends on Phase 3 (US1) — instruments existing McpToolAdapter
- **Polish (Phase 7)**: Depends on all user stories being complete

### Within Each User Story

- Tests MUST be written and FAIL before implementation (TDD per Constitution Principle III)
- Models/types before services
- Services before integration
- Core implementation before CLI wiring
- Story complete before moving to next priority

### Parallel Opportunities

- T003 (config tests) can run in parallel during Phase 2
- T007, T008 (US1 tests) can run in parallel
- T014 (US2 tests) can run in parallel with US1 implementation
- T017 (US3 tests) can run in parallel with US2 implementation
- T021 (US4 tests) can run in parallel with US3 implementation
- T024, T025 (docs) can run in parallel during Polish

---

## Parallel Example: User Story 1

```bash
# Write tests first (parallel):
Task T007: "Unit tests for McpClient::connect_stdio()"
Task T008: "Unit tests for McpToolAdapter"

# Implement sequentially:
Task T009: "Implement McpClient with connect_stdio()"       # T007 tests now pass
Task T010: "Implement McpToolAdapter (Rig Tool trait)"       # T008 tests now pass
Task T011: "Implement connect_all_servers()"                 # Orchestration
Task T012: "Wire MCP into CLI startup"                       # End-to-end works
Task T013: "Implement graceful shutdown"                     # Cleanup
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T002)
2. Complete Phase 2: Foundational — config parsing (T003–T006)
3. Complete Phase 3: User Story 1 — stdio MCP connectivity (T007–T013)
4. **STOP and VALIDATE**: Test with reference filesystem MCP server
5. This delivers a working MCP client that connects to local servers

### Incremental Delivery

1. Setup + Foundational → Config parsing works
2. User Story 1 → Stdio MCP servers work end-to-end (MVP!)
3. User Story 2 → Tool merging handles collisions correctly
4. User Story 3 → HTTP MCP servers also work
5. User Story 4 → Full observability of MCP calls
6. Polish → Documentation and final validation
7. Each story adds value without breaking previous stories

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story is independently completable and testable
- TDD is mandatory: write tests first, verify they fail, then implement
- Commit after each task or logical group
- Stop at any checkpoint to validate story independently
