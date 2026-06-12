# Feature Specification: DSL Completion — Value Context, Loop, Nested Workflows

**Feature Branch**: `012-dsl-completion`  
**Created**: 2026-06-12  
**Status**: Draft  
**Input**: Phase 6.2 from docs/09-roadmap.md — the `String → serde_json::Value`
context migration (decided in 008, deliberately staged before `mv-server`
multiplies consumers; fable risk #3), then the step types that need it:
`loop` and nested `workflow`. **Pulled-in fixes** per
[specs/supplemental-spec.md](../supplemental-spec.md): this branch carries the
three `fix-default-run` commits forward (default-run pin, default model), and
the sprint fixes the **backend-error misclassification** deferred there. Both
branches are deleted after the 012 PR merges (rollup note in the supplemental
spec).

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Backend errors tell the truth (Priority: P1, pulled-in fix)

A model backend that *responds* with an HTTP error status was reached — yet
m-v reports "Cannot reach model backend… Is Ollama running?" (observed live:
Ollama 500 `llama runner process has terminated`, classified
`BackendUnreachable` because the rig error string contains `HttpError`). The
user must see the real status and body, and the fallback chain must still
treat a per-model 5xx as a reason to advance.

**Acceptance Scenarios**:

1. **Given** a backend answering 500 with an error body (hermetic via the
   fake proxy), **When** a prompt runs, **Then** the error names the status,
   endpoint, and model and carries the body — and does NOT say
   "Cannot reach"/"Is Ollama running?".
2. **Given** a genuine transport failure (connection refused / dead port),
   **Then** `BackendUnreachable` with its hint is unchanged.
3. **Given** a TRT-LLM 502 with the Triton body, **Then** the existing
   `ModelNotLoaded` + `just load` hint mapping is unchanged (checked first).
4. **Given** a chain whose primary returns 500, **When** the chain walks,
   **Then** it advances to the fallback (5xx responses are
   fallback-eligible) and the per-attempt record carries the truthful error.
5. **Given** a 4xx response that is not a recognized model-not-found shape,
   **Then** it classifies as a non-eligible failure (fail fast — a bad
   request will not improve on another model).
6. The live-test skip heuristic (`cli_klams.rs`) recognizes the new 5xx
   message as "no usable model backend" and still skips rather than fails.

### User Story 2 - Structured data flows through workflows (Priority: P1)

A workflow author extracts JSON from a model response and then *uses its
fields*: `{{report.title}}` in a template, `score >= 8` in a branch
condition. Today `extract_json` validates JSON and then flattens it back to a
string, and every condition compares strings (the documented `"false"` is
truthy trap). The execution context becomes `serde_json::Value`.

**Acceptance Scenarios**:

1. **Given** a transform step whose `extract_json` output is
   `{"title": "X", "score": 9}`, **When** a later template references
   `{{out.title}}`, **Then** it renders `X`; **and** a branch condition
   `out.score >= 8` evaluates numerically true.
2. **Given** a string output interpolated into a template, **Then** it
   renders raw (no quotes) — existing workflows behave identically.
3. **Given** a non-string value interpolated whole into template text,
   **Then** the rendering is deterministic and pinned by a unit test (JSON
   for containers), and documented in docs/06.
4. **Given** `workflow run` completing with structured outputs, **Then**
   text-mode and `--json` printing follow one documented rule: strings print
   raw, non-strings as JSON.
5. **Given** every pre-012 example and test workflow, **Then** all pass
   unchanged (back-compat is a gate, not a goal).

### User Story 3 - Workflows iterate (`loop`) (Priority: P2)

A refine loop: evaluate a draft, improve it, repeat until a typed
`exit_condition` (e.g. `evaluation.score >= 8`) or `max_iterations`. The body
shares the workflow context so each iteration sees the previous one's
outputs — the accumulator pattern.

**Acceptance Scenarios**:

1. **Given** a loop whose condition becomes true on iteration 2 of 5,
   **Then** the body ran exactly twice and post-loop steps see the final
   iteration's outputs.
2. **Given** a condition that never becomes true, **Then** the body runs
   exactly `max_iterations` times and the workflow continues (cap, not
   error).
3. **Given** no `exit_condition`, **Then** the body runs exactly
   `max_iterations` times.
4. **Given** `max_iterations: 0`, an empty body, or a malformed/unresolvable
   condition, **Then** validation rejects before execution. The condition
   may reference body outputs (it is evaluated after each iteration).
5. **Given** a post-loop reference to a body output, **Then** validation
   accepts (the body executes at least once, so its definitely-defined set
   propagates).

### User Story 4 - Workflows compose (`workflow` step) (Priority: P2)

A parent workflow runs another workflow file as a step, passing templated
inputs and receiving the child's outputs as one structured object output —
field-accessible thanks to US2.

**Acceptance Scenarios**:

1. **Given** a parent with `type: workflow, file: child.yaml` (relative to
   the parent's directory), **When** it runs, **Then** the child executes
   with the templated inputs and the step output is an object of the child's
   outputs (`{{sub.answer}}` works downstream).
2. **Given** a child that references the parent (directly or via a chain),
   **Then** execution fails with a cycle error naming the file chain.
3. **Given** nesting deeper than the documented depth cap, **Then**
   execution fails with an actionable depth error.
4. **Given** `workflow validate` on a parent (directory known), **Then** the
   child file is loaded and validated too — a broken child fails the
   parent's validation, like `template_file` does today.
5. **Given** a missing child file, **Then** the error names the resolved
   path.
6. The child sees ONLY its declared inputs (no parent context leakage).

## Requirements

- **FR-001**: `classify_backend_error` MUST distinguish a **status-bearing
  response** from a **transport failure**. Status-bearing: TRT-LLM 502 →
  `ModelNotLoaded` (unchanged, first); recognized not-found shapes →
  `ModelNotFound` (unchanged); other **5xx** → a new status-carrying variant
  (endpoint, model, status, details) that `Display`s truthfully; other
  **4xx** → `CompletionFailed`. Transport keywords alone (connect / error
  sending request / timeout) → `BackendUnreachable`. The bare `HttpError`
  substring MUST no longer imply unreachable.
- **FR-002**: The new 5xx variant is `is_fallback_eligible() == true` (a
  crashed per-model runner is a sound reason to advance the chain) and
  retryable; the taxonomy test matrix is updated and the 009 lessons-learned
  entry about 500→`BackendUnreachable` gets a pointer to the fix.
- **FR-003**: The `cli_klams.rs` live-test skip heuristic treats the new 5xx
  message as "no usable model backend" (skip, not fail).
- **FR-004**: `ExecutionContext` (inputs, outputs, template context,
  `WorkflowResult.outputs`) migrates `String → serde_json::Value`. CLI
  `--input` values enter as strings; `PromptExecutor`/`ToolExecutor` trait
  signatures stay `String`-based (the engine wraps tool/prompt results as
  `Value::String`; the 10k truncation is untouched). Typed *input
  declarations* are out of scope.
- **FR-005**: Templates and conditions receive real values: field access
  (`{{a.b}}`, `a.b >= 8`) works; string values interpolate raw; whole-container
  interpolation is deterministic, pinned by tests, and documented. Workflow
  output printing (text and `--json`) follows the strings-raw /
  non-strings-JSON rule.
- **FR-006**: `extract_json` stores the parsed `Value` (no re-stringify);
  schema checking behavior is unchanged.
- **FR-007**: Back-compat gate: all pre-012 workflows, examples, and tests
  pass with at most mechanical test-expectation changes; the docs/06
  string-truthiness caveat is resolved (typed comparisons) rather than
  re-documented.
- **FR-008**: `Step::Loop { id, name?, max_iterations, exit_condition?,
  steps }`: do-while semantics (body executes, then the condition is
  evaluated against the full context; truthy → exit). `max_iterations >= 1`
  validated; body non-empty; condition compiles and its references resolve
  (body outputs allowed); the recursive walks (ids, outputs, maybe-defined,
  reference validation) extend into the body; post-loop definitely-defined =
  the body's (body runs ≥ 1). Per-iteration tracing carries the iteration
  index.
- **FR-009**: `Step::SubWorkflow { id, name?, file, inputs?, output }`
  (`type: workflow`): `file` resolves relative to the parent workflow's
  directory; templated `inputs` map; the child executes through the same
  executors via `execute_workflow` recursion with an **isolated context**
  (declared inputs only); the step output is a JSON object of the child's
  declared outputs.
- **FR-010**: Cross-file safety: cycle detection over the canonicalized
  file-path chain (self-reference and longer cycles rejected, error names
  the chain) and a documented nesting depth cap. `workflow validate`
  validates child files when the parent's directory is known; runtime
  re-validates on load.
- **FR-011**: All new behavior lands with hermetic tests (mock executors for
  engine semantics; fake proxy for classification; e2e CLI runs for loop +
  nested workflow examples; shipped examples pinned by validate tests, like
  branch/parallel).
- **FR-012**: Docs are part of done: docs/06 drops the "Not yet implemented"
  tags for `loop`/`workflow`, documents Value semantics + interpolation rules
  + loop/cycle/depth rules, and fixes the previously-stale numeric-condition
  example (now real); docs/01 sprint entry; roadmap 6.2 checkboxes + lessons;
  supplemental-spec rollup note closed out at ship.

## Success Criteria

- **SC-001**: Hermetic: a 500-with-body from the fake proxy produces the
  truthful status error (no "Is Ollama running?"), and a chained model still
  falls back past it; transport-failure and TRT-LLM-502 classifications are
  byte-for-byte unchanged.
- **SC-002**: Hermetic e2e: extract JSON → branch on `score >= 8` (numeric)
  → template field access — the docs/06 example pattern runs for real.
- **SC-003**: Hermetic e2e: a refine `loop` exits early on a typed condition
  (body ran exactly N times, proven by the mock/proxy call count) and a
  capped loop runs exactly `max_iterations`.
- **SC-004**: Hermetic e2e: a parent workflow runs a child file; the child's
  outputs are field-accessible downstream; a cyclic pair and an over-deep
  chain both fail with actionable errors; `workflow validate` catches a
  broken child.
- **SC-005**: Every pre-012 workflow test passes; `just ci` stays green,
  hermetic, ≤ 10s.
- **SC-006**: Shipped examples (`loop-example.yaml`,
  `subworkflow-example.yaml` + child) validate clean via pinned tests.
