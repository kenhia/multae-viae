# Architecture Design

## System Overview

The controller follows a layered architecture with clear separation of concerns.
The core philosophy is: **small core, extensible surface**.

```
┌─────────────────────────────────────────────────────────────┐
│                     CLI / API Surface                       │
│  (gRPC server, REST API, CLI commands, WebSocket events)    │
├─────────────────────────────────────────────────────────────┤
│                   Orchestration Layer                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐   │
│  │ Workflow │  │  Model   │  │   Tool   │  │  Context   │   │
│  │  Engine  │  │  Router  │  │ Registry │  │  Manager   │   │
│  └──────────┘  └──────────┘  └──────────┘  └────────────┘   │
├─────────────────────────────────────────────────────────────┤
│                   Integration Layer                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐   │
│  │   MCP    │  │  Model   │  │   RAG    │  │  External  │   │
│  │  Client  │  │ Backends │  │  Client  │  │  Services  │   │
│  └──────────┘  └──────────┘  └──────────┘  └────────────┘   │
├─────────────────────────────────────────────────────────────┤
│                Cross-Cutting Concerns                       │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────┐   │
│  │Telemetry │  │ Logging  │  │  Config  │  │  Security  │   │
│  │  (OTel)  │  │(tracing) │  │  (DSL)   │  │  (AuthZ)   │   │
│  └──────────┘  └──────────┘  └──────────┘  └────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

## Component Breakdown

### 1. Orchestration Layer

#### Workflow Engine

The heart of the system. Responsible for:

- Parsing and executing DSL-defined workflows (YAML pipelines)
- Managing step sequencing, branching, and error handling
- Maintaining conversation/session state across multi-turn interactions
- Supporting both prescriptive (DSL-defined) and autonomous (agent-decided)
  execution modes

**Implementation** (`crates/mv-core/src/workflow/`):

The DSL engine is implemented as a module within `mv-core` and has **no
dependency on Rig or any provider** — it is generic over `PromptExecutor` /
`ToolExecutor` traits (both `Send`-bounded) and unit-tested with mocks:

- `types.rs` — Workflow, Step (prompt/tool/transform/branch/parallel), Input,
  Output, and `ModelSpec` (`Single` | `Prefer`) types with
  `#[serde(deny_unknown_fields)]` for strict YAML parsing. `Step` is recursive
  (branch/parallel own nested step lists); `Step::output()` is `Option` since
  control-flow steps have no single output
- `parser.rs` — YAML loading via `serde_yml` with error mapping
- `validate.rs` — Structural validation via a recursive walk (duplicate step
  IDs, duplicate output names, unresolvable refs, circular references, retry
  config, template syntax/reference checks via minijinja — including
  `template_file` contents). For `branch`: condition syntax + **maybe-defined**
  analysis (an output is available afterward only if every arm defines it). For
  `parallel`: sibling outputs are invisible (validated against the pre-fork
  context) and must be disjoint
- `template.rs` — Variable interpolation using `minijinja` with
  `{{variable}}` syntax; plus `evaluate_condition`/`condition_references` for
  `branch` conditions (minijinja expressions, one engine for both)
- `engine.rs` — Execution engine; an encapsulated `ExecutionContext` where step
  outputs shadow workflow inputs; the default model is a required parameter (no
  hardcoded fallback). `branch` recurses into the chosen arm; `parallel`
  fork-joins children via `futures::future::join_all`, each against a context
  snapshot, merging disjoint outputs at the join
- `retry.rs` — skip/fail/retry error strategies; retry re-attempts only
  transient (`is_retryable()`) errors, with configurable `base_delay_ms`,
  exponential or fixed backoff, and a 30s delay cap
- `transform.rs` — Transform operations (currently only `extract_json`)

CLI subcommands `workflow run` and `workflow validate` are exposed via
`mv-cli` using `clap` subcommand groups.

```rust
// Conceptual interface
trait WorkflowEngine {
    async fn execute(&self, workflow: Workflow, context: &mut Context) -> Result<Output>;
    async fn step(&self, step: Step, context: &mut Context) -> Result<StepResult>;
}
```

#### Model Router

Selects the model for each step. Sprint 009 shipped the mechanism on the
Sprint 008 seam (`complete()` dispatch + `is_fallback_eligible()`):

