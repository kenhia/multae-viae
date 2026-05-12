# Tasks: DSL Workflow Engine

**Input**: Design documents from `specs/005-dsl-engine/`  
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli.md, quickstart.md

**Tests**: TDD is mandated by the project constitution. Tests are included for each story.

**Organization**: Tasks grouped by user story. Each story is independently implementable and testable.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story (US1–US5)
- Exact file paths included in all descriptions

---

## Phase 1: Setup

**Purpose**: Add minijinja dependency and create the workflow module skeleton

- [x] T001 Add `minijinja` dependency to `crates/mv-core/Cargo.toml`
- [x] T002 Create workflow module skeleton with `crates/mv-core/src/workflow/mod.rs` exposing submodules (types, parser, validate, template, engine)
- [x] T003 [P] Create `crates/mv-core/src/workflow/types.rs` with all DSL types from data-model.md (Workflow, WorkflowDefaults, WorkflowInput, InputType, Step enum, PromptStep, ToolStep, TransformStep, ErrorAction, RetryConfig, BackoffStrategy, WorkflowOutput) using `#[serde(deny_unknown_fields)]`
- [x] T004 [P] Add workflow error variants to `MvError` in `crates/mv-core/src/lib.rs` (WorkflowParseError, WorkflowValidationError, WorkflowStepFailed, WorkflowInputMissing, WorkflowTemplateError, WorkflowFileNotFound)
- [x] T005 Re-export workflow module from `crates/mv-core/src/lib.rs`

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: YAML parser, template engine, and CLI restructure that ALL user stories depend on

**CRITICAL**: No user story work can begin until this phase is complete

- [x] T006 Implement YAML parser in `crates/mv-core/src/workflow/parser.rs` — `load_from_file()` and `load_from_str()` functions using serde_yml with strict deserialization, mapping errors to MvError::WorkflowParseError
- [x] T007 Write unit tests for parser in `crates/mv-core/src/workflow/parser.rs` — valid workflow, unknown field rejection, missing required fields, unknown step type, empty file
- [x] T008 Implement template rendering in `crates/mv-core/src/workflow/template.rs` — `render_template()` function using minijinja Environment, accepting a HashMap context of step outputs and workflow inputs
- [x] T009 Write unit tests for template rendering in `crates/mv-core/src/workflow/template.rs` — variable substitution, missing variable error, multiple variables, empty template
- [x] T010 Implement `ExecutionContext` struct in `crates/mv-core/src/workflow/engine.rs` with `inputs` and `outputs` HashMaps and a `to_template_context()` method that merges both (outputs shadow inputs)
- [x] T011 Refactor CLI to use clap subcommands in `crates/mv-cli/src/main.rs` — create `Commands` enum with `Prompt` (default) and `Workflow` variants, move shared flags (verbose, otlp, json) to top-level Cli struct, preserve backward compatibility for `mv-cli "prompt text"` usage
- [x] T012 Update existing CLI integration tests in `crates/mv-cli/tests/cli_args.rs` to work with the refactored subcommand structure

**Checkpoint**: Parser, template engine, and CLI structure ready for user story implementation

---

## Phase 3: User Story 1 — Run a Sequential Multi-Step Workflow (Priority: P1) MVP

**Goal**: Execute a multi-step workflow with prompt steps, sequential output passing, and CLI `workflow run` subcommand

**Independent Test**: Run `mv-cli workflow run workflow.yaml --input topic="Rust async"` with a two-step prompt workflow and verify both steps execute with output passing

### Tests for User Story 1

- [x] T013 [US1] Write unit test for sequential engine execution in `crates/mv-core/src/workflow/engine.rs` — mock model calls, verify steps execute in order, verify output context accumulates; include a 5-step workflow fixture to cover SC-003
- [x] T014 [US1] Write unit test for workflow input validation in `crates/mv-core/src/workflow/engine.rs` — required input missing, enum value invalid, default value applied
- [x] T015 [P] [US1] Write CLI integration test for `workflow run` in `crates/mv-cli/tests/cli_workflow.rs` — valid workflow file, missing file error, missing required input error

### Implementation for User Story 1

