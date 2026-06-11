# Feature Specification: Consolidation & Hardening

**Feature Branch**: `008-consolidation`
**Created**: 2026-06-11
**Status**: Active
**Input**: Phase 4.6 from docs/09-roadmap.md — make the shipped feature set true,
tested, and load-bearing before Phase 5. Driven by the post-007 project review in
`docs/fable/` (finding IDs F1–F34 reference `docs/fable/02-findings.md`).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Workflow tool steps actually execute (Priority: P1)

A user writes a workflow with a `tool` step (e.g. `file_list` on a directory) and
runs it via `mv-cli workflow run`. The tool genuinely executes and its real output
flows into subsequent steps — instead of today's behavior where every tool step
fails against a placeholder executor (F2) and the shipped example silently feeds an
empty string to the next prompt.

**Acceptance Scenarios**:

1. **Given** a workflow with a `tool` step invoking a built-in tool, **When** the
   user runs it, **Then** the tool executes and its output is available to later
   steps via `{{output_name}}`.
2. **Given** a tool step naming a tool that does not exist, **When** the workflow
   runs, **Then** the step fails with an error naming the unknown tool (honoring
   `on_error`).
3. **Given** a workflow step with `temperature`/`max_tokens` set, **When** a prompt
   step runs, **Then** those values are sent to the provider rather than silently
   dropped (F3).
4. **Given** a workflow step naming a model not present in the registry, **When**
   the workflow runs, **Then** the run fails with `ModelNotInRegistry` listing
   available models — never a silent substitution of the default model (F6).

### User Story 2 - Tools never panic, leak, or lie (Priority: P1)

Built-in and MCP tool output handling is robust: truncation cannot panic on
multi-byte UTF-8 (F1); a timed-out shell command's child process is killed, not
orphaned (F5); a failing shell command always reports its exit status and labels
stdout/stderr so the model can tell success from failure (F8); MCP tool output is
truncated to the same 10,000-char cap as built-ins (F9).

**Acceptance Scenarios**:

1. **Given** tool output where byte 10,000 falls inside a multi-byte character,
   **When** truncation runs, **Then** it returns a valid string (no panic).
2. **Given** a shell command exceeding the timeout, **When** the timeout fires,
   **Then** the child process is killed and an actionable timeout error returned.
3. **Given** a shell command that exits non-zero with output on stderr, **When**
   the tool returns, **Then** the result includes the exit status and labels the
   streams.
4. **Given** an MCP tool returning more than 10,000 characters, **When** the call
   completes, **Then** the output is truncated with the standard notice.

### User Story 3 - Failures are classified, actionable, and machine-readable (Priority: P2)

The agentic loop's turn-limit exhaustion maps to a dedicated, hinted error instead
of rig's raw string (F7); `retry: {max_attempts: 0}` is rejected at validation
instead of panicking the engine (F4); with `--json`, errors go to stderr per the
documented contract (F10).

**Acceptance Scenarios**:

1. **Given** an agent that exhausts its turn limit, **When** the error surfaces,
   **Then** the message names the limit and suggests next steps.
2. **Given** a workflow with `retry: {max_attempts: 0}`, **When** validated,
   **Then** validation fails with an actionable message; the engine never panics
   even if validation is bypassed.
3. **Given** any failure under `--json`, **When** the error prints, **Then** it is
   a JSON object on **stderr** and the exit code is non-zero.

### User Story 4 - Phase-5-ready dispatch and engine (Priority: P2)

Internal restructuring with no behavior change: a single `complete()` dispatch
seam used by both the prompt and workflow paths (F11); typed error classification
in `mv-core` with `is_fallback_eligible()` (F12); a `Provider` enum rejecting
unknown providers at config load (F13); registry validation (F14); engine prep —
extracted step execution, encapsulated context, `Send`-bounded executor traits,
one template language, output-collision validation (F15–F18); a default-allow
`ToolPolicy` seam (F19).

### User Story 5 - The gate can catch what it currently cannot (Priority: P2)

Hermetic tests exercise a successful model call, the multi-turn tool loop, the
streaming path, and a full workflow run via a scripted fake OpenAI/TRT-LLM proxy
(F25, F28) and a fake stdio MCP server (F26); CLI tests cannot reach a developer's
live Ollama (F27).

## Requirements

- **FR-001**: Workflow tool steps MUST execute real built-in/MCP tools via the
  agent tool server (F2).
- **FR-002**: `temperature` and `max_tokens` MUST be forwarded to providers from
  workflow steps (F3); unknown step models MUST fail with `ModelNotInRegistry` (F6).
- **FR-003**: Tool-output truncation MUST be UTF-8-safe (F1); MCP output MUST be
  truncated to `MAX_TOOL_OUTPUT_CHARS` (F9).
- **FR-004**: `shell_exec` MUST kill timed-out children (F5) and MUST always
  surface non-zero exit status, labeling stdout/stderr when both present (F8).
- **FR-005**: Retry config MUST require `max_attempts >= 1` at validation; the
  engine MUST return an error, not panic, for invalid retry config; computed
  backoff MUST be capped (F4).
- **FR-006**: Turn-limit exhaustion MUST map to `MvError::MaxTurnsExceeded` with
  an actionable hint (F7).
- **FR-007**: `--json` errors MUST print to stderr (F10).
- **FR-008**: Provider dispatch MUST converge on one `complete()` function used by
  both CLI prompt and workflow executor paths (F11–F13), with classification and
  the system preamble living in `mv-core` (F12, F20).
- **FR-009**: Engine executor traits MUST carry `Send` bounds; `ExecutionContext`
  fields MUST be private; validation and rendering MUST share one template
  language; duplicate output names MUST be a validation error (F15–F18).
- **FR-010**: All fixes MUST land with hermetic tests (TDD per Principle III);
  live-backend tests stay `#[ignore]`d.

## Success Criteria

- **SC-001**: `workflows/examples/tool-example.yaml` runs end-to-end with real
  tool output (no `on_error: skip` masking).
- **SC-002**: `just ci` hermetically exercises ≥1 successful completion, ≥1
  multi-turn tool round-trip, and ≥1 full workflow run.
- **SC-003**: No `unwrap`/`unreachable!`/byte-slice panic reachable from user
  input or config anywhere in tool/engine paths.
- **SC-004**: Default test suite wall time ≤ 10s (down from ~35s; F27).
- **SC-005**: README/docs claims match behavior (F29–F34 closed).