1. **Prescriptive** *(shipped)*: DSL specifies an exact model per step
2. **Hybrid** *(shipped, sprint 009)*: a step `model: { prefer: [...] }` list
   and/or a `fallback: [...]` chain on a `models.yaml` entry, walked by
   `complete_chain()` in `mv-cli/src/providers.rs` — the first reachable model
   serves. `mv_core::preflight` skips dead locals before an agent is built; the
   substitution surfaces via a stderr note, the `--json` `model_used` field, and
   `router.*` span attributes
3. **Adaptive** *(future — Phase 7)*: router selects based on task metadata
   (complexity, domain, latency), layered on the same mechanism

See [07 — Model Routing](07-model-routing.md) for detailed strategies.

#### Tool Registry

Manages available tools from multiple sources:

- **MCP Servers**: Discovered dynamically via MCP protocol (stdio and
  streamable-HTTP transports, configured in `mcp-servers.yaml`)
- **Built-in Tools**: File I/O, shell execution, HTTP requests — gated
  behind the `ToolPolicy` seam (default-allow today; Phase 7 sandboxing
  lands as deny rules on this type)
- **Custom Tools** *(future)*: User-defined Rust functions registered at startup
- **Remote Tools**: Accessible via MCP over HTTP

MCP tools merge into the same tool set the model sees; built-in tools take
precedence on name collision, MCP server failures are logged and skipped,
and all tool output (built-in and MCP) truncates at 10,000 chars.

```rust
trait ToolRegistry {
    async fn list_tools(&self) -> Vec<ToolDescription>;
    async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult>;
    async fn refresh(&self);  // Re-discover from MCP servers
}
```

#### Context Manager *(future)*

Will maintain the environmental context for agent operations:

- System information (OS, hardware, running processes)
- User preferences and history
- Active project/workspace context
- Conversation memory (short-term and long-term via RAG)
- Retrieved documents from RAG

### 2. Integration Layer

#### MCP Client

Uses the **RMCP** crate (`rmcp`) to connect to MCP servers. Supports:

- **stdio transport**: For local MCP servers (spawned as child processes)
- **Streamable HTTP transport**: For remote MCP servers on the network,
  with optional **bearer auth** via `auth_token_env` (sprint 010) — the
  token is read from a named env var into an `Authorization` header, kept
  out of config/logs/traces, and rejected on stdio entries
- Multiple simultaneous server connections
- Dynamic capability discovery (tools, resources, prompts)

#### Model Backends

Abstraction over multiple inference providers:

| Backend | Transport | Use Case |
|---------|-----------|----------|
| Ollama | HTTP API (localhost:11434) | Primary local inference, model management |
| TensorRT-LLM | OpenAI-compatible API (localhost:8003/v1) | High-performance local GPU inference via OpenAI-compatible proxy |
| OpenAI-compatible | HTTP API | Cloud fallback (OpenAI, etc.) |
| mistral.rs *(future)* | Embedded Rust library | High-performance embedded inference |

`Provider` is a **closed enum** (`ollama` / `openai` / `trtllm`): an unknown
provider string in `models.yaml` is rejected at config load with the list of
valid values, instead of resolving to fictitious defaults and failing at call
time. The registry also rejects duplicate model ids, multiple `default: true`
entries, and unknown `ModelEntry` fields; a missing config file reports
`Config file not found` rather than a parse error.

**TRT-LLM Provider** (`crates/mv-core/src/trtllm/`):

The TRT-LLM provider uses Rig's `CompletionsClient` (not the default
`openai::Client` which targets the Responses API) since the TRT-LLM proxy exposes
an OpenAI-compatible `/v1/chat/completions` endpoint. The provider module adds:

- `health.rs` — Pre-prompt health check against `/health` endpoint (2s
  timeout) plus a served-model preflight for the streaming path
- `stop.rs` — Provider-default stop sequences (`</s>`, `<|im_end|>`,
  `<|eot_id|>`) forwarded via Rig `additional_params`
- `usage.rs` — Lenient token-usage deserialization, recorded as
  `gen_ai.usage.{input,output}_tokens` span attributes
- Provider dispatch in `mv-cli/src/providers.rs` (`call_trtllm()` /
  `stream_trtllm()` sharing preflight, agent builder, and usage helpers)