- [x] T016 [US1] Implement prompt step execution in `crates/mv-core/src/workflow/engine.rs` — resolve model from registry (step-level or defaults), render template with ExecutionContext, call model via rig-core, store response in context outputs
- [x] T017 [US1] Implement sequential workflow execution loop in `crates/mv-core/src/workflow/engine.rs` — `execute_workflow()` function that iterates steps, dispatches by type, accumulates outputs, returns designated workflow outputs
- [x] T018 [US1] Implement `workflow run` subcommand handler in `crates/mv-cli/src/main.rs` — parse `--input key=value` flags, load workflow file, validate, execute, print outputs (plain text and JSON formats per contracts/cli.md)
- [x] T019 [US1] Implement external template file loading in `crates/mv-core/src/workflow/template.rs` — resolve `template_file` paths relative to workflow file directory, read file contents, render as template; include unit tests for file-not-found and unreadable file errors
- [x] T020 [US1] Implement workflow defaults merging in `crates/mv-core/src/workflow/engine.rs` — step-level model/temperature/max_tokens override workflow-level defaults
- [x] T021 [US1] Create example workflow file at workspace root `workflows/examples/research.yaml` (create `workflows/examples/` directory) per quickstart.md for manual testing

**Checkpoint**: Two-step prompt workflows execute end-to-end from CLI with output passing

---

## Phase 4: User Story 2 — Author and Validate Workflow Files (Priority: P2)

**Goal**: Validate workflow files for structural errors without executing them, via `workflow validate` CLI subcommand

**Independent Test**: Run `mv-cli workflow validate workflow.yaml` on a workflow with a bad reference, verify error reported without execution

### Tests for User Story 2

- [x] T022 [P] [US2] Write unit tests for validation in `crates/mv-core/src/workflow/validate.rs` — duplicate step IDs, unresolvable output reference, circular self-reference, missing template, both template and template_file, empty steps, output references nonexistent step, unknown transform operation
- [x] T023 [P] [US2] Write CLI integration test for `workflow validate` in `crates/mv-cli/tests/cli_workflow.rs` — valid workflow reports success with step/input/output counts, invalid workflow reports errors and exits 1

### Implementation for User Story 2

- [x] T024 [US2] Implement `validate()` function in `crates/mv-core/src/workflow/validate.rs` — check duplicate step IDs, unresolvable template references, circular self-references, missing/both templates on prompt steps, empty steps list, output `from` references, unknown transform operations; return Vec\<ValidationError\>. Note: unknown-field rejection is handled at parse time by serde `deny_unknown_fields` (T006/T007), not in this validation pass.
- [x] T025 [US2] Implement `workflow validate` subcommand handler in `crates/mv-cli/src/main.rs` — load workflow, run validation, print success message with counts or error list, exit 0/1 per contracts/cli.md
- [x] T026 [US2] Wire validation into `workflow run` path in `crates/mv-core/src/workflow/engine.rs` — call `validate()` before execution begins, fail with all validation errors if any found (depends on T017: execute_workflow must exist)

**Checkpoint**: `workflow validate` catches all structural errors; `workflow run` validates before executing

---

## Phase 5: User Story 3 — Use Tool Steps in Workflows (Priority: P3)

**Goal**: Tool steps invoke built-in and MCP tools within workflows, with error handling (skip/fail/retry)

**Independent Test**: Create a workflow with a `tool` step calling `file_list`, followed by a `prompt` step that summarizes the listing

### Tests for User Story 3

- [x] T027 [P] [US3] Write unit test for tool step execution in `crates/mv-core/src/workflow/engine.rs` — mock tool invocation, verify inputs rendered from context, verify output stored
- [x] T028 [P] [US3] Write unit test for tool error handling in `crates/mv-core/src/workflow/engine.rs` — on_error skip (output empty, continue), on_error fail (workflow stops), on_error retry (verify retry count, backoff delay progression, and eventual success/failure)

### Implementation for User Story 3

- [x] T029 [US3] Implement tool step execution in `crates/mv-core/src/workflow/engine.rs` — resolve tool from registry (built-in or MCP), render input values from ExecutionContext, invoke tool, store result in context outputs
- [x] T030 [US3] Implement on_error handling for tool steps in `crates/mv-core/src/workflow/engine.rs` — skip (log warning, empty output, continue), fail (stop workflow), retry (exponential/fixed backoff, max attempts)
- [x] T031 [US3] Create example workflow with tool step at `workflows/examples/tool-example.yaml` per quickstart.md

**Checkpoint**: Tool steps work in workflows with configurable error handling

---

## Phase 6: User Story 4 — Transform Step Outputs (Priority: P4)

**Goal**: Transform steps extract and restructure data between workflow steps

**Independent Test**: Workflow with prompt → transform (extract_json) → prompt chain

### Tests for User Story 4

- [x] T032 [P] [US4] Write unit test for extract_json transform in `crates/mv-core/src/workflow/engine.rs` — valid JSON extraction, invalid JSON error, JSON schema validation pass/fail

