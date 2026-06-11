# Implementation Plan: Consolidation & Hardening

**Branch**: `008-consolidation` | **Spec**: [spec.md](spec.md)
**Research**: the post-007 review at [docs/fable/](../../docs/fable/README.md) serves as
this sprint's research record; [02-findings.md](../../docs/fable/02-findings.md) is the
finding registry (F1–F34) and [03-roadmap-recommendations.md](../../docs/fable/03-roadmap-recommendations.md)
the rationale for sequencing.

## Technical Context

**Language**: Rust edition 2024, workspace (`mv-core` lib, `mv-cli` bin)
**Key deps**: rig-core 0.35 (agents/providers/ToolServer), rmcp via rig feature,
minijinja, serde_yml, tokio
**New dev-deps (WS4)**: `wiremock` (fake OpenAI/TRT-LLM proxy) in mv-cli
**Testing**: `just ci` (fmt + clippy -D warnings + workspace tests), hermetic by
default; live tests `#[ignore]`d behind `just test-trtllm`

## Constitution Check

- Spec entry: this plan + spec.md cover all changes (Principle I). ✔
- TDD: every fix lands with the test that was missing (Principle III). ✔
- No new features: consolidation only; YAGNI respected — no dyn provider
  abstraction, no speculative Phase 6 work (Principle IV). ✔
- Docs are part of done: WS5 is in-sprint, not follow-up (Principle V). ✔

## Workstream structure (maps to tasks.md phases)

1. **WS1 Correctness (F1–F10)** — independent, behavior-visible fixes; each is a
   failing test + minimal fix. No structural change. Ships first so the diff is
   reviewable apart from the refactors.
2. **WS2 Phase-5 seam (F11–F14, F20, F21)** — extract `complete()` in mv-cli;
   move `SYSTEM_PREAMBLE` + error classification to mv-core
   (`mv_core::providers`), classify typed rig errors before string fallback; add
   `Provider` enum + registry validation; `effective_max_turns()` on `ModelEntry`.
   Behavior-preserving except for new load-time config rejections.
3. **WS3 Engine prep (F15–F18)** — split engine.rs (`execute_step`/`execute_steps`,
   `transform.rs`, `retry.rs`); encapsulate `ExecutionContext`; `Send` bounds on
   executor traits; minijinja-driven reference validation (incl. `template_file`);
   duplicate-output/shadowing validation. Record the String→`serde_json::Value`
   context decision in research notes (decision: adopt `Value` in Phase 6 `loop`
   work; WS3 only encapsulates so the migration is non-breaking).
4. **WS4 Test infrastructure (F25–F28)** — wiremock proxy fixture (health, models,
   chat completions incl. 502 body + tool_calls + SSE), fake stdio MCP server
   dev-binary, hermetic CLI test configs, e2e workflow test, de-flake (port-0
   listener trick, injectable timeouts).
5. **WS5 Docs truth pass (F29–F34)** — README quickstart + flag matrix, docs/01
   refresh through sprint 008, docs/06 not-implemented tags, docs/04/05 fixes,
   spec hygiene (T042 in 007, supplemental-spec.md stub).

## Ordering & risk

- WS1 first (user-visible correctness, zero structural risk).
- WS2 before WS3 (the workflow executor delegates to `complete()`; engine
  refactor then touches stable call sites).
- WS4 interleaves: the wiremock fixture lands early in WS4 so WS2's `complete()`
  gets happy-path coverage the moment it exists.
- Biggest risk: rig type-level generics in the `complete()` extraction — mitigated
  by unifying on `AgentBuilder<M: CompletionModel>` helpers (no object safety
  needed); see docs/fable/03 §Phase 5 amendments.
- `Send` bounds on executor traits are a breaking trait change — done while there
  are exactly two implementors (Rig + mocks).
