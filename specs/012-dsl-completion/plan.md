# Implementation Plan: DSL Completion — Value Context, Loop, Nested Workflows

**Branch**: `012-dsl-completion` (off `fix-default-run`, carrying its three
commits per the supplemental-spec rollup note) | **Spec**: [spec.md](spec.md)  
**Research**: [docs/06-dsl-flow-management.md](../../docs/06-dsl-flow-management.md)
(promised `loop`/`workflow` syntax + the string-truthiness caveat this sprint
retires), fable notes ([02-findings.md](../../docs/fable/02-findings.md) F15 —
"String→Value is the single most breaking change on the roadmap — decide now",
decided in 008; [03-roadmap-recommendations.md](../../docs/fable/03-roadmap-recommendations.md)
§Phase 6 — nested `workflow` "needs cross-file cycle detection and a depth
cap"), and the misclassification evidence in
[supplemental-spec.md](../supplemental-spec.md).

## Technical Context

**Language**: Rust edition 2024, workspace (`mv-core` lib, `mv-cli` bin)  
**Key deps**: unchanged — minijinja (already accepts `serde` values natively,
which is what makes the migration mostly a type change), serde_json, rig-core
0.35  
**New deps**: none expected  
**Testing**: `just ci` hermetic; engine semantics via mock executors;
classification via the wiremock fake proxy; e2e via shipped examples

## Constitution Check

- Spec entry: this plan + spec.md cover all changes, including the pulled-in
  fix tracked in supplemental-spec (Principle I). ✔
- TDD: failing hermetic test first for every behavior (Principle III). ✔
- YAGNI: no typed *input declarations* (CLI inputs stay strings), no
  `loop.iteration` context variable, no `tojson`-style filter work beyond
  pinning the default rendering, no workflow-step output *mapping* DSL (the
  child's whole outputs object is the output). Each is noted, none built. ✔
- Docs are part of done: WS5 in-sprint; docs/06 finally stops promising. ✔

## The stringly surface (measured)

The migration touches exactly three files' types plus their call sites:

- `workflow/template.rs` — `render_template` / `evaluate_condition` take
  `&HashMap<String, String>`; both become `&HashMap<String, Value>` (minijinja
  ingests `Value` via serde — field access and typed comparisons come free).
- `workflow/engine.rs` — `ExecutionContext { inputs, outputs }`,
  `validate_inputs`, `to_template_context`, `render_json_value` vars,
  `build_workflow_outputs`, `WorkflowResult.outputs`.
- `workflow/transform.rs` — `extract_json` parses then `.to_string()`s the
  JSON back into a string (line 71): the exact loss this sprint removes.

`validate.rs` reasons about *names*, not values — untouched by the type
change. Executor traits stay `String`-based.

## Design decisions

1. **Classification order and shapes** (US1). Keep TRT-LLM-502-first, then:
   status-bearing responses (rig's `InvalidStatusCodeWithMessage(NNN, body)`
   shape — parse the status) classify by status: recognized not-found bodies
   → `ModelNotFound` (unchanged), other 4xx → `CompletionFailed` (fail
   fast), 5xx → new `MvError::BackendErrorResponse { endpoint, model,
   status, details }` (`Display`: backend responded, status + body —
   truthful). Transport classification keeps the connection/send/timeout
   keywords but **drops the bare `HttpError` substring** (the bug: every rig
   HTTP-layer error contains it). Genuine transport failures still match via
   "error sending request"/"connect…". 5xx is fallback-eligible AND
   retryable; 4xx is neither.
2. **Values in, values through, one stringify rule out.** Context maps hold
   `serde_json::Value`. CLI `--input` and prompt/tool results enter as
   `Value::String`; `extract_json` and the nested-workflow step introduce
   structure. Templates/conditions get the real values (minijinja handles
   field access + numeric comparison natively — this is why one template
   language was the 008 lesson). Where a value leaves the typed world
   (workflow output printing, text + `--json`), the rule is: strings render
   raw, everything else compact JSON. Whole-container interpolation *inside
   templates* uses minijinja's native rendering — pinned by a unit test and
   documented verbatim in docs/06 rather than assumed.
3. **Loop is do-while with a shared context.** The body executes, then
   `exit_condition` (optional; bare minijinja expression like `branch`)
   evaluates against the full context — so it can reference body outputs,
   and iteration N+1 sees iteration N's outputs (the refine/accumulator
   pattern; same-name overwrite per iteration is the mechanism, not a
   collision — it is the same step re-executing). `max_iterations` required,
   `>= 1`, validated; cap reached = normal continuation, not an error.
   Maybe-defined: the body runs at least once, so the loop contributes the
   body's definitely-defined set. Per-iteration tracing span carries the
   index.
4. **Nested workflow = `execute_workflow` recursion with isolation.** The
   fable note holds: the engine's own entry point is already the sub-call
   contract. The step loads `file` (relative to the parent's directory),
   templates its `inputs` map, and runs the child with ONLY those inputs
   (no parent context leakage — same philosophy as parallel's snapshot
   isolation). The child's declared outputs become one JSON object stored at
   the step's `output` (structure courtesy of decision 2). `Box::pin` for
   the recursive future, as branch arms already do.
5. **Cross-file safety is explicit state, not convention.** Execution
   threads a chain of canonicalized paths: revisit = cycle error naming the
   chain; `MAX_WORKFLOW_DEPTH = 8` (constant, documented). `workflow
   validate` with a known directory loads and validates children
   recursively (the `template_file` precedent) with the same cycle/depth
   guards, so authoring errors surface before any model is invoked.
6. **Back-compat is a tested gate.** The migration lands behind the existing
   suite: every pre-012 workflow/example/test passes with at most mechanical
   expectation updates (e.g. output printing). Anything more than mechanical
   means the rendering rule is wrong — stop and rethink, don't adapt tests.

## Workstream structure (maps to tasks.md phases)

1. **WS1 Truthful backend errors** (pulled-in fix) — status-aware
   classification + `BackendErrorResponse`; taxonomy/test-matrix update;
   fake-proxy 500 e2e incl. chain advance; live-skip heuristic update.
2. **WS2 Value migration** — template.rs/engine.rs/transform.rs type
   migration; rendering rules pinned (interpolation + output printing);
   field-access + numeric-condition e2e (the docs/06 example pattern);
   back-compat sweep.
3. **WS3 `loop` step** — types/parser (`deny_unknown_fields`); validation
   (cap, body, condition, recursive walks, definitely-defined); do-while
   engine with iteration tracing; `workflows/examples/loop-example.yaml` +
   e2e + validate-pin.
4. **WS4 `workflow` step** — types/parser; load/validate child (validate-time
   + runtime), cycle chain + depth cap; isolated-context execution + object
   output; `workflows/examples/subworkflow-example.yaml` (+ child) + e2e +
   validate-pin.
5. **WS5 Docs truth pass** — docs/06 (tags off, Value semantics +
   interpolation rule, loop/cycle/depth, fix the now-real numeric example,
   retire the truthiness caveat); docs/01; README; roadmap 6.2 boxes +
   lessons; supplemental-spec rollup closure at ship.

## Ordering & risk

- WS1 first: small, independent, and everything after it inherits truthful
  errors. WS2 strictly before WS3/WS4 (loop conditions need typed values;
  the workflow step's object output needs `Value`). WS3 before WS4 only by
  convention (both extend the recursive walks; loop is the simpler
  extension). WS5 last.
- Biggest risk: **minijinja's rendering of non-string values** interpolated
  into text (containers especially). Mitigation: pin it with unit tests in
  WS2 *before* building on it; document the observed behavior; if it proves
  unusable for prompts, define the stringify-at-interpolation rule ourselves
  (containers → compact JSON) at the template boundary — a contained change.
- Second risk: **back-compat drift** — subtle behavior changes hiding behind
  green-but-edited tests. Mitigation: decision 6's "mechanical changes only"
  rule; the pre-012 examples run unmodified.
- Third risk: classification changes silently shifting fallback behavior.
  Mitigation: the 009 taxonomy matrix test is extended, not replaced; the
  502/transport paths are pinned byte-for-byte.
- Bookkeeping: at ship, the supplemental-spec change-log rows get the 012 PR
  reference and the rollup note is marked done; both `fix-default-run` and
  `012-dsl-completion` local branches are deleted after merge.