### Implementation for User Story 4

- [x] T033 [US4] Implement transform step execution in `crates/mv-core/src/workflow/engine.rs` — dispatch by operation name, render input from ExecutionContext, apply transform, store result
- [x] T034 [US4] Implement `extract_json` transform operation in `crates/mv-core/src/workflow/engine.rs` — parse JSON from input string (handle markdown code fences), optional schema validation using serde_json::Value comparison, store stringified result

**Checkpoint**: Transform steps extract structured data from LLM responses

---

## Phase 7: User Story 5 — Workflow Execution Appears in Telemetry (Priority: P5)

**Goal**: Workflow and step execution instrumented with OpenTelemetry spans

**Independent Test**: Run a workflow with `--otlp`, verify spans in Jaeger

### Tests for User Story 5

- [x] T035 [US5] Write unit test for telemetry span creation in `crates/mv-core/src/workflow/engine.rs` — verify `#[instrument]` attributes produce expected span names and fields

### Implementation for User Story 5

- [x] T036 [US5] Add `#[tracing::instrument]` to `execute_workflow()` in `crates/mv-core/src/workflow/engine.rs` with workflow name, version, step count as span fields
- [x] T037 [US5] Add `#[tracing::instrument]` to each step execution function in `crates/mv-core/src/workflow/engine.rs` with step ID, step type, model (for prompt), tool name (for tool) as span fields
- [x] T038 [US5] Log step completion events with output size and duration in `crates/mv-core/src/workflow/engine.rs`

**Checkpoint**: Workflow traces visible in Jaeger with parent/child span hierarchy

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, cleanup, and validation across all stories

- [x] T039 [P] Update `docs/01-architecture-design.md` with DSL engine module description
- [x] T040 [P] Update `README.md` with workflow CLI usage examples
- [x] T041 Run `quickstart.md` end-to-end validation — execute all examples and verify expected behavior
- [x] T042 Run full pre-commit checks: `cargo fmt --check && cargo clippy --all-targets --all-features -- -D warnings && cargo test`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup — BLOCKS all user stories
- **US1 (Phase 3)**: Depends on Foundational
- **US2 (Phase 4)**: Depends on Foundational; integrates with US1 (validate wired into run)
- **US3 (Phase 5)**: Depends on Foundational + US1 (engine loop exists)
- **US4 (Phase 6)**: Depends on Foundational + US1 (engine loop exists)
- **US5 (Phase 7)**: Depends on US1 (engine functions exist to instrument)
- **Polish (Phase 8)**: Depends on all user stories

### User Story Dependencies

- **US1 (P1)**: After Foundational — no other story dependencies (MVP)
- **US2 (P2)**: After Foundational — validation wired into US1's run path (T026)
- **US3 (P3)**: After US1 — extends engine step dispatch with tool type
- **US4 (P4)**: After US1 — extends engine step dispatch with transform type
- **US5 (P5)**: After US1 — adds instrumentation to existing engine functions
- **US3, US4, US5**: Can proceed in parallel after US1 is complete

### Within Each User Story

- Tests MUST be written and FAIL before implementation (TDD per constitution)
- Types/models before services/logic
- Core implementation before CLI integration
- Story checkpoint verified before next priority

### Parallel Opportunities

- T003 and T004 can run in parallel (different files)
- T008/T009 (template) and T006/T007 (parser) can run in parallel after T003
- T022/T023 and T027/T028 and T032 can run in parallel (test files, different stories)
- After US1, US3/US4/US5 can proceed in parallel
- T039 and T040 can run in parallel (different doc files)

---

## Parallel Example: After Foundational

```text
# Parser and template engine can be built in parallel:
Task T006: Parser in workflow/parser.rs
Task T008: Template engine in workflow/template.rs

# After US1 complete, these stories can start in parallel:
Task T027-T031: US3 (Tool steps)
Task T032-T034: US4 (Transform steps)
Task T035-T038: US5 (Telemetry)
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T005)
2. Complete Phase 2: Foundational (T006–T012) — CRITICAL
3. Complete Phase 3: User Story 1 (T013–T021)
4. **STOP and VALIDATE**: Run the example workflow end-to-end
5. Demo: `mv-cli workflow run workflows/examples/research.yaml --input topic="Rust async"`

### Incremental Delivery

6. Phase 4: Add validation (US2) — users get early error feedback
7. Phase 5: Add tool steps (US3) — workflows interact with the world
8. Phase 6: Add transform steps (US4) — structured data flow
9. Phase 7: Add telemetry (US5) — observability
10. Phase 8: Polish — docs, cleanup, final validation
