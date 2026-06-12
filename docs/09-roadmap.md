# Implementation Roadmap

## Phase 0: Foundation (Weeks 1-2)

**Goal**: Working Rust workspace that can call a local model and print a response.

### Tasks
- [x] Initialize Cargo workspace with crate structure
- [x] Set up `mv-core` with core types and error handling
- [x] Add Rig dependency, configure Ollama provider
- [x] Write a simple CLI that sends a prompt to Ollama via Rig
- [x] Set up basic `tracing` with console output
- [x] Establish CI (cargo check, clippy, test)

### Deliverable
```bash
$ cargo run -p mv-cli -- "What is Rust?"
# → Response from local Ollama model
```

### Key Dependencies
```toml
[workspace.dependencies]
rig-core = "0.35"
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
anyhow = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
clap = { version = "4", features = ["derive"] }
```

---

## Phase 1: Multi-Model & Telemetry (Weeks 3-4)

**Goal**: Call multiple models, basic model routing, OpenTelemetry traces.

### Tasks
- [x] Implement model registry (static configuration from YAML)
- [x] Add prescriptive model routing (DSL specifies model per step)
- [x] Set up OpenTelemetry with OTLP exporter
- [x] Add `tracing-opentelemetry` bridge
- [x] Instrument model calls with GenAI semantic conventions
- [x] Deploy Jaeger all-in-one for trace visualization
- [x] Add a second provider (e.g., OpenAI for cloud fallback)

### Deliverable
- CLI can route requests to different models based on configuration
- Traces visible in Jaeger showing model calls with token counts

---

## Phase 2: Tool Calling (Weeks 5-6)

**Goal**: Agent can call tools and use results in responses.

### Tasks
- [x] Implement tool registry with built-in tools (file read, shell exec, HTTP)
- [x] Integrate Rig's tool calling with `#[tool_macro]`
- [x] Add tool call instrumentation (telemetry)
- [x] Implement basic agentic loop (model → tool → model → response)
- [x] Add tool result formatting and context injection

### Deliverable
```bash
$ cargo run -p mv-cli -- "What files are in the current directory?"
# → Agent calls file_list tool, returns formatted response
```

---

## Phase 3: MCP Integration (Weeks 7-9)

**Goal**: Connect to MCP servers for external tool access.

### Tasks
- [x] Add `rmcp` dependency with client feature
- [x] Implement MCP server configuration (YAML)
- [x] Connect to stdio-based MCP servers (spawn child processes)
- [x] Merge MCP tools into the tool registry
- [x] Connect to HTTP-based MCP servers (for network services)
- [x] Test with reference MCP servers (filesystem, git)
- [x] Instrument MCP calls in telemetry

### Deliverable
- Controller discovers and calls tools from MCP servers
- Can connect to both local (stdio) and remote (HTTP) MCP servers

---

## Phase 4: DSL Engine (Weeks 10-12)

**Goal**: Execute multi-step workflows defined in YAML.

### Tasks
- [x] Define DSL schema types in `mv-core::workflow`
- [x] Implement YAML parser with validation
- [x] Build workflow engine with sequential step execution
- [x] Add template engine for prompt interpolation (minijinja)
- [x] Implement step output passing (output of step N → input of step N+1)
- [x] Add `prompt`, `tool`, and `transform` step types
- [x] Workflow execution traces in telemetry

### Deliverable
```bash
$ cargo run -p mv-cli -- workflow run workflows/research.yaml --input topic="Rust async"
# → Executes multi-step workflow with multiple model calls and tool uses
```

---

## Phase 4.5: TRT-LLM Provider (Weeks 13-14)

**Goal**: Add TensorRT-LLM as a high-performance local inference provider
alongside Ollama.

See [TRT-LLM Integration Assessment](11-trt-llm-integration.md) for full
analysis and rationale.