- `gen_ai.system = "trtllm"` span attribute for OpenTelemetry traces
- `served_name` field on `ModelEntry` for HuggingFace path → short name mapping
- A 502 from the proxy classifies as `ModelNotLoaded` with a
  `Run: just load <id>` hint (referring to the *trt-llm-explore* justfile)

#### RAG Client *(shipped — Phase 5.5 / sprint 010)*

Retrieval is the **klams** memory service on kubs0, consumed over
authenticated MCP — not a service built here:

- klams owns embeddings (HF TEI), the vector store (Qdrant), chunking, and
  ingestion (its filesystem scanner); m-v never embeds or stores
- m-v calls `memory_search` (read-only this sprint, `Read`-scoped token); it
  merges into the agent tool set and is reachable from workflow `tool` steps
- Bearer auth via `auth_token_env` on the HTTP MCP entry
- See [08-rag-integration.md](08-rag-integration.md) and the pinned tool
  contract in `specs/010-klams-rag/contracts/`

#### External Services

Any additional integrations (calendars, file watchers, notification systems)
connected via MCP or direct API calls.

### 3. Cross-Cutting Concerns

#### Telemetry

First-class requirement. See [05 — Telemetry](05-telemetry-observability.md).

- **Traces**: Every workflow execution, model call, tool invocation
- **Metrics**: Token counts, latency distributions, cache hit rates
- **Logs**: Structured logging via `tracing` crate
- Export to OpenTelemetry collector → dashboard

#### Configuration (DSL)

YAML-based workflow definitions. See [06 — DSL Design](06-dsl-flow-management.md).

#### Security *(future — Phase 7)*

- API authentication for remote access
- Tool execution sandboxing — the hook point exists today:
  `mv_core::tools::ToolPolicy` (default-allow) is consulted by all four
  built-in tools, so sandboxing lands as a policy implementation, not a
  tool rewrite
- Secret management for API keys
- Audit logging of all tool executions

## Data Flow

### Simple Request Flow

```
User Request
    │
    ▼
┌──────────┐     ┌──────────┐     ┌──────────┐
│  Parse   │────▶│  Route   │────▶│ Execute  │
│  Intent  │     │  Model   │     │  Step    │
└──────────┘     └──────────┘     └──────────┘
                                       │
                              ┌────────┴────────┐
                              ▼                  ▼
                        ┌──────────┐      ┌──────────┐
                        │  Model   │      │   Tool   │
                        │  Call    │      │   Call   │
                        └──────────┘      └──────────┘
                              │                  │
                              ▼                  ▼
                        ┌──────────────────────────┐
                        │   Aggregate Results      │
                        │   Update Context         │
                        │   Check for More Steps   │
                        └──────────────────────────┘
                                    │
                                    ▼
                              ┌──────────┐
                              │ Response │
                              └──────────┘
```

### Agentic Loop Flow

```
User Goal
    │
    ▼
┌──────────────┐
│  Plan Steps  │◀──────────────────────┐
│  (via LLM)   │                       │
└──────────────┘                       │
       │                               │
       ▼                               │
┌──────────────┐     ┌──────────┐      │
│Execute Step  │────▶│Evaluate  │──────┘
│(tool/model)  │     │Result    │  (needs more steps)
└──────────────┘     └──────────┘
                          │
                          ▼ (goal achieved)
                    ┌──────────┐
                    │ Response │
                    └──────────┘
```

## Crate Structure

A deliberately small two-crate workspace. The originally proposed satellite
crates (`mv-engine`, `mv-mcp`, `mv-dsl`, `mv-telemetry`) were absorbed into
`mv-core` as modules — the standing rule is **anything a future `mv-server`
needs lives in `mv-core`**; the binary crate holds only CLI concerns.

