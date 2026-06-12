# Tasks: DSL Completion — Value Context, Loop, Nested Workflows

**Input**: Design documents from `/specs/012-dsl-completion/`  
**Prerequisites**: plan.md, spec.md, supplemental-spec.md (pulled-in fix +
branch rollup)

**Tests**: TDD per Principle III — failing hermetic test first. Engine
semantics via mock executors; classification via the wiremock fake proxy;
shipped examples pinned by validate tests.

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (FR reference)
```

---

## Phase 1: WS1 — Truthful backend errors (pulled-in fix)

- [X] T001 [WS1] Status-aware classification in
  `crates/mv-core/src/providers.rs`: parse status-bearing rig errors
  (`InvalidStatusCodeWithMessage`); order = TRT-LLM 502 → not-found shapes →
  4xx `CompletionFailed` → 5xx new variant; drop the bare `HttpError`
  keyword from the unreachable branch; transport keywords unchanged. New
  `MvError::BackendErrorResponse { endpoint, model, status, details }` with a
  truthful `Display`; eligible + retryable. Unit matrix incl. byte-for-byte
  pins of the 502/transport paths (FR-001, FR-002)
- [X] T002 [WS1] Hermetic CLI e2e: fake-proxy 500-with-body → truthful error
  (no "Is Ollama running?"); chained primary-500 → fallback serves
  (`cli_fallback.rs`) (FR-001, FR-002, SC-001)
- [X] T003 [P] [WS1] `cli_klams.rs` live-skip heuristic recognizes the new
  5xx message as "no usable model backend"; 009 lessons entry gets a
  fixed-in-012 pointer (FR-003)

**Checkpoint**: errors truthful; fallback taxonomy intact; `just ci` green.

---

## Phase 2: WS2 — Value context migration

- [X] T004 [WS2] Migrate `workflow/template.rs` + `workflow/engine.rs` +
  `workflow/transform.rs` to `HashMap<String, serde_json::Value>` contexts:
  `render_template`/`evaluate_condition`, `ExecutionContext`,
  `validate_inputs`, `render_json_value`, `build_workflow_outputs`,
  `WorkflowResult.outputs`; CLI inputs and prompt/tool results enter as
  `Value::String`; `extract_json` stores the parsed `Value` (FR-004, FR-006)
- [X] T005 [WS2] Pin the rendering rules with unit tests: string values
  interpolate raw; whole-container interpolation behavior recorded (test is
  the spec); typed comparisons in conditions (`a.b >= 8` numeric;
  `"false"`-truthiness retired for typed values); output printing rule
  (strings raw, non-strings compact JSON) in `commands/workflow.rs` text +
  `--json` (FR-005)
- [X] T006 [WS2] Field-access e2e (the docs/06 pattern, hermetic): prompt →
  `extract_json` → branch on numeric `score` → template `{{out.title}}`
  (SC-002)
- [X] T007 [WS2] Back-compat sweep: every pre-012 workflow test/example
  passes with at most mechanical expectation changes — anything more is a
  design bug, not a test chore (FR-007, SC-005)

**Checkpoint**: structured data flows end-to-end; nothing existing broke.

---

## Phase 3: WS3 — `loop` step

- [ ] T008 [WS3] `Step::Loop { id, name?, max_iterations, exit_condition?,
  steps }` in `workflow/types.rs` + parser (`deny_unknown_fields`,
  `Step::output()` → `None`); parse/shape tests (FR-008)
- [ ] T009 [WS3] Validation: `max_iterations >= 1`; non-empty body;
  condition compiles + references resolve (body outputs visible — condition
  runs post-iteration); recursive id/output/maybe-defined/reference walks
  extend into the body; post-loop definitely-defined = body's; tests incl.
  nested loop-in-branch (FR-008)
- [ ] T010 [WS3] Engine: do-while over the shared context via
  `execute_steps` recursion; cap reached = normal continuation;
  per-iteration tracing span with index; mock-executor tests (early exit
  after exactly N body runs; cap-bound run; condition references body
  output); `workflows/examples/loop-example.yaml` + e2e CLI test + validate
  pin (FR-008, FR-011, SC-003, SC-006)

**Checkpoint**: workflows iterate with typed exit conditions.

---

## Phase 4: WS4 — `workflow` (nested) step

- [ ] T011 [WS4] `Step::SubWorkflow { id, name?, file, inputs?, output }`
  (`type: workflow`) in types + parser; parse tests (FR-009)
- [ ] T012 [WS4] Safety + validation: canonicalized path-chain cycle
  detection (error names the chain) + `MAX_WORKFLOW_DEPTH` cap; `workflow
  validate` recursively loads + validates children when the parent's dir is
  known (missing file → actionable path error); runtime re-validates on
  load; tests incl. self-cycle, two-file cycle, over-deep chain (FR-010)
- [ ] T013 [WS4] Engine: templated `inputs` map → isolated child context
  (declared inputs only) → `execute_workflow` recursion (`Box::pin`) → child
  outputs as one JSON object at `output`; mock-executor tests + e2e
  (`workflows/examples/subworkflow-example.yaml` + child file, downstream
  field access) + validate pin (FR-009, FR-011, SC-004, SC-006)

**Checkpoint**: workflows compose across files, safely.

---

## Phase 5: WS5 — Docs truth pass (polish)

- [ ] T014 [WS5] docs/06: drop "Not yet implemented" for `loop`/`workflow`;
  document Value semantics (field access, typed conditions, interpolation +
  printing rules verbatim from the pinned tests); loop do-while/cap rules;
  cycle/depth rules; fix the previously-stale numeric-condition example;
  retire the string-truthiness caveat; refresh the Evolution Path (FR-012)
- [ ] T015 [P] [WS5] docs/01 sprint-012 entry (Value migration, new steps,
  truthful classification); README workflow section touch-up (FR-012)
- [ ] T016 [P] [WS5] Roadmap Phase 6.2 checkboxes + lessons learned;
  supplemental-spec: change-log rows get the 012 PR ref, rollup note marked
  done (both local branches deleted after merge) (FR-012)

**Checkpoint**: SC-001–SC-006 met; sprint shippable.
