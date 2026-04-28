# Implementation Roadmap

## Phase 0: Foundation (Weeks 1-2)

**Goal**: Working Rust workspace that can call a local model and print a response.

### Tasks
- [ ] Initialize Cargo workspace with crate structure
- [ ] Set up `mv-core` with core types and error handling
- [ ] Add Rig dependency, configure Ollama provider
- [ ] Write a simple CLI that sends a prompt to Ollama via Rig
- [ ] Set up basic `tracing` with console output
- [ ] Establish CI (cargo check, clippy, test)

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
- [ ] Implement model registry (static configuration from YAML)
- [ ] Add prescriptive model routing (DSL specifies model per step)
- [ ] Set up OpenTelemetry with OTLP exporter
- [ ] Add `tracing-opentelemetry` bridge
- [ ] Instrument model calls with GenAI semantic conventions
- [ ] Deploy Jaeger all-in-one for trace visualization
- [ ] Add a second provider (e.g., OpenAI for cloud fallback)

### Deliverable
- CLI can route requests to different models based on configuration
- Traces visible in Jaeger showing model calls with token counts

---

## Phase 2: Tool Calling (Weeks 5-6)

**Goal**: Agent can call tools and use results in responses.

### Tasks
- [ ] Implement tool registry with built-in tools (file read, shell exec, HTTP)
- [ ] Integrate Rig's tool calling with `#[tool_macro]`
- [ ] Add tool call instrumentation (telemetry)
- [ ] Implement basic agentic loop (model → tool → model → response)
- [ ] Add tool result formatting and context injection

### Deliverable
```bash
$ cargo run -p mv-cli -- "What files are in the current directory?"
# → Agent calls file_list tool, returns formatted response
```

---

## Phase 3: MCP Integration (Weeks 7-9)

**Goal**: Connect to MCP servers for external tool access.

### Tasks
- [ ] Add `rmcp` dependency with client feature
- [ ] Implement MCP server configuration (YAML)
- [ ] Connect to stdio-based MCP servers (spawn child processes)
- [ ] Merge MCP tools into the tool registry
- [ ] Connect to HTTP-based MCP servers (for network services)
- [ ] Test with reference MCP servers (filesystem, git)
- [ ] Instrument MCP calls in telemetry

### Deliverable
- Controller discovers and calls tools from MCP servers
- Can connect to both local (stdio) and remote (HTTP) MCP servers

---

## Phase 4: DSL Engine (Weeks 10-12)

**Goal**: Execute multi-step workflows defined in YAML.

### Tasks
- [ ] Define DSL schema types in `mv-dsl`
- [ ] Implement YAML parser with validation
- [ ] Build workflow engine with sequential step execution
- [ ] Add template engine for prompt interpolation (handlebars or minijinja)
- [ ] Implement step output passing (output of step N → input of step N+1)
- [ ] Add `prompt`, `tool`, and `transform` step types
- [ ] Workflow execution traces in telemetry

### Deliverable
```bash
$ cargo run -p mv-cli -- workflow run workflows/research.yaml --input topic="Rust async"
# → Executes multi-step workflow with multiple model calls and tool uses
```

---

## Phase 5: Advanced Routing & RAG (Weeks 13-16)

**Goal**: Adaptive model routing and RAG integration.

### Tasks
- [ ] Implement adaptive model routing algorithm
- [ ] Add hybrid routing with preference lists and fallbacks
- [ ] Set up Qdrant vector store (Docker)
- [ ] Implement embedding pipeline (Ollama + nomic-embed-text)
- [ ] Build RAG MCP server for network deployment
- [ ] Integrate RAG context into agent workflows
- [ ] Add document ingestion pipeline
- [ ] Add `branch` and `parallel` step types to DSL

### Deliverable
- Agent selects appropriate models based on task characteristics
- Agent retrieves relevant context from RAG for knowledge-intensive tasks

---

## Phase 6: Always-On Agent (Weeks 17-20)

**Goal**: Long-running agent service with API and monitoring capabilities.

### Tasks
- [ ] Build gRPC/REST API server (`mv-server`)
- [ ] Implement session/conversation management
- [ ] Add file system watching for project context
- [ ] Implement system monitoring capabilities (CPU, memory, processes)
- [ ] Add scheduled task execution
- [ ] Implement persistent memory (conversations, learned preferences)
- [ ] Add `loop` and `workflow` (nested) step types to DSL
- [ ] Graceful shutdown and state persistence

### Deliverable
- Controller runs as a system service
- Accepts requests via API, monitors system, executes scheduled workflows

---

## Phase 7: Polish & Dashboard Foundation (Weeks 21+)

**Goal**: Production-grade telemetry export and dashboard-ready APIs.

### Tasks
- [ ] Refine telemetry: custom metrics, dashboard-oriented spans
- [ ] Add Prometheus metrics endpoint
- [ ] Expose WebSocket for real-time event streaming (for dashboard)
- [ ] Controller as MCP server (expose capabilities to other AI tools)
- [ ] Meta-routing experiments (model selects model)
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
| 5 | Embeddings, vector search, RAG tuning, routing algorithms |
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
