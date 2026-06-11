# Feature Specification: Advanced Routing & DSL Composition

**Feature Branch**: `009-routing-composition`  
**Created**: 2026-06-11  
**Status**: Draft  
**Input**: Phase 5 from docs/09-roadmap.md — resilient model routing (fallback
chains, per-provider preflight, preference lists) and workflow composition
(`branch` / `parallel` step types). Sequencing follows the Phase 5 amendments in
[docs/fable/03](../../docs/fable/03-roadmap-recommendations.md): mechanism before
policy. RAG is split into Phase 5.5 (sprint 010); adaptive scoring is deferred to
Phase 7 alongside meta-routing.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Fallback chains keep prompts working when a backend dies (Priority: P1)

A user lists `fallback: [qwen3:8b, gpt-4o-mini]` on their TRT-LLM model entry.
They run a prompt while the proxy is down (or the model is unloaded). Instead of
an error, the next model in the chain serves the request, and the substitution is
visible — a stderr notice in text mode, a `model_used` field in `--json`, and
span attributes in traces.

**Acceptance Scenarios**:

1. **Given** a chain whose primary backend is unreachable, **When** the user runs
   a prompt, **Then** the next chain entry serves it and the output notes which
   model responded.
2. **Given** a failure that is not fallback-eligible (e.g. empty prompt, turn
   limit exhausted, a completed-but-failed completion), **When** it occurs,
   **Then** no fallback is attempted — the original error surfaces immediately.
3. **Given** a chain where every entry fails, **When** the user runs a prompt,
   **Then** the error lists each model attempted and why it failed.
4. **Given** a `fallback` list naming an unknown model id or the entry's own id,
   **When** the registry loads, **Then** load fails with an actionable message
   (never a runtime surprise).

### User Story 2 - Dead local backends are skipped fast (Priority: P2)

Before building an agent against a model, the router can ask "is this backend
even alive?" via a cheap per-provider preflight. A chain containing a dead local
entry skips it in milliseconds instead of waiting out a connection timeout, and
the skip is recorded in telemetry.

**Acceptance Scenarios**:

1. **Given** a chain entry whose preflight reports `Dead`, **When** the chain is
   walked, **Then** that entry is skipped without a completion attempt and the
   skip is traced.
2. **Given** a provider with no cheap probe (OpenAI cloud), **When** preflighted,
   **Then** the result is `Unknown` and the completion is attempted normally.
3. **Given** a TRT-LLM entry whose proxy is up but model unloaded, **When**
   preflighted, **Then** the result is `Dead` with the existing `just load` hint
   preserved on the final error if no entry succeeds.

### User Story 3 - Workflows branch on intermediate results (Priority: P2)

A workflow author writes a `branch` step whose condition inspects a prior step's
output and runs one of two arm step-lists (`then` / `else`). Validation
understands that an output defined in only one arm may not exist afterward.

**Acceptance Scenarios**:

1. **Given** a branch whose condition evaluates truthy, **When** the workflow
   runs, **Then** only the `then` arm executes and its outputs are available to
   later steps.
2. **Given** a falsy condition with no `else` arm, **When** the workflow runs,
   **Then** the branch is a no-op and execution continues.
3. **Given** a later step referencing an output defined in only one arm,
   **When** the workflow is validated, **Then** validation fails naming the
   maybe-undefined output (both arms must define it for post-branch use).
4. **Given** a condition that is not a valid expression, **When** validated,
   **Then** validation fails before execution.

### User Story 4 - Workflows fan out independent steps (Priority: P2)

A workflow author writes a `parallel` step whose child steps run concurrently.
Each child sees an immutable snapshot of the context taken at the fork; outputs
must be disjoint and merge at the join. No child ever observes a sibling's
output.

**Acceptance Scenarios**:

1. **Given** a parallel step with two independent children, **When** the
   workflow runs, **Then** both execute concurrently and both outputs are
   available after the join.
2. **Given** a child template referencing a sibling's output, **When**
   validated, **Then** validation fails — siblings are invisible to each other
   by construction.
3. **Given** one child failing (after its own `on_error` handling), **When**
   the join completes, **Then** the parallel step fails reporting every failed
   child, not just the first.
4. **Given** two children writing the same output name, **When** validated,
   **Then** the existing duplicate-output validation rejects it.

### User Story 5 - Step-level model preference lists (Priority: P3)

