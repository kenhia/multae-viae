# Project Review — Post-Sprint 007

**Date:** 2026-06-11 · **Scope:** sprints 001–007 (Phases 0 → 4.5.1) · **Reviewer:** Claude (Fable 5)

**Method:** five parallel review streams (core library, CLI dispatch seam, workflow engine,
test suite, docs/specs) over the full ~6.2k-line workspace, cross-checked against `just ci`
(passes clean) and hand-verification of every high-severity finding in source.

## Documents

| Doc | Contents |
|-----|----------|
| [01-retrospective.md](01-retrospective.md) | What sprints 001–007 got right; the patterns worth protecting |
| [02-findings.md](02-findings.md) | Consolidated findings registry — 30 items, severity-ranked, with locations and fixes |
| [03-roadmap-recommendations.md](03-roadmap-recommendations.md) | Proposed plan changes: insert Sprint 008 (Consolidation) before Phase 5; amendments to Phases 5–7 |

## Executive summary

**The architecture is sound and the discipline is real.** The executor-trait seam keeps the
workflow engine Rig-free and mock-testable; the `ModelEntry` resolved-method convention is
actually followed; error messages are pinned by tests; the streaming work in 007 shows
genuine failure-mode thinking (the `/v1/models` preflight covering rig's swallowed-502 hole
is the kind of fix most projects never make). 189 tests, zero failures, hermetic by default.

**But the green gate is hiding three classes of problems:**

1. **Declared features that don't work.** Workflow `tool` steps run against a
   `NoopToolExecutor` that errors on every call (the shipped example masks it with
   `on_error: skip`); `temperature`/`max_tokens` are parsed, validated, plumbed through the
   engine — and then silently dropped by the one real `PromptExecutor`. README and docs
   claim both work. No end-to-end workflow test exists, which is exactly why nobody noticed.

2. **Reachable panics and process leaks.** Tool-output truncation byte-slices UTF-8 and will
   panic on any large non-ASCII file; `retry: {max_attempts: 0}` in user YAML hits an
   `unreachable!()`; a timed-out `shell_exec` orphans the child process.

3. **Structure that Phase 5 will collide with.** The provider dispatch is inlined *twice*
   (prompt path and workflow path), the 502 classifier matches on error *strings* and lives
   in the binary crate that `mv-server` can't depend on, and the engine's executor traits
   lack `Send` bounds — so `parallel` steps can't be spawned. Fallback chains, adaptive
   routing, and branch/parallel steps all land on these exact lines.

**Core recommendation: insert a consolidation sprint (008) before Phase 5.** Roughly one
sprint of work — correctness fixes, the `complete()` dispatch extraction, typed error
classification, the engine prep refactor, a wiremock-based fake-backend fixture, and a docs
truth-reconciliation pass. Every item gets dramatically more expensive after Phase 5
multiplies the call sites. Phase 5's scope itself is right; it just needs a load-bearing
floor under it. Details in [03-roadmap-recommendations.md](03-roadmap-recommendations.md).

**One standing rule to adopt now:** *anything `mv-server` will need must live in `mv-core`.*
The system preamble, the error classifier, the provider dispatch decision, and usage
recording have all accreted into `mv-cli/src/main.rs`. Phase 6 imports all of them.