### Tasks
- [x] Add `trtllm` provider to `Locality::from_provider()` → `Local`
- [x] Add `trtllm` default endpoint (`http://localhost:8003/v1`)
- [x] Route `trtllm` provider through OpenAI Chat Completions client in CLI
- [x] Add health check integration (`/health` endpoint, 2s timeout)
- [x] Extend `ModelEntry` with optional TRT-LLM metadata (`served_name`,
  `architecture`, `quant`, `expected_vram_gb`)
- [x] Add provider-specific error hints in `BackendUnreachable`
- [x] Instrument TRT-LLM calls with `gen_ai.system = "trtllm"` telemetry
- [x] Ensure MCP connections shut down on error paths (terminal fix)
- [x] Update documentation (models.yaml, README, architecture docs)
- [x] Integration tests for trtllm provider (6 tests)

### Deliverable
```bash
$ cargo run -p mv-cli -- -m llama-fp8 "Explain Rust ownership"
# → Response from TRT-LLM optimized model via OpenAI-compatible proxy
```

### Key Dependencies
- RTX 5090 with TRT-LLM engines built (via trt-llm-explore)
- trt-llm-explore OpenAI proxy running (`just up-openai && just load <model>`)

### Lessons Learned
- Rig v0.35 `openai::Client` defaults to the **Responses API** (`/v1/responses`),
  not Chat Completions. Use `openai::CompletionsClient` for OpenAI-compatible
  proxies that only implement `/v1/chat/completions`.
- MCP stdio servers inherit the terminal; if the process exits before
  `shutdown_all()`, the child can corrupt terminal state. Always shut down MCP
  connections before propagating errors.

---

## Phase 4.5.1: TRT-LLM Streaming & Hardening

**Goal**: Wire up streaming responses and improve robustness for TRT-LLM as a
first-class local inference backend.

The trt-llm-explore proxy now supports SSE streaming, tool calling, and
approximate token counts. This phase adds client-side support.

