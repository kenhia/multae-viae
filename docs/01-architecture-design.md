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

```rust
// Conceptual interface
trait WorkflowEngine {
    async fn execute(&self, workflow: Workflow, context: &mut Context) -> Result<Output>;
    async fn step(&self, step: Step, context: &mut Context) -> Result<StepResult>;
}
```

#### Model Router

Selects the appropriate model for each task. Supports three modes:

1. **Prescriptive**: DSL specifies exact model per step
2. **Adaptive**: Router selects based on task metadata (complexity, domain,
   latency requirements)
3. **Hybrid**: DSL provides constraints/preferences, router selects within those
   bounds

See [07 — Model Routing](07-model-routing.md) for detailed strategies.

#### Tool Registry

Manages available tools from multiple sources:

- **MCP Servers**: Discovered dynamically via MCP protocol
- **Built-in Tools**: File I/O, shell execution, HTTP requests
- **Custom Tools**: User-defined Rust functions registered at startup
- **Remote Tools**: Accessible via MCP over HTTP/WebSocket

```rust
trait ToolRegistry {
    async fn list_tools(&self) -> Vec<ToolDescription>;
    async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult>;
    async fn refresh(&self);  // Re-discover from MCP servers
}
```

#### Context Manager

Maintains the environmental context for agent operations:

- System information (OS, hardware, running processes)
- User preferences and history
- Active project/workspace context
- Conversation memory (short-term and long-term via RAG)
- Retrieved documents from RAG

### 2. Integration Layer

#### MCP Client

Uses the **RMCP** crate (`rmcp`) to connect to MCP servers. Supports:

- **stdio transport**: For local MCP servers (spawned as child processes)
- **Streamable HTTP transport**: For remote MCP servers on the network
- Multiple simultaneous server connections
- Dynamic capability discovery (tools, resources, prompts)

#### Model Backends

Abstraction over multiple inference providers:

| Backend | Transport | Use Case |
|---------|-----------|----------|
| Ollama | HTTP API (localhost:11434) | Primary local inference, model management |
| mistral.rs | Embedded Rust library | High-performance embedded inference |
| OpenAI-compatible | HTTP API | Cloud fallback (OpenAI, Anthropic, etc.) |
| Rig providers | HTTP API | 20+ providers via unified interface |

#### RAG Client

Connects to a RAG service on the local network:

- Embedding generation (local via Candle/Ollama or remote)
- Vector store queries (Qdrant, LanceDB)
- Document ingestion pipeline
- Exposed as an MCP server for the controller to consume

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

#### Security

- API authentication for remote access
- Tool execution sandboxing
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

## Crate Structure (Proposed)

```
multae-viae/
├── Cargo.toml              # Workspace root
├── crates/
│   ├── mv-core/            # Core types, traits, error handling
│   ├── mv-engine/          # Workflow engine, step execution
│   ├── mv-router/          # Model routing logic
│   ├── mv-mcp/             # MCP client integration (wraps rmcp)
│   ├── mv-tools/           # Built-in tool implementations
│   ├── mv-telemetry/       # OTel setup, custom spans/metrics
│   ├── mv-dsl/             # YAML DSL parser and validator
│   ├── mv-rag/             # RAG client, embedding pipeline
│   ├── mv-server/          # gRPC/REST API server
│   └── mv-cli/             # CLI binary
├── config/                 # Default configurations
├── workflows/              # Example DSL workflow files
└── docs/                   # This documentation
```

## Technology Stack Summary

| Category | Technology | Crate |
|----------|-----------|-------|
| Async Runtime | Tokio | `tokio` |
| HTTP Client | reqwest | `reqwest` |
| HTTP Server | axum or tonic | `axum` / `tonic` |
| Serialization | serde + serde_yaml + serde_json | `serde`, `serde_yaml`, `serde_json` |
| Agent Framework | Rig | `rig-core` |
| MCP | RMCP | `rmcp` |
| Telemetry | OpenTelemetry | `opentelemetry`, `opentelemetry-otlp` |
| Tracing | tracing ecosystem | `tracing`, `tracing-subscriber`, `tracing-opentelemetry` |
| CLI | clap | `clap` |
| Error Handling | anyhow + thiserror | `anyhow`, `thiserror` |
| ML Framework | Candle | `candle-core`, `candle-nn` |
| Local Inference | Ollama API | `ollama-rs` or HTTP via `reqwest` |
| Embedded Inference | mistral.rs | `mistralrs` |