A workflow prompt step accepts `model: {prefer: [a, b]}` in place of a bare id.
The preference list resolves through the same chain mechanism as US1 — no second
routing implementation.

**Acceptance Scenarios**:

1. **Given** a `prefer` list whose first entry's backend is dead, **When** the
   step runs, **Then** the second entry serves it (chain semantics, including
   eligibility rules).
2. **Given** a `prefer` list naming an unknown id, **When** the workflow is
   validated against the registry, **Then** validation fails listing available
   models.
3. **Given** a bare-string `model:`, **Then** behavior is unchanged (back-compat).

## Requirements

- **FR-001**: `ModelEntry` MUST support an optional `fallback: [id, …]` list.
  Registry load MUST reject unknown ids and self-reference. Chains are
  **non-transitive**: only the requested entry's own list is walked.
- **FR-002**: A single `complete_with_fallback()` MUST walk the chain — attempt,
  advance only on `MvError::is_fallback_eligible()` errors, fail fast otherwise —
  and return a `CompletionOutcome` carrying the response text and the model that
  served it. Both the CLI prompt path and the workflow `PromptExecutor` MUST go
  through it.
- **FR-003**: When a fallback occurs, the CLI MUST surface it: stderr notice in
  text mode, `model_used` field in `--json`, `router.*` span attributes and
  per-attempt span events in traces.
- **FR-004**: When every chain entry fails, the error MUST enumerate each
  attempted model with its failure reason.
- **FR-005**: mv-core MUST expose a per-provider
  `preflight(entry) -> Healthy | Dead | Unknown`, decoupled from completion:
  TRT-LLM = existing health + served-model check; Ollama = endpoint
  reachability; OpenAI = `Unknown` (no probe). Chain walking MUST skip `Dead`
  entries without a completion attempt. `trtllm` call paths MUST reuse this
  preflight (single source).
- **FR-006**: `--stream` keeps single-model semantics — no mid-stream fallback.
  Streaming a model whose chain would be needed fails with the primary's
  classified error (preflight makes this fast). Documented limitation.
- **FR-007**: A `branch` step MUST carry `condition` (a minijinja expression
  evaluated against the execution context — same language as templates), a
  `then` arm, and an optional `else` arm (each a non-empty step list). Arms MAY
  nest further branch/parallel steps.
- **FR-008**: Reference validation MUST treat branch-arm outputs with
  maybe-defined semantics: a post-branch reference to an output is valid only if
  every arm (including an implicit empty `else`) defines it.
- **FR-009**: A `parallel` step MUST carry a list of child steps executed
  fork-join: each child renders against an immutable context snapshot taken at
  the fork; outputs merge at the join; sibling outputs are never visible to each
  other (validated, not just documented). All children run to completion; the
  step fails if any child failed, aggregating the failures.
- **FR-010**: Prompt steps MUST accept `model:` as either a bare id (unchanged)
  or `{prefer: [id, …]}` resolved through the FR-002 chain mechanism. Workflow
  validation MUST check `prefer` ids against the registry.
- **FR-011**: All new behavior lands with hermetic tests (TDD per Principle
  III): fallback via the wiremock fake proxy (502 / dead port / refused),
  branch/parallel via mock executors, e2e through the CLI against the fixture.
  Live-backend tests stay `#[ignore]`d.
- **FR-012**: Docs are part of done: docs/06 drops the "not implemented" tags
  for `branch`/`parallel`/model-preference and documents condition semantics and
  the maybe-defined rule; docs/07 records hybrid-shipped/adaptive-deferred;
  docs/01 and README updated.

## Success Criteria

- **SC-001**: With the primary's endpoint dead (hermetic), a chained prompt
  completes via the fallback model; stderr names the substitution; the trace
  records `router.selected` and the failed attempt.
- **SC-002**: A test matrix proves fallback never triggers on ineligible errors
  (empty prompt, max-turns, completion-failed) and always triggers on eligible
  ones (unreachable, not-loaded, not-found, missing key).
- **SC-003**: Example workflows exercising `branch` and `parallel` run
  end-to-end hermetically; an engine test proves parallel children are genuinely
  concurrent (rendezvous, not timing).
- **SC-004**: Validation rejects: unknown/self-referencing `fallback` ids,
  unknown `prefer` ids, maybe-defined post-branch references, sibling-visible
  references, duplicate outputs across arms.
- **SC-005**: `just ci` stays green and hermetic; default suite wall time stays
  ≤ 10s.
