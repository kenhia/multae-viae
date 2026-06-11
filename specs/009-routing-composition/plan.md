# Implementation Plan: Advanced Routing & DSL Composition

**Branch**: `009-routing-composition` | **Spec**: [spec.md](spec.md)
**Research**: [docs/07-model-routing.md](../../docs/07-model-routing.md) (routing
strategies; this sprint ships §3 Hybrid, defers §2 Adaptive),
[docs/06-dsl-flow-management.md](../../docs/06-dsl-flow-management.md) (promised
`branch`/`parallel` syntax), and the Phase 5 amendments in
[docs/fable/03](../../docs/fable/03-roadmap-recommendations.md) §Phase 5.

## Technical Context

**Language**: Rust edition 2024, workspace (`mv-core` lib, `mv-cli` bin)  
**Key deps**: rig-core 0.35, rmcp via rig feature, minijinja (templates *and*
branch conditions via `compile_expression`), serde_yml, tokio, futures  
**New dev-deps**: `wiremock` in mv-core (preflight unit tests; already a
mv-cli dev-dep from 008)  
**Testing**: `just ci` hermetic; fallback failure injection via the 008
wiremock fake-proxy fixture; live tests stay `#[ignore]`d

## Constitution Check

- Spec entry: this plan + spec.md cover all changes (Principle I). ✔
- TDD: every behavior lands with a hermetic test first (Principle III). ✔
- YAGNI: non-transitive chains; no adaptive scoring, no cost tracking, no
  capability metadata — those are Phase 7 policies layered on this mechanism.
  `CompletionOutcome` gains exactly the field that 008/T014 deferred it for
  (`model_used`). ✔
- Docs are part of done: WS6 is in-sprint (Principle V). ✔

## What 008 already bought us

- `complete()` is the single dispatch seam (F11) — `complete_with_fallback`
  wraps it; no new dispatch surface.
- `MvError::is_fallback_eligible()` exists with a tested taxonomy (F12).
- `Send`-bounded executor traits + `ExecutionContext::snapshot()` (F15) —
  `parallel` is an additive change.
- `execute_step`/`execute_steps` extraction (F15) — `branch` arms recurse
  through `execute_steps`.
- Duplicate-output validation (F17) — parallel-arm disjointness falls out of
  extending the same walk to nested steps.
- Wiremock fake proxy (F25) — the failure-injection harness fallback tests
  need.

## Design decisions

1. **Chains are non-transitive.** `complete_with_fallback` walks
   `[entry] + entry.fallback`; it does not follow a fallback's own `fallback`
   list. Cycle detection reduces to rejecting self-reference at load. Revisit
   only if a real config needs depth — YAGNI.
2. **`CompletionOutcome { text, model_used }` now.** 008/T014 deliberately
   returned bare `String` until the router needed a second field; this is that
   moment. `complete()` keeps returning `String`; the wrapper is produced by
   `complete_with_fallback`.
3. **Preflight is advisory, not authoritative.** `Dead` skips the entry;
   `Unknown`/`Healthy` proceed to a real attempt (a healthy `/health` can still
   502 on completion). Eligibility on the *attempt* error remains the decision
   point. Statuses: `Healthy | Dead { reason } | Unknown`. Lives in
   `mv_core::preflight`, injectable timeout (de-flake lesson from 008);
   `trtllm::health` becomes its TRT-LLM implementation detail.
4. **No mid-stream fallback.** Tokens already shown to the user can't be
   unshown. `--stream` keeps single-model behavior; preflight makes the failure
   fast and hinted. Documented in docs/06/README.
5. **Branch conditions are minijinja expressions** (`compile_expression`,
   truthiness per minijinja), evaluated against the same context the templates
   see. One template language was an 008 lesson — the validator checks
   conditions with the same engine that evaluates them. With today's
   all-`String` context, comparisons are string-typed; numeric conditions
   arrive with the `Value` migration (decided in 008, scheduled with Phase 6
   `loop`). Documented honestly in docs/06.
6. **Parallel children = single steps, concurrent via `join_all`** on the
   current task (model calls and tools are IO-bound; no `tokio::spawn`, so no
   `'static` gymnastics on executor borrows). Each child renders from a
   `snapshot()`; results buffer and merge at the join in declaration order
   (deterministic). All children run to completion — first-error abort would
   make partial side effects scheduling-dependent; instead aggregate failures
   into one error naming every failed child. Arms-as-sequences (a list of
   lists) is deferred until a workflow needs it; docs/06's promised syntax is a
   flat step list.
7. **`prefer` lists reuse the chain mechanism.** `model:` on prompt steps
   becomes an untagged enum (bare string | `{prefer: [...]}`); resolution
   builds the candidate list and hands it to the same walker. The CLI `-m`
   flag stays a bare id.
8. **Step nesting makes `Step` recursive** (`Vec<Step>` arms). Parser,
   validator (reference walk, duplicate-output walk, maybe-defined analysis),
   and engine all recurse through the shared `execute_steps`/walk helpers —
   no inline match arms (the F15 lesson).

## Workstream structure (maps to tasks.md phases)

1. **WS1 Fallback mechanism** — `fallback` field + registry validation;
   `CompletionOutcome`; `complete_with_fallback()`; CLI surfacing (stderr
   notice, `--json` `model_used`); `AllModelsFailed`-style aggregated error;
   telemetry (`router.selected`, per-attempt events).
2. **WS2 Preflight** — `mv_core::preflight` (per-provider, injectable
   timeout); chain walker skips `Dead`; TRT-LLM call paths re-pointed at the
   shared preflight.
3. **WS3 `branch` step** — recursive types/parser; condition compilation +
   maybe-defined validation; engine recursion; example workflow + e2e test.
4. **WS4 `parallel` step** — types/parser; nested validation walk
   (sibling-invisibility, disjoint outputs); fork-join engine with snapshot
   isolation + aggregated failure; rendezvous concurrency test; example +
   e2e test.
5. **WS5 Preference lists** — `model:` string-or-object on prompt steps;
   registry-checked validation; resolution through the WS1 walker.
6. **WS6 Docs truth pass** — docs/06 (tags off, condition semantics,
   maybe-defined rule, parallel isolation), docs/07 (hybrid shipped, adaptive
   → Phase 7), docs/01, README, roadmap checkboxes at merge.

## Ordering & risk

- WS1 before WS2: fallback is correct without preflight (attempt-fail-advance);
  preflight then folds in as a skip optimization. Both before WS5, which is
  pure reuse.
- WS3 before WS4: branch introduces the recursive `Step` shape and the nested
  validation walk; parallel extends both.
- Biggest risk: **maybe-defined analysis** complicating the validator — keep it
  a set-algebra pass (defined-in-all-arms = defined; defined-in-some = maybe;
  reference to maybe = error) over the recursive walk, nothing flow-sensitive.
- Second risk: minijinja expression truthiness on string-typed values
  surprising authors (`"false"` is truthy as a non-empty string). Mitigate with
  docs + a validation warning for comparisons against non-literal values if it
  proves confusing; do not invent a second expression language.
- `futures::future::join_all` keeps executor borrows simple; if a future needs
  real CPU parallelism that's a Phase 6+ concern.
