# Roadmap Recommendations

Finding IDs (F1–F34) reference [02-findings.md](02-findings.md).

## The core call: insert Sprint 008 — "Consolidation" — before Phase 5

Phase 5 as written (adaptive routing, fallback chains, RAG, branch/parallel steps) is the
**right scope** — nothing in this review argues for cutting or reordering its goals. But
four of its work items land directly on code that is currently duplicated, string-matched,
or non-`Send`:

| Phase 5 item | Lands on | Today's state |
|---|---|---|
| Fallback chains ("try A, on failure B") | a callable dispatch function + machine-readable failure classes | dispatch inlined twice (F11); classifier matches error *strings* in the binary crate (F12) |
| Adaptive routing | registry selection API + validated entries + preflight | `get(id)` only, duplicate ids silently resolve (F14); preflight fused into `call_trtllm` |
| `parallel` step | `Send`-bounded executor traits + collision-validated outputs | no `Send` bounds — `tokio::spawn` won't compile (F15); collisions silently overwrite (F17) |
| `branch` step | recursive step execution + one template language | inline match arm (F15); validator and renderer disagree on the language (F16) |

Building Phase 5 on this floor means writing it twice. The consolidation sprint is
~1–2 weeks and every item in it gets strictly more expensive after Phase 5 multiplies the
call sites.

### Proposed `specs/008-consolidation/` scope

**Workstream 1 — Correctness (the green-gate liars):** F1 UTF-8 truncation panic ·
F2 real `ToolExecutor` (un-Noop workflow tool steps) · F3 wire `temperature`/`max_tokens` ·
F4 retry-config panic · F5 orphaned shell child · F6 silent default-model substitution ·
F7 MaxTurns classification · F8 shell exit-status visibility · F9 MCP output truncation ·
F10 `--json` error channel. Each is small; together they make the declared feature set true.

**Workstream 2 — The Phase 5 seam:**
- Extract `complete(entry, params, handle) -> Result<CompletionOutcome, MvError>` —
  generic helpers over `AgentBuilder<M: CompletionModel>`, both dispatch sites call it (F11).
  No dyn abstraction: fallback operates at the *Result* level, and Rig's generics never
  escape the function. (When Phase 6 sessions need a held-open agent, use a closed
  `enum AnyAgent` over the three concrete agent types — not `Box<dyn>`. Not now; YAGNI.)
- Move classifier + `SYSTEM_PREAMBLE` to mv-core; classify typed rig errors first; add
  `MvError::is_fallback_eligible()` (F12, F20).
- `Provider` enum + registry validation (F13, F14). Split lib.rs into `config.rs`/`error.rs`
  with re-exports while touching it (it's ~280 code lines — not urgent, but this is the
  moment).

**Workstream 3 — Engine prep (behavior-preserving):** extract `execute_step`/
`execute_steps`; move transforms to `transform.rs` (consolidating the op list currently
defined in two files); encapsulate `ExecutionContext`; add `Send` bounds to both traits;
one template language via minijinja's own `undeclared_variables()`; collision validation
(F15–F18). **Decide the `String` → `serde_json::Value` context question here** even though
implementation can wait for Phase 6 `loop` — it's the most breaking change on the roadmap
and every Phase 5 step type written before the decision is churn.

