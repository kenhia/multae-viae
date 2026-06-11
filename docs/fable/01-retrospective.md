# Retrospective: Sprints 001–007

What the first seven sprints got right, and which patterns to actively protect as the
project grows. (Problems live in [02-findings.md](02-findings.md) — this doc is the
keep-doing-this list, which matters just as much for steering.)

## The big structural wins

### 1. The executor-trait seam (sprint 005) is the best decision in the codebase

`execute_workflow()` is generic over `PromptExecutor`/`ToolExecutor`, so the entire engine —
sequencing, templating, defaults merging, skip/fail/retry — is unit-tested with mocks and
has **zero** dependency on Rig or any provider. 21 hermetic engine tests exist because of
this one choice. It is also the template for every abstraction Phase 6 needs (memory,
sessions): *define the trait in mv-core, implement it in the binary.* Repeat this pattern;
don't invent a different one.

### 2. `ModelEntry` resolved-methods convention is actually being followed

`endpoint()`, `model_name()`, `locality()`, `effective_stop_sequences()` — provider
differences resolve in one place instead of leaking as match arms into call sites, and
sprint 007 extended the pattern correctly (`stop.rs` owns the defaults,
`request_stop_value()` owns the wire shape). Most projects state a convention like this in
their docs and abandon it by the third feature. This one held for seven sprints. The
findings registry asks for more of it (`effective_max_turns()`, `api_key_env()`), which is
a compliment: the pattern is worth extending.

### 3. Failure-mode engineering in 007 was genuinely senior work

Three examples worth calling out:

- The streaming path does a `/v1/models` preflight **specifically because** rig's SSE layer
  swallows proxy 502s — and `served_model_present()` returns `Option<bool>` with an explicit
  "don't block streaming on a flaky preflight" contract, documented in-line with *why*.
- The 502 classifier ordering hazard (Triton's "...is not found" body shadowing the
  `ModelNotLoaded` mapping) was found live, fixed, **and pinned with a regression test using
  the verbatim live-proxy payload**. That error string can never silently regress.
- The `--stream` + tools impossibility (proxy streams tool calls as text) was resolved by a
  principled degrade-to-buffered design rather than a half-working stream — correctness over
  demo appeal.

The "Lessons Learned" sections in [docs/09-roadmap.md](../09-roadmap.md) capturing these are
exactly the constitution's "document decisions" principle working as intended.

### 4. Errors as a user-facing product surface

`MvError` Display strings are actionable (hints included), asserted verbatim in tests for
~9 variants, and treated as a contract. The `ModelNotLoaded { hint: "Run: just load <id>" }`
flow — proxy 502 → classified error → actionable next command — is a complete UX loop most
CLI tools never close.

### 5. Spec/process discipline is nearly perfect

All seven sprints have complete spec/plan/tasks/research/data-model/quickstart sets;
sprints 001–006 have **zero** unchecked tasks; 007 has exactly one (T042, which is actually
done — just uncheck-box drift). The constitution's gates (`just ci` clean, TDD, hermetic
default suite with `#[ignore]`d live tests) are real, enforced, and passing. The
`just test-trtllm` opt-in pattern for live-backend tests is the right shape and should be
reused for Ollama-dependent tests too.

## The honest scorecard

| Dimension | Grade | One-line justification |
|---|---|---|
| Architecture & seams | **A−** | Trait seam + ModelEntry pattern excellent; provider dispatch duplication and classifier placement cost the plus |
| Code correctness | **B** | Green suite, but a reachable panic, an orphaned-process leak, and a YAML-triggered `unreachable!()` slipped through |
| Feature truthfulness | **C+** | Workflow tool steps and sampling params are declared, documented, and non-functional — the single biggest gap |
| Test suite | **B+** | 189 hermetic-by-default tests with real negative-path coverage; but no happy-path provider test and no e2e workflow test exist offline |
| Docs | **B−** | docs/11 and the roadmap are excellent and current; docs/01 is three sprints stale and README's quickstart fails as written |
| Process & specs | **A** | Seven complete spec sets, enforced gate, recorded lessons |

## Patterns to protect going forward

1. **Trait-in-core, impl-in-binary** — reuse for memory/sessions in Phase 6.
2. **Resolved methods on `ModelEntry`** — never a provider `match` at a call site.
3. **Pinned error-string regression tests** — extend to every new classifier branch.
4. **`#[ignore]` + `just test-<backend>` opt-in** for anything needing a live service.
5. **Lessons-learned blocks in the roadmap** after each sprint — these are already paying
   for themselves (the 007 entries directly informed this review's Phase 5 analysis).