```
multae-viae/
├── Cargo.toml              # Workspace root (resolver = "3", edition 2024)
├── crates/
│   ├── mv-core/            # Library: all reusable logic
│   │   └── src/
│   │       ├── lib.rs       # ModelEntry/ModelRegistry, Provider enum, MvError
│   │       ├── providers.rs # SYSTEM_PREAMBLE, error classification,
│   │       │                #   is_fallback_eligible / is_retryable taxonomy
│   │       ├── mcp/         # MCP config, RMCP client, tool registry merge
│   │       ├── tools/       # Built-in tools + ToolPolicy seam
│   │       │   ├── mod.rs    # ToolPolicy, constants, truncation helper
│   │       │   ├── file_list.rs
│   │       │   ├── file_read.rs
│   │       │   ├── shell_exec.rs
│   │       │   └── http_get.rs
│   │       ├── preflight.rs # per-provider liveness probe (Healthy/Dead/Unknown)
│   │       ├── trtllm/      # health.rs, stop.rs, usage.rs
│   │       └── workflow/    # DSL engine (rig-free)
│   │           ├── types.rs / parser.rs / validate.rs / template.rs
│   │           └── engine.rs / retry.rs / transform.rs
│   ├── mv-cli/             # CLI binary
│   │   └── src/
│   │       ├── main.rs      # Output contract (stdout/stderr × --json), dispatch
│   │       ├── cli.rs       # clap definitions
│   │       ├── providers.rs # complete() seam + complete_chain() fallback walker
│   │       ├── telemetry.rs # tracing + OTLP-HTTP exporter wiring
│   │       ├── executors.rs # RigPromptExecutor / HandleToolExecutor
│   │       └── commands/    # prompt.rs, workflow.rs
│   └── mv-server/          # gRPC/REST API server (future — Phase 6)
├── docs/                   # This documentation
├── specs/                  # Sprint specifications (SDD)
└── workflows/              # Example workflow YAML files
```

## Current Implementation (through Sprint 012)

The CLI operates as an agentic system with built-in tools (Sprint 003). The
architecture uses Rig's native multi-turn agent loop — tools are registered with
the agent builder and the model decides when and how to invoke them.

Subsequent sprints layered on:

- **Sprint 004 — MCP**: external MCP servers (stdio + streamable-HTTP)
  configured via `mcp-servers.yaml` merge into the same tool set; built-ins
  win name collisions, failed servers are skipped, connections shut down
  gracefully on exit.
- **Sprint 005 — DSL engine**: `mv_core::workflow` executes YAML workflows
  (prompt/tool/transform steps) through executor traits, keeping the engine
  free of Rig and provider dependencies.
- **Sprints 006/007 — TRT-LLM**: third provider via the OpenAI-compatible
  proxy; health preflight, stop-sequence defaults, token-usage telemetry,
  502 → `ModelNotLoaded` classification, and `--stream` (TRT-LLM only;
  `--no-tools` required for genuine streaming).
- **Sprint 008 — consolidation**: one `complete()` dispatch seam shared by
  the prompt command and the workflow executor (the Phase 5 fallback-chain
  hook); `SYSTEM_PREAMBLE` and typed-first error classification moved to
  `mv_core::providers`; closed `Provider` enum and registry validation;
  workflow tool steps execute real built-in/MCP tools via
  `HandleToolExecutor`; `temperature`/`max_tokens` honored;
  `ToolPolicy` seam for Phase 7 sandboxing; new `MvError` variants
  (`MaxTurnsExceeded`, `ToolCallFailed`, `WorkflowStepError`,
  `ConfigNotFound`); `--json` errors print to stderr (channel unchanged).
- **Sprint 009 — routing & DSL composition**: `fallback: [...]` chains on
  `ModelEntry` and step-level `model: { prefer: [...] }` lists, both walked by
  `complete_chain()` (advance on `is_fallback_eligible()` errors, fail fast
  otherwise); per-provider `mv_core::preflight` skips dead locals; the
  substitution surfaces via stderr note / `--json` `model_used` / `router.*`
  spans. New DSL steps `branch` (minijinja-expression condition, maybe-defined
  validation) and `parallel` (fork-join, snapshot isolation, disjoint outputs).
  New `MvError` variants (`AllModelsFailed`, `WorkflowParallelFailed`).
- **Sprint 010 — RAG via klams**: bearer auth for HTTP MCP servers
  (`auth_token_env` → `Authorization` header, secret kept out of
  config/logs/traces, HTTP-only). Retrieval is served by the **klams** memory
  service on kubs0 over authenticated MCP — `memory_search` merges into the
  tool set (read-only this sprint); workflows retrieve via a `tool` step
  (`workflows/examples/rag-example.yaml`). No vector store, embedding, or RAG
  server is built here — klams owns that behind the contract in
  `specs/010-klams-rag/contracts/`. Tested hermetically against a fake klams
  MCP server; live round-trips behind `just test-klams`.
