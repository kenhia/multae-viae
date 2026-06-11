# Tasks: Advanced Routing & DSL Composition

**Input**: Design documents from `/specs/009-routing-composition/`  
**Prerequisites**: plan.md, spec.md

**Tests**: TDD per Principle III — failing hermetic test first. Fallback
failure injection uses the 008 wiremock fake proxy; engine behavior uses mock
executors; live tests stay `#[ignore]`d.

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (FR reference)
```

---

## Phase 1: WS1 — Fallback mechanism

- [X] T001 [WS1] `fallback: Option<Vec<String>>` on `ModelEntry`
  (`crates/mv-core/src/lib.rs`); registry validation rejects unknown ids and
  self-reference at load; tests (FR-001)
- [X] T002 [WS1] `CompletionOutcome { text, model_used }` in
  `crates/mv-cli/src/providers.rs`; `complete()` keeps returning `String`
  (FR-002)
- [X] T003 [WS1] `complete_with_fallback(registry, entry, …) ->
  Result<CompletionOutcome, MvError>`: walk `[entry] + fallback`, advance only
  on `is_fallback_eligible()`, fail fast otherwise; new aggregated
  `MvError::AllModelsFailed { attempts }` enumerating every (model, error)
  pair; wiremock tests — 502 → fallback serves; dead port → fallback serves;
  ineligible error → no second attempt (FR-002, FR-004, SC-002)
- [X] T004 [WS1] Route both call sites through the walker: CLI prompt command
  (`crates/mv-cli/src/commands/prompt.rs`) and `RigPromptExecutor`
  (`crates/mv-cli/src/executors.rs`); hermetic CLI tests (FR-002)
- [X] T005 [P] [WS1] Surface the substitution: stderr notice in text mode,
  `model_used` in `--json` output; CLI tests asserting channel + shape (FR-003)
- [X] T006 [P] [WS1] Telemetry: `router.selected` / `router.reason` span
  attributes + one span event per failed attempt, per docs/07 §Telemetry
  (FR-003)
- [X] T007 [WS1] `--stream` + chain: no mid-stream fallback; primary's
  classified error surfaces (fast via T009 preflight); test + docs note (FR-006).
  *Note: the streaming path already preflights via the existing
  `trtllm_preflight` (`check_health`), which fails fast on connection-refused;
  WS2/T010 swaps that for the shared `mv_core::preflight`. The user-facing docs
  note (the `--stream` limitation) lands with WS6 (T021/T023).*
  *Deviation: the walker advances on an eligible error only when the chain has
  >1 entry. A single model with no fallback returns its real classified error
  (e.g. `is not loaded`) rather than wrapping it in `AllModelsFailed` — this
  keeps every pre-009 single-model error path byte-for-byte unchanged.*

**Checkpoint**: a dead primary falls back hermetically; ineligible errors
never do; `just ci` green.

---

## Phase 2: WS2 — Preflight

- [X] T008 [WS2] `crates/mv-core/src/preflight.rs`:
  `enum PreflightStatus { Healthy, Dead, Unknown }` +
  `preflight(entry, endpoint, timeout)` — trtllm = health + served-model
  presence (delegating to `trtllm::health`), ollama = endpoint reachability,
  openai = `Unknown`; injectable timeout (threaded into `check_health` /
  `served_model_present`); wiremock unit tests (mv-core dev-dep) (FR-005).
  *Deviation 1: `Dead(MvError)` carries the exact error the entry would surface
  (not a bare `reason` string) — this lets both the walker (record verbatim)
  and the trtllm call paths (return verbatim) share one probe without losing
  the `just load` / `trtllm-serve` / "Is Ollama running?" hints. Deviation 2:
  `preflight` takes an explicit `endpoint` so it probes the same resolved
  endpoint the completion uses (honoring a `--endpoint` override), not
  `entry.endpoint()`.*
- [X] T009 [WS2] Chain walker skips `Dead` entries without a completion
  attempt, recording the skip as a span event; preserve provider hints (e.g.
  `just load <id>`) in the final `AllModelsFailed`; tests (FR-005).
  *Note: the skip is guarded on `chain_len > 1` so single-model chains go
  straight to `complete` (their real classified error and pre-009 tests are
  unchanged). The real win is a dead Ollama: `complete` has no internal
  preflight there, so the skip avoids waiting out rig's connect timeout
  (`dead_ollama_primary_preflight_skips_to_backup`).*
- [X] T010 [WS2] Re-point `call_trtllm`/`stream_trtllm` preflight at
  `mv_core::preflight` — single source, `trtllm::health` becomes its
  implementation detail; existing tests keep passing (FR-005). *The streaming
  path's two preflight steps (health, then served-model) collapse into one
  `preflight` call. Both call paths still self-preflight (robust if `complete`
  ever gets another caller); on a healthy trtllm backend in a chain this
  overlaps the walker's probe by one cheap GET.*

**Checkpoint**: dead locals skipped in milliseconds; one preflight seam.

---

## Phase 3: WS3 — `branch` step

- [ ] T011 [WS3] Recursive `Step::Branch { condition, then, else }` in
  `crates/mv-core/src/workflow/types.rs` + parser; arms are non-empty
  `Vec<Step>`; `deny_unknown_fields`; parse/shape tests (FR-007)
- [ ] T012 [WS3] Validation (`crates/mv-core/src/workflow/validate.rs`):
  condition compiles via minijinja `compile_expression`; recursive walk
  extends duplicate-output detection into arms; **maybe-defined analysis** —
  post-branch reference to an output not defined in *every* arm (missing
  `else` = empty arm) is an error; tests incl. nested branches (FR-008,
  SC-004)
- [ ] T013 [WS3] Engine (`crates/mv-core/src/workflow/engine.rs`): evaluate
  condition against the context (minijinja truthiness), recurse into the
  chosen arm via `execute_steps`, skip cleanly when falsy with no `else`;
  tracing span per branch; mock-executor tests (FR-007)
- [ ] T014 [WS3] Example workflow `workflows/examples/branch-example.yaml` +
  e2e CLI test against the fake proxy (FR-011, SC-003)

**Checkpoint**: workflows branch on intermediate results; validator sees
through arms.

---

## Phase 4: WS4 — `parallel` step

- [ ] T015 [WS4] `Step::Parallel { steps }` in types + parser; parse tests
  (FR-009)
- [ ] T016 [WS4] Validation: children render only against pre-fork context —
  referencing a sibling's output is an error; disjoint outputs via the T012
  recursive duplicate-output walk; tests (FR-009, SC-004)
- [ ] T017 [WS4] Engine: fork-join via `futures::future::join_all` (no
  `tokio::spawn`); each child gets `ExecutionContext::snapshot()`; merge
  outputs in declaration order at the join; all children run to completion,
  then aggregate failures into one error naming every failed child;
  **rendezvous test** proving genuine concurrency (two children must be
  in-flight simultaneously to complete) (FR-009, SC-003)
- [ ] T018 [WS4] Example workflow `workflows/examples/parallel-example.yaml` +
  e2e CLI test against the fake proxy (FR-011, SC-003)

**Checkpoint**: independent steps fan out with snapshot isolation.

---

## Phase 5: WS5 — Preference lists

- [ ] T019 [WS5] `model:` on prompt steps (and `defaults.model`) accepts bare
  string or `{prefer: [id, …]}` (untagged enum) in workflow types; parse +
  back-compat tests (FR-010)
- [ ] T020 [WS5] Validation: `prefer` ids checked against the registry,
  failure lists available models; resolution builds the candidate chain and
  hands it to the WS1 walker (no second routing path); hermetic test —
  first-preferred dead → second serves (FR-010)

**Checkpoint**: hybrid routing per docs/07 §3, one mechanism.

---

## Phase 6: WS6 — Docs truth pass (polish)

- [ ] T021 [P] [WS6] docs/06: drop "Not yet implemented" tags for
  `branch`/`parallel`/model-preference; document condition semantics
  (minijinja expression, string-typed until the Value migration), the
  maybe-defined rule, parallel snapshot isolation + aggregated failure;
  refresh the Evolution Path list (FR-012)
- [ ] T022 [P] [WS6] docs/07: mark §3 Hybrid shipped (sprint 009), §2 Adaptive
  deferred to Phase 7; align telemetry section with the shipped `router.*`
  attributes (FR-012)
- [ ] T023 [P] [WS6] docs/01 refresh through sprint 009 (preflight module,
  fallback walker, recursive steps); README: `fallback:` in models.yaml
  example, fallback notice behavior, `--stream` limitation (FR-012)
- [ ] T024 [WS6] Update `docs/09-roadmap.md` Phase 5 checkboxes + lessons
  learned (after merge)

**Checkpoint**: SC-001–SC-005 met; sprint shippable.