### Tasks
- [x] Add streaming support for trtllm provider (`stream_prompt()` using
  Rig's SSE streaming with `CompletionsClient`)
- [x] Surface token usage from TRT-LLM responses in telemetry spans
  (`gen_ai.usage.input_tokens`, `gen_ai.usage.output_tokens`)
- [x] Integration-test tool calling through TRT-LLM (verify agent can call
  built-in tools like `file_list` via the proxy)
- [x] Add `--stream` CLI flag for interactive streaming output
- [x] Improve error messages when model is not loaded (proxy returns 502 →
  detect and suggest `just load <model>`)
- [x] Configure proper stop sequences per provider to prevent runaway
  generation (test against all models in TRT-LLM registry)
- [x] End-to-end test: workflow with TRT-LLM model step

### Deliverable
```bash
$ cargo run -p mv-cli -- -m llama-fp8 --stream --no-tools "Explain Rust ownership"
# → Tokens stream to terminal as they arrive from TRT-LLM
```

> `--no-tools` is required for genuine streaming: the TRT-LLM proxy streams
> tool calls as plain text, so with tools attached `--stream` falls back to
> buffered output. See [TRT-LLM Integration](11-trt-llm-integration.md).

### Lessons Learned
- The proxy's 502 body wraps Triton's `"...is not found"` (and rig surfaces
  it as `HttpError`), which shadowed the `502 → ModelNotLoaded` mapping —
  the `just load` hint never fired live until the classifier was reordered.
- rig's streaming layer swallows a 502 (logs an SSE parse error, ends empty),
  so the streaming path needs a `/v1/models` preflight to surface the hint.
- The proxy streams tool calls as text, not `tool_calls` — streaming + tools
  can't round-trip, hence the `--no-tools` design.

---

## Phase 4.6: Consolidation (Weeks 15-16)

**Goal**: Make the shipped feature set true, tested, and load-bearing before Phase 5
builds on it. No new features.

Driven by the post-007 review in [docs/fable/](fable/README.md); finding IDs (F1–F34)
reference [docs/fable/02-findings.md](fable/02-findings.md). Sprint directory:
`specs/008-consolidation/`.

### Tasks
- [x] Correctness (F1–F10): UTF-8 truncation panic, real workflow `ToolExecutor`,
      wire `temperature`/`max_tokens`, retry-config panic, shell child-process leak,
      shell exit-status visibility, MCP output truncation, silent default-model
      substitution, max-turns classification, `--json` error channel
- [x] Phase-5 seam (F11–F14): extract `complete()` dispatch, typed error
      classification in `mv-core` + `is_fallback_eligible()`, `Provider` enum,
      `ModelRegistry` validation
- [x] Engine prep (F15–F18): extract `execute_step`/`execute_steps`, encapsulate
      `ExecutionContext`, `Send` bounds on executor traits, one template language,
      output-collision validation; decide the String→Value context migration
- [x] `ToolPolicy` seam, default-allow (F19)
- [x] Test infrastructure (F25–F28): wiremock fake-proxy fixture, fake stdio MCP
      server, hermetic CLI tests, end-to-end workflow test
- [x] Docs truth pass (F29–F34): README quickstart, docs/01 refresh, docs/06
      not-implemented tags, spec hygiene

### Deliverable
- Every feature claimed by README/docs works as documented
- `just ci` exercises a successful model call and a full workflow hermetically
- Phase 5 fallback/routing has a single dispatch seam and machine-readable
  failure classes to build on

### Lessons Learned
- A green gate only certifies what it exercises: workflow tool steps and
  sampling params shipped non-functional behind 189 passing tests because no
  hermetic test ever ran a successful completion or a full workflow. The
  wiremock fake-proxy fixture closed that class of gap (and cut the suite
  from ~35s to ~1.3s).
- rig's typestate `AgentBuilder<M, P, ToolState>` unifies across providers
  with plain generic helpers — no `dyn` provider abstraction was needed for
  a single dispatch seam; fallback chains operate at the `Result` level.
- Validation and rendering must share one template engine: the naive
  `{{…}}` scanner rejected valid minijinja (filters) and missed `{% if %}`
  references. `Template::undeclared_variables()` is the single source.
- `Send` bounds on async traits are cheap with two implementors and breaking
  with ten — typestate/trait shape changes belong in consolidation windows,
  not feature sprints.

---

## Phase 5: Advanced Routing & DSL Composition (Weeks 17-19)

**Goal**: Resilient model routing — fallback chains, per-provider preflight,
preference lists — and workflow composition via `branch` / `parallel` step types.

RAG is split into its own Phase 5.5 per the post-007 review
([docs/fable/03](fable/03-roadmap-recommendations.md) §Phase 5 amendments):
the original Phase 5 was the heaviest on the roadmap and splits cleanly — RAG
is genuinely new surface with no coupling to routing or the engine. Sequencing
also follows the review: fallback *mechanism* before adaptive *policy*; adaptive
scoring ([07-model-routing.md](07-model-routing.md) §2) moves to Phase 7 next to
meta-routing rather than being co-designed with the chain mechanism. Sprint
directory: `specs/009-routing-composition/`.

### Tasks
- [x] `fallback: [id, …]` on `ModelEntry` (registry-validated) +
      `complete_with_fallback()` driven by `is_fallback_eligible()` (WS1)
- [x] Generalize `trtllm::health` to a per-provider
      `preflight(entry) -> Healthy | Dead | Unknown` in mv-core; router skips
      dead locals before burning an agent build (WS2)
- [x] Hybrid routing: step-level `prefer: [id, …]` lists resolved through the
      same chain mechanism (07-model-routing §3) (WS5)
- [x] Add `branch` step type to DSL (maybe-defined output validation) (WS3)
- [x] Add `parallel` step type to DSL (fork-join, snapshot isolation,
      disjoint outputs validated at parse time) (WS4)
- [x] Routing decisions in telemetry (`router.*` spans per 07-model-routing) (WS1)

### Deliverable
- A prompt against a dead local backend transparently falls back to the next
  model in the chain, with the decision visible in traces
- Workflows branch on intermediate results and fan out independent steps
  concurrently

### Lessons Learned
- The 008 seam paid off exactly as intended: fallback was a *wrapper* over the
  existing `complete()` — `complete_chain()` walks a candidate list, and both
  `complete_with_fallback` (a model + its `fallback`) and step `prefer:` lists
  build a list and hand it over. No second routing path, no dyn abstraction.
- `PreflightStatus::Dead(MvError)` (carrying the exact error, not a string)
  let one probe serve both the router (record it) and the TRT-LLM call paths
  (return it) without losing the `just load` / `trtllm-serve` hints — the kind
  of detail that decides whether a "unify the seam" refactor actually unifies.
- rig surfaces an HTTP 500 as `HttpError`, which classifies to
  `BackendUnreachable` (fallback-*eligible*). Triggering a genuinely
  *ineligible* error hermetically meant a 200-with-no-`choices` body, not a 5xx
  — a reminder that the proxy contract is informal and the wiremock fixture is
  the only place it's pinned. **(Fixed in sprint 012:** a *reached* backend
  that answers 5xx is now `BackendErrorResponse` — truthful message, still
  fallback-eligible — instead of the misleading "Is the server running?";
  `HttpError` no longer implies unreachable. The misclassification surfaced in
  practice via `just run` against a model whose Ollama runner crashed — see
  `specs/supplemental-spec.md`.**)