- **Sprint 011 — persistent memory via klams**: `mv_core::memory::MemoryStore`
  (trait in core, `KlamsMemory` impl in the binary — the `PromptExecutor`
  pattern) gives the CLI cross-invocation continuity through the *same* MCP
  handle as retrieval (no new client/protocol). `--session <name>` registers a
  per-run author, recalls prior turns + relevant memories before the
  completion, and records the turn after; with a `Read|Write` token the model
  can also call `memory_add` itself, attributed to the run's author. Memory is
  best-effort — never blocks a prompt. `FakeKlams` is now stateful; live
  round-trips behind `just test-klams`. Contract v1.1 in
  `specs/011-klams-memory/contracts/`. New `MvError::MemoryError`.
- **Sprint 012 — DSL completion**: the workflow `ExecutionContext` migrated
  from `String` to `serde_json::Value`, so templates do field access
  (`{{report.title}}`) and conditions compare typed (`score >= 8` numeric);
  `extract_json` stores the parsed value. New steps `loop` (do-while with a
  typed `exit_condition` and `max_iterations` cap) and `workflow` (run another
  file as a step — isolated child context, child outputs returned as one
  object, cross-file cycle detection + depth cap). Also a pulled-in routing
  fix: a backend that *responds* with a 5xx is now `BackendErrorResponse`
  (truthful, fallback-eligible) instead of being misclassified as
  `BackendUnreachable`. New `MvError` variants (`BackendErrorResponse`,
  `WorkflowCycle`, `WorkflowDepthExceeded`).

### Tool Architecture

```
User Prompt
    │
    ▼
┌───────────────────────────────────┐
│  Agent (Rig AgentBuilder)         │
│  ├── preamble (SYSTEM_PREAMBLE)   │
│  ├── tools: [FileList, FileRead,  │
│  │     ShellExec, HttpGet] + MCP  │
│  └── default_max_turns            │
│      (ModelEntry.max_turns ?? 10) │
└───────────────┬───────────────────┘
                │
    ┌───────────▼────────────┐
    │    Rig Agentic Loop    │
    │  (internal multi-turn) │
    └───────────┬────────────┘
                │
        ┌───────┴───────┐
        ▼               ▼
  Text Response    Tool Call(s)
  (return to       (execute locally)
   user)                │
                        ▼
                  Tool Result(s)
                  (feed back to model)
```

### Built-in Tools

| Tool | Module | Description | Timeout |
|------|--------|-------------|---------|
| `file_list` | `mv_core::tools::file_list` | List directory contents | N/A |
| `file_read` | `mv_core::tools::file_read` | Read file contents | N/A |
| `shell_exec` | `mv_core::tools::shell_exec` | Execute shell command | 30s |
| `http_get` | `mv_core::tools::http_get` | HTTP GET request | 30s |

All tool output is truncated at 10,000 characters. Tools are implemented using
the `#[rig_tool]` macro and instrumented with `#[tracing::instrument]` for
OpenTelemetry trace visibility.

### Error Flow

Tool errors are returned as `ToolError::ToolCallError(String)`. Rig feeds error
messages back to the model automatically, allowing the model to retry or explain
the failure without crashing the session.

## Technology Stack Summary

Current dependencies (see the crate `Cargo.toml`s):

| Category | Technology | Crate |
|----------|-----------|-------|
| Async Runtime | Tokio | `tokio` |
| HTTP Client | reqwest | `reqwest` |
| Serialization | serde + serde_yml + serde_json | `serde`, `serde_yml`, `serde_json` |
| Templating | minijinja | `minijinja` |
| Agent Framework | Rig | `rig-core` |
| MCP | RMCP | `rmcp` |
| Telemetry | OpenTelemetry | `opentelemetry`, `opentelemetry_sdk`, `opentelemetry-otlp` |
| Tracing | tracing ecosystem | `tracing`, `tracing-subscriber`, `tracing-opentelemetry` |
| CLI | clap | `clap` |
| Error Handling | thiserror (single `MvError` enum) | `thiserror` |
| Local Inference | Ollama + TRT-LLM proxy, via Rig's HTTP providers | `rig-core` |

Future (not yet dependencies): an HTTP/gRPC server stack for `mv-server`
(Phase 6) and an embedded-inference backend (mistral.rs) if adopted.