**Workstream 4 — Test infrastructure:** wiremock fake-proxy fixture (F25 — also the
failure-injection harness Phase 5's fallback tests require); fake stdio MCP server (F26);
hermetically pin CLI tests + de-flake (F27 — recovers ~30s of the 35s suite); one e2e
workflow test (F28, the test that would have caught F2/F3).

**Workstream 5 — Docs truth reconciliation (polish phase, per the constitution):**
F29–F34. Headliners: README quickstart model, docs/01 full refresh, docs/06
not-implemented tags, T042 checkbox, supplemental-spec.md stub.

**Explicitly out of scope for 008:** any new feature. This sprint makes the existing
feature set true, tested, and load-bearing.

## Phase 5 amendments (scope unchanged, sequencing sharpened)

1. **Fallback before adaptive.** Ship `complete_with_fallback(chain, …)` — iterate entries,
   recurse on `is_fallback_eligible()` errors, fail fast otherwise — driven by a
   `fallback: [id, …]` field on `ModelEntry` (the established resolved-method pattern).
   Adaptive scoring is a *policy* layered on the same mechanism later; don't co-design them.
2. **Generalize preflight.** Promote `trtllm::health` to a per-provider
   `preflight(entry) -> Healthy | Dead | Unknown` in mv-core, decoupled from completion —
   the router needs to skip dead locals before burning an agent build, and Ollama/OpenAI
   currently have no preflight at all. `locality()` is already the local-first ordering key.
3. **Parallel = fork-join with snapshot isolation.** Each arm gets an immutable context
   snapshot; arms write disjoint output names (validated at parse time — F17 is the
   prerequisite); outputs merge at the join. Never a shared `Arc<Mutex>` context — that
   makes sibling visibility scheduling-dependent and destroys the validator's linear
   reasoning.
4. **Branch validation needs maybe-defined semantics.** Under strict-undefined minijinja,
   an output defined in only one arm is a runtime error the validator currently can't see.
   Require both arms to define a referenced output, or default it.
5. **RAG (Qdrant, embeddings, ingestion) is untouched by this review** — genuinely new
   surface, no findings constrain it. Consider making RAG its own sprint separate from
   routing+DSL; Phase 5 is currently the heaviest phase on the roadmap and splits cleanly.

## Phase 6 pre-work to note in its plan now

- **The "sink to mv-core" rule** (F20) is the whole Phase 6 de-risk: preamble, classifier,
  dispatch, usage recording all get imported by `mv-server`. If 008's Workstream 2 lands,
  Phase 6 starts from a library, not a refactor.
- Shared `reqwest::Client` injection + `tokio::fs` in tool paths (F23).
- `MvError::code()` for machine-readable API errors (`"MODEL_NOT_LOADED"`) — add while the
  variant set is small; a server API and `--json` both want it (pairs with F10's exit-code
  question).
- MCP needs a *connection manager* (keep-alive, reconnect-with-backoff, per-server restart)
  — the current connect-per-invocation `Vec` is correct for a CLI and a throwaway for a
  daemon (F22). Plan the manager; don't grow the module.
- Persistent memory should repeat the `PromptExecutor` pattern: trait in mv-core, impl in
  the binary.
- Nested `workflow` step is the cleanest fit of the four new step types
  (`execute_workflow`'s signature is already the sub-call contract) — needs cross-file
  cycle detection and a depth cap.

## Phase 7 — one pull-forward

Pull the **`ToolPolicy` seam** (F19) into 008 as a default-allow no-op threaded through
tool construction. It's an hour of work now; it converts Phase 7's "tool sandboxing" from a
four-tool signature rewrite (plus Rig re-registrations) into writing a policy
implementation. Everything else in Phase 7 stays put. Note the existing asymmetry while you
build the policy: the example MCP filesystem server is scoped to `/tmp`, the built-ins are
unscoped.

## Risk register (top 3, unchanged by any of the above)

1. **Rig version coupling.** The classifier pins rig 0.35 error *strings*; rig upgrades
   keep unit tests green while breaking live classification (only `#[ignore]`d tests would
   notice). Typed-error classification (F12) + the wiremock fixture (F25) are the
   mitigations — both in 008.
2. **The proxy contract is informal.** Streaming tool-calls-as-text, the 502 body shape,
   and approximate usage all come from trt-llm-explore's current behavior with no version
   pin or contract test. The wiremock fixture doubles as the recorded contract; consider
   noting the proxy commit/tag the fixtures were captured against.
3. **Workflow values are stringly.** `HashMap<String, String>` context survives Phase 5 but
   not Phase 6 (`loop` accumulation, structured tool results, RAG payloads). Deciding the
   `Value` migration in 008 (Workstream 3) caps this; deferring the *decision* — not just
   the work — lets Phase 5 widen the blast radius.

## Suggested roadmap edit

In [docs/09-roadmap.md](../09-roadmap.md), insert between Phase 4.5.1 and Phase 5:

```markdown
## Phase 4.6: Consolidation (1–2 weeks)

**Goal**: Make the shipped feature set true, tested, and load-bearing before Phase 5
builds on it. No new features.

See docs/fable/ (post-007 review) for the findings registry driving this phase.

### Tasks
- [ ] Correctness: F1–F10 (UTF-8 truncation panic, real workflow ToolExecutor,
      sampling params, retry panic, shell child leak, silent model fallback, …)
- [ ] Phase-5 seam: extract `complete()`, typed error classification in mv-core,
      `Provider` enum, registry validation (F11–F14)
- [ ] Engine prep: step-fn extraction, context encapsulation, Send bounds,
      one template language, collision validation; decide String→Value (F15–F18)
- [ ] ToolPolicy seam, default-allow (F19)
- [ ] Test infra: wiremock proxy fixture, fake MCP server, hermetic CLI tests,
      e2e workflow test (F25–F28)
- [ ] Docs truth pass: F29–F34
```

…and in Phase 5, split RAG into its own sub-phase if scheduling pressure appears.