- The recursive `Step` shape was the real cost of branch+parallel: `output()`
  became `Option`, and every walk (engine, validator, id/output collection,
  `outputs` mapping) had to recurse. Doing `branch` first (WS3) and letting
  `parallel` (WS4) extend the same walk kept each diff small — the maybe-defined
  set-algebra and the sibling-invisible/disjoint rules are the same walk with
  different "what's available" rules (arm intersection vs. pre-fork snapshot).
- `futures::future::join_all` on the current task (not `tokio::spawn`) kept
  parallel children borrowing `&P`/`&T` without `'static` gymnastics; the
  `Barrier` + `timeout` rendezvous test is what actually proves concurrency
  rather than asserting it.

---

## Phase 5.5: RAG Integration (Weeks 20-22)

**Goal**: Retrieval-augmented context wired into agent workflows, served by
**klams** (Ken's Local Agent Memory System) on kubs0 over MCP.

Split out of Phase 5 (see above). Amended 2026-06-12: the original task list
(build a Qdrant store, an embedding pipeline, and a RAG MCP server) is
superseded — klams already provides all three, deployed: an rmcp Streamable
HTTP MCP server with scoped bearer auth, hybrid vector+FTS retrieval, and
scanner + push ingestion. A krag-backed alternative was evaluated and set
aside (handoff doc in the krag repo, superseded). The klams tool surface m-v
depends on is pinned in `specs/010-klams-rag/contracts/`. Vector store,
embedding, and ingestion are klams's concern behind that contract — improving
them (code-aware chunking, larger embedding models) is klams roadmap, not m-v.
klams's facts/events/knowledge model is also the planned Phase 6
persistent-memory backend, so this boundary serves both phases.

### Tasks
- [x] Bearer-token auth for HTTP MCP servers (`auth_token_env`)
- [x] Agentic retrieval: klams `memory_search` in the merged toolset, proven
      hermetically (fake klams server)
- [x] Workflow retrieval: shipped RAG example (tool step → prompt step)
- [x] Tool-output cap evaluated against realistic retrieval payloads
- [x] Graceful degradation when klams is unreachable; live `#[ignore]`d
      kubs0 tests (`just test-klams`)

### Deliverable
- Agent retrieves relevant context from klams for knowledge-intensive tasks

### Lessons Learned
- The biggest win was *not building*: assessing klams (already deployed, 77
  tests, MCP + auth + retrieval + ingestion) against the krag handoff turned a
  multi-sprint "build a RAG service" into a ~4-workstream "consume one over
  MCP." The reusable move was writing the integration contract
  (`contracts/klams-tool-surface.md`) *first*, pinned to a real commit — it
  made the dependency explicit and the eventual backend swap cheap.
- The one genuinely new mechanism, bearer auth, was small because the rmcp
  client takes a caller-supplied `reqwest::Client`: a default `Authorization`
  header (marked `set_sensitive`) was the whole change, no transport fork.
  Following the existing `api_key_env` env-indirection convention meant no new
  config philosophy to invent.
- The flagged risk (rmcp client↔server Streamable-HTTP compatibility, with m-v
  on rmcp 1.5 and klams on 1.7) was retired *hermetically* by reading the
  client source: it accepts plain `application/json` (no SSE), needs an
  `Mcp-Session-Id` on initialize (not stateless by default), and tolerates a
  `405` on the background GET. A wiremock fake with a custom id-echoing
  responder then exercises the real protocol path — no live service required
  for the default suite.
- Measuring the tool-output cap beat guessing about it: top_k 5 ≈ 6.5k vs the
  10k cap, top_k 10 ≈ 13k. A one-line size test pinned the boundary and made
  "keep the universal cap, cap the example at top_k 5" a recorded decision
  rather than a latent truncation bug.

---

## Phase 6: Always-On Agent

**Goal**: Long-running agent capabilities — persistent memory, a completed
DSL, and a service binary.

Restructured 2026-06-12 (the original 8-task list was the heaviest phase on
the roadmap, like Phase 5 before the 008/009/010 split). Two of its tasks are
amended by the klams ecosystem: **system monitoring** is collected by
`klams-monitor` (m-v *consumes* its events via `event_search`, optionally
adding a small `system_info` built-in tool later), and **file watching for
project context** is covered by `klams-scanner` (real-time event *triggers*
are a separate, later feature). gRPC is dropped from the server scope (YAGNI —
REST now; "controller as MCP server" in Phase 7 is the second protocol).

### Phase 6.1: Persistent Memory via klams (sprint 011)

Because klams holds the state, persistent memory does not need the always-on
server — the CLI gains continuity first, and `mv-server` later imports the
same seam. Writes go through the same authenticated MCP boundary as Phase 5.5
(contract v1.1 adds the write tools).

- [x] Contract v1.1: `register_author`, `memory_add`, `memory_append_event`,
      `event_search` become load-bearing; Write-scoped token
- [x] `MemoryStore` trait in mv-core, klams-backed impl in the binary
      (the `PromptExecutor` pattern, per the fable Phase 6 pre-work)
- [x] Session continuity: `--session <name>` — register author, recall before
      the prompt, record the turn after; two runs, second remembers the first
- [x] Agent-writable memory: klams write tools in the merged toolset with
      per-run author attribution
- [x] Degradation (memory never blocks a prompt) + stateful fake klams +
      live kubs0 round-trip

**Deliverable**: m-v remembers across invocations — conversations recallable,
preferences learnable — with every write attributed to a registered author.

### Phase 6.1 Lessons Learned

- The 010 boundary paid off exactly as designed: memory writes reuse the
  *same* `ToolServerHandle::call_tool` path as retrieval, so the only new
  mechanism was a trait + a thin impl. No REST client, no second protocol, no
  new dependency (author ids cross the trait as opaque `String`). "Same seam"
  was real, not aspirational.
- The streamlining insight held: persistent memory did **not** need
  `mv-server`. Continuity is about durable state (which klams owns), not about
  staying resident — so the CLI got memory now, and 6.3's server becomes a
  later consumer of the same `MemoryStore`.
- The stateful fake was the unlock for testing memory honestly: making
  `FakeKlams` accumulate writes let a *write→recall round-trip across two
  separate CLI processes* be proven hermetically (the fixture lives in the
  test process; both subprocesses talk to it). A static fake could only have
  proven reads.
- One deviation worth flagging forward: the model's memory-capability note
  rides the **prompt prefix**, not a true system-preamble suffix — the
  preamble is hardcoded at the four provider call sites and threading an
  override through `complete`/`complete_chain` wasn't worth it for v1. If
  agent writes prove unreliable, that threading (or a typed `memory_add`
  wrapper tool) is the next step.
- `mv-cli` has no library target, so `KlamsMemory` can't be imported by
  `tests/`. Its wire behavior is proven black-box through the CLI; only pure
  render/parse logic is unit-tested in-crate. Worth remembering before
  promising "integration-test X directly" for any binary-crate type.

### Phase 6.2: DSL Completion (sprint 012)

The `String → serde_json::Value` context migration (decided in 008; the most
breaking change on the roadmap, landed before `mv-server` multiplies
consumers), then the step types that need it.

- [ ] `Value` context migration (typed conditions, structured tool results)
- [ ] `loop` step (max_iterations, typed exit_condition)
- [ ] Nested `workflow` step (cross-file cycle detection, depth cap)

**Deliverable**: workflows iterate, compose, and carry structured data.

### Phase 6.3: mv-server (sprint 013)

- [ ] Axum REST API server (`mv-server`); `MvError::code()` for
      machine-readable errors
- [ ] Session/conversation management over the API (held-open agents via a
      closed `enum AnyAgent`; memory via the 6.1 seam)
- [ ] Scheduled workflow execution
- [ ] Daemon pre-work from the fable register: MCP connection manager (F22),
      shared `reqwest::Client` + `tokio::fs` in tool paths (F23)
- [ ] Graceful shutdown and state persistence

**Deliverable**: controller runs as a system service; accepts requests via
API, executes scheduled workflows, consumes klams-monitor events.

---

## Phase 7: Polish & Dashboard Foundation (Weeks 25+)

**Goal**: Production-grade telemetry export and dashboard-ready APIs.

### Tasks
- [ ] Refine telemetry: custom metrics, dashboard-oriented spans
- [ ] Add Prometheus metrics endpoint
- [ ] Expose WebSocket for real-time event streaming (for dashboard)
- [ ] Controller as MCP server (expose capabilities to other AI tools)
- [ ] Adaptive-scoring & meta-routing experiments (router scores candidates by
      task metadata; model selects model) — policies layered on the Phase 5
      fallback mechanism
- [ ] Security hardening (API auth, tool sandboxing, secret management)
- [ ] Documentation and examples
- [ ] Begin companion dashboard project (separate repo)

---

## Learning Milestones

Throughout implementation, these are the key learning opportunities:

| Phase | Learning Focus |
|-------|---------------|
| 0 | Rust async, Rig API, LLM basics |
| 1 | Model differences, tokenization, OpenTelemetry |
| 2 | Function calling, structured output, agentic patterns |
| 3 | MCP protocol, inter-process communication, service architecture |
| 4 | DSL design, template engines, workflow orchestration |
| 5 | Routing algorithms, failure classification, structured concurrency |
| 5.5 | Embeddings, vector search, RAG tuning |
| 6 | System programming, service architecture, state management |
| 7 | Observability engineering, security, system design |

## Principles to Follow

1. **Working software over perfect design**: Get something running, then refine
2. **Instrument everything**: You can't improve what you can't measure
3. **Test with real models**: Unit tests with mocks are necessary but not
   sufficient — test against actual Ollama models
4. **Document decisions**: When you make an architecture choice, record the
   alternatives considered and why you chose what you did
5. **Small PRs**: Each phase should produce multiple small, reviewable changes
6. **Don't optimize prematurely**: Profile first, then optimize the hot paths
