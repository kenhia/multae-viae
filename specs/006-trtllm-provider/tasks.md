# Tasks: TensorRT-LLM Provider

**Input**: Design documents from `/specs/006-trtllm-provider/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli.md, quickstart.md

**Tests**: Tests are included — the spec references unit and integration tests,
and the project constitution (Principle III) requires TDD.

**Organization**: Tasks are grouped by user story to enable independent
implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

---

## Phase 1: Setup

**Purpose**: Extend model registry and provider infrastructure to support
`trtllm` before any user story work begins.

- [x] T001 Add `served_name`, `architecture`, `quant`, `expected_vram_gb` fields to `ModelEntry` in crates/mv-core/src/lib.rs (note: `served_name` is a cross-cutting feature benefiting all providers but primarily needed for TRT-LLM)
- [x] T002 Add `"trtllm" => Locality::Local` to `Locality::from_provider()` in crates/mv-core/src/lib.rs
- [x] T003 Add `"trtllm"` default endpoint `http://localhost:8000/v1` to `ModelEntry::endpoint()` in crates/mv-core/src/lib.rs
- [x] T004 Create `trtllm` module with `pub mod trtllm;` in crates/mv-core/src/lib.rs and create crates/mv-core/src/trtllm/mod.rs

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Health check module — required by US1 and US3 before prompt
dispatch can be wired.

**CRITICAL**: No user story work can begin until this phase is complete.

- [x] T005 Implement `HealthCheckResult` enum and `check_health()` async function with 2-second reqwest timeout in crates/mv-core/src/trtllm/health.rs
- [x] T006 [P] Add unit tests for `check_health()` in crates/mv-core/src/trtllm/health.rs (test URL construction, healthy/unhealthy/unreachable variants)
- [x] T007 [P] Add unit tests for new `ModelEntry` fields (`served_name` deserialization, `from_provider("trtllm")`, default endpoint) in crates/mv-core/src/lib.rs

**Checkpoint**: Foundation ready — `trtllm` module exists with health check,
ModelEntry supports new fields, all unit tests pass.

---

## Phase 3: User Story 1 — Send a Prompt to a TRT-LLM Model (Priority: P1) MVP

**Goal**: A user sends a prompt via `mv-cli -m <trtllm-model> "prompt"` and
receives a response from a TRT-LLM-served model.

**Independent Test**: Add a `trtllm` entry to `models.yaml`, run
`mv-cli -m llama-3_1-8b-fp8 "Hello"` against a running `trtllm-serve`, verify
coherent response.

### Implementation for User Story 1

- [x] T008 [US1] Add `"trtllm"` arm to provider match in `run_prompt()` — call health check then `call_openai()` with `api_key = "tensorrt_llm"` in crates/mv-cli/src/main.rs
- [x] T009 [US1] Pass `served_name` (if set) instead of `id` as the model name to `call_openai()` in crates/mv-cli/src/main.rs
- [x] T010 [US1] Update `BackendUnreachable` error message for trtllm — replace "Is Ollama running?" with actionable TRT-LLM hint in crates/mv-core/src/lib.rs
- [x] T011 [P] [US1] Add CLI integration test for trtllm provider dispatch (unsupported-provider error replaced, config parsing with new fields) in crates/mv-cli/tests/cli_trtllm.rs (integration test — runs after implementation per TDD integration-level pattern)

**Checkpoint**: `mv-cli -m <trtllm-model> "Hello"` works against a running
`trtllm-serve`. Health check runs before prompt. Actionable error when server is
down.

---

## Phase 4: User Story 2 — Use TRT-LLM Models in Workflows (Priority: P2)

**Goal**: Workflow prompt steps route through the TRT-LLM provider when the
step's model has `provider: trtllm`.

**Independent Test**: Create a workflow YAML with `model: llama-3_1-8b-fp8`,
run `mv-cli workflow run`, verify the step executes against TRT-LLM.

### Implementation for User Story 2

- [x] T012 [US2] Add `"trtllm"` arm to provider match in `RigPromptExecutor::execute_prompt()` — call health check then `call_openai()` with `api_key = "tensorrt_llm"` in crates/mv-cli/src/main.rs
- [x] T013 [US2] Pass `served_name` (if set) instead of `id` in `RigPromptExecutor` trtllm arm in crates/mv-cli/src/main.rs

**Checkpoint**: Workflows with trtllm-backed model steps execute correctly.

---

## Phase 5: User Story 3 — Health Check Before Prompt (Priority: P3)

**Goal**: When `trtllm-serve` is unreachable, the user gets a clear error within
2 seconds instead of a 30-second timeout.

**Independent Test**: Stop `trtllm-serve`, run a prompt targeting a `trtllm`
model, verify "TRT-LLM server not reachable" error with startup hint.

### Implementation for User Story 3

Note: The health check function itself was built in Phase 2 (T005) and wired
into the prompt path in T008/T012. This phase covers the error formatting and
edge cases.

- [x] T014 [US3] Format health check failure into actionable CLI error with endpoint and `trtllm-serve` hint in crates/mv-cli/src/main.rs
- [x] T015 [P] [US3] Add CLI integration test verifying error output when trtllm endpoint is unreachable (assert stderr contains hint; also test edge case: server running but model not loaded returns 404/model-not-found error) in crates/mv-cli/tests/cli_trtllm.rs

**Checkpoint**: Unreachable TRT-LLM server produces a clear, actionable error
within 2 seconds.

---

## Phase 6: User Story 4 — TRT-LLM Telemetry Attributes (Priority: P4)

**Goal**: OpenTelemetry spans for TRT-LLM calls include
`gen_ai.system = "trtllm"` and optional model metadata attributes.

**Independent Test**: Send a prompt with `--otlp`, inspect Jaeger trace, verify
`gen_ai.system = "trtllm"` and metadata attributes appear.

### Implementation for User Story 4

- [x] T016 [US4] Add `call_trtllm()` wrapper function with `#[tracing::instrument]` setting `gen_ai.system = "trtllm"` and optional `trtllm.architecture`, `trtllm.quant`, `trtllm.expected_vram_gb` span fields in crates/mv-cli/src/main.rs
- [x] T017 [US4] Update `run_prompt()` and `RigPromptExecutor` trtllm arms to call `call_trtllm()` instead of `call_openai()` directly in crates/mv-cli/src/main.rs
- [x] T018 [P] [US4] Add unit test verifying trtllm telemetry span attributes are set correctly in crates/mv-cli/tests/cli_trtllm.rs

**Checkpoint**: TRT-LLM calls produce spans with `gen_ai.system = "trtllm"` and
metadata attributes in OTLP traces.

---

## Phase 7: User Story 5 — Tool Calling Through TRT-LLM (Priority: P5)

**Goal**: Built-in tools and MCP tools work through TRT-LLM models that have
tool parser support, reusing the same `call_openai()` / `call_trtllm()` path.

**Independent Test**: Start `trtllm-serve` with a tool-capable model and
`--tool_parser auto`, send a prompt requiring tool use (e.g., "List files in
the current directory"), verify tool invocation and result incorporation.

### Implementation for User Story 5

- [x] T019 [US5] Verify tool definitions are passed through `call_trtllm()` → `call_openai()` to `trtllm-serve` — no code change expected, validate with manual test in crates/mv-cli/src/main.rs
- [x] T020 [P] [US5] Add CLI integration test for tool-calling with trtllm provider (verify tool definitions appear in request, graceful degradation when model lacks tool support) in crates/mv-cli/tests/cli_trtllm.rs

**Checkpoint**: Tool calling works end-to-end through TRT-LLM models with tool
parser support. Models without tool support degrade gracefully.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, examples, and validation across all stories.

- [x] T021 [P] Add `trtllm` provider section to docs/01-architecture-design.md
- [x] T022 [P] Add example `trtllm` entries to models.yaml with comments
- [x] T023 [P] Update README.md with TRT-LLM provider in feature list and configuration example
- [x] T024 Run quickstart.md validation — verify all steps execute correctly against a live `trtllm-serve` instance
- [x] T025 Run `cargo fmt && cargo clippy && cargo test` — verify all existing tests still pass (no regression)

**Checkpoint**: All documentation updated, all tests pass, quickstart validated.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Phase 1 (T004 creates the module)
- **US1 (Phase 3)**: Depends on Phase 2 (health check module)
- **US2 (Phase 4)**: Depends on Phase 3 (same dispatch pattern, same
  `call_openai()` reuse)
- **US3 (Phase 5)**: Depends on Phase 3 (health check already wired in T008)
- **US4 (Phase 6)**: Depends on Phase 4 (wraps the trtllm arms from T008/T012)
- **US5 (Phase 7)**: Depends on Phase 6 (uses `call_trtllm()` with tool
  server handle)
- **Polish (Phase 8)**: Depends on all user stories

### User Story Dependencies

- **US1 (P1)**: Depends on Foundational only — MVP deliverable
- **US2 (P2)**: Depends on US1 (same dispatch pattern applied to workflow
  executor)
- **US3 (P3)**: Depends on US1 (health check error formatting for the path
  wired in US1)
- **US4 (P4)**: Depends on US2 (wraps trtllm arms in both `run_prompt` and
  `RigPromptExecutor`)
- **US5 (P5)**: Depends on US4 (validates tool passing through `call_trtllm()`)

### Within Each User Story

- Implementation before integration tests
- Core dispatch before error formatting
- Error formatting before telemetry wrapping

### Parallel Opportunities

Phase 2:

```text
T006 (health check tests) ║ T007 (ModelEntry field tests)
```

Phase 3:

```text
T008–T010 (sequential) → T011 (integration test, parallel with US2 start)
```

Phase 7:

```text
T019 (tool validation) → T020 (tool calling integration test)
```

Phase 8:

```text
T021 (arch docs) ║ T022 (models.yaml) ║ T023 (README)
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T004)
2. Complete Phase 2: Foundational (T005–T007)
3. Complete Phase 3: User Story 1 (T008–T011)
4. **STOP and VALIDATE**: `mv-cli -m <trtllm-model> "Hello"` works
5. Run `cargo test` — all 134+ existing tests still pass

### Incremental Delivery

1. Setup + Foundational → trtllm module exists with health check
2. Add US1 → Direct prompts work → MVP!
3. Add US2 → Workflows work with trtllm models
4. Add US3 → Actionable errors on server down
5. Add US4 → Telemetry spans distinguish trtllm from other providers
6. Add US5 → Tool calling validated through TRT-LLM
7. Polish → Docs, examples, full validation

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- `served_name` support benefits all providers but is primarily needed for
  TRT-LLM where server-side names are HuggingFace paths
- Health check URL: strip `/v1` from endpoint, append `/health`
- `call_openai()` reuse means no new HTTP client code for prompt execution
- All new fields on `ModelEntry` are `Option<T>` — backward compatible with
  existing `models.yaml` files
