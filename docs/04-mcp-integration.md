# MCP Integration

## What is MCP?

The **Model Context Protocol** (MCP) is an open standard for connecting AI
applications to external systems. Think of it as USB-C for AI — a standardized
interface that lets any AI application connect to any data source, tool, or
workflow.

**Specification**: [modelcontextprotocol.io](https://modelcontextprotocol.io/)

MCP is supported by Claude, ChatGPT, VS Code Copilot, Cursor, and many other
AI tools. Building with MCP means your tools are interoperable with the entire
ecosystem.

## MCP Architecture

```
┌──────────────────────────────────────────────────┐
│              MCP Host (Multae Viae)              │
│                                                  │
│  ┌────────────┐  ┌────────────┐  ┌────────────┐  │
│  │ MCP Client │  │ MCP Client │  │ MCP Client │  │
│  │     #1     │  │     #2     │  │     #3     │  │
│  └─────┬──────┘  └─────┬──────┘  └─────┬──────┘  │
└────────┼───────────────┼───────────────┼─────────┘
         │               │               │
    ┌────▼────┐    ┌─────▼─────┐   ┌─────▼─────┐
    │  Local  │    │  Local    │   │  Remote   │
    │ Server  │    │  Server   │   │  Server   │
    │ (stdio) │    │  (stdio)  │   │  (HTTP)   │
    │         │    │           │   │           │
    │ e.g.    │    │ e.g.      │   │ e.g.      │
    │ File    │    │ Database  │   │ RAG on    │
    │ System  │    │ Access    │   │ network   │
    └─────────┘    └───────────┘   └───────────┘
```

### Key Concepts

- **MCP Host**: Your application (Multae Viae) that coordinates MCP clients
- **MCP Client**: A connection to a single MCP server, handles protocol
  negotiation
- **MCP Server**: A program that provides tools, resources, or prompts

### Protocol Primitives

| Primitive | Direction | Purpose |
|-----------|-----------|---------|
| **Tools** | Server → Client | Executable functions (file ops, API calls, DB queries) |
| **Resources** | Server → Client | Data sources (file contents, DB records, API responses) |
| **Prompts** | Server → Client | Reusable message templates for LLM interactions |
| **Sampling** | Client ← Server | Server asks client to run LLM completion |
| **Logging** | Server → Client | Structured log messages |
| **Notifications** | Bidirectional | Real-time updates about capability changes |

### Transport Mechanisms

| Transport | Use Case | Connection |
|-----------|----------|------------|
| **stdio** | Local servers on same machine | Process stdin/stdout |
| **Streamable HTTP** | Remote servers on network | HTTP POST + SSE |

## RMCP — The Official Rust SDK

**Repository**: [github.com/modelcontextprotocol/rust-sdk](https://github.com/modelcontextprotocol/rust-sdk)
**Stars**: 3.3k | **Crate**: `rmcp` v1.5.0 | **License**: Apache-2.0

RMCP is the official Rust implementation of MCP. It provides both client and
server capabilities.

### Installation

```toml
[dependencies]
rmcp = { version = "1.5.0", features = ["client"] }
# For building MCP servers:
# rmcp = { version = "1.5.0", features = ["server"] }
```

### Client Usage (Connecting to MCP Servers)

```rust
use rmcp::{ServiceExt, transport::stdio};

// Connect to a local MCP server via stdio
let service = ().serve(stdio()).await?;

// List available tools
let tools = service.list_all_tools().await?;

// Call a tool
let result = service.call_tool(CallToolRequestParams::new("search"))
    .await?;
```

### Server Usage (Exposing Your Own Tools)

```rust
use rmcp::{tool, tool_router, ServiceExt, transport::stdio};
use rmcp::handler::server::wrapper::Parameters;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct SearchParams {
    query: String,
    limit: Option<usize>,
}

#[derive(Clone)]
struct MyTools;

#[tool_router(server_handler)]
impl MyTools {
    #[tool(description = "Search the knowledge base")]
    async fn search(&self, Parameters(params): Parameters<SearchParams>) -> String {
        // Implementation here
        format!("Results for: {}", params.query)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = MyTools.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
```

### Key RMCP Features

- **Macro-based tool definitions**: `#[tool]`, `#[tool_router]`, `#[tool_handler]`
- **Prompt macros**: `#[prompt]`, `#[prompt_router]`, `#[prompt_handler]`
- **Resource handling**: `list_resources()`, `read_resource()`
- **Sampling**: Server can request LLM completions from the client
- **Completions**: Auto-completion suggestions for prompt/resource arguments
- **Progress notifications**: Report progress during long operations
- **Cancellation**: Either side can cancel in-progress requests
- **Subscriptions**: Subscribe to resource change notifications
- **OAuth support**: For authenticated remote servers

## Integration Plan for Multae Viae

### Phase 1: MCP Client

The controller acts as an MCP **Host** with multiple clients:

```rust
// Conceptual configuration
struct McpConfig {
    servers: Vec<McpServerConfig>,
}

struct McpServerConfig {
    name: String,
    transport: McpTransport,
}

enum McpTransport {
    Stdio { command: String, args: Vec<String> },
    Http { url: String, auth: Option<AuthConfig> },
}
```

### Phase 2: RAG as MCP Server

The RAG service on the local network exposes itself as an MCP server:

- **Tools**: `search_documents`, `ingest_document`, `list_collections`
- **Resources**: Retrieved document chunks
- **Transport**: Streamable HTTP (since it's on a different machine)

### Phase 3: Controller as MCP Server

Expose the controller itself as an MCP server, allowing other AI tools
(VS Code Copilot, Claude Desktop) to use your controller's capabilities:

- **Tools**: Run workflows, query system state, execute tasks
- **Resources**: Workflow results, agent memory, context

### Existing MCP Servers to Leverage

Many pre-built MCP servers exist that can be connected immediately:

| Server | Capability | Transport |
|--------|-----------|-----------|
| filesystem | File operations | stdio |
| git | Repository operations | stdio |
| sqlite | Database queries | stdio |
| brave-search | Web search | stdio |
| memory | Persistent memory | stdio |
| puppeteer | Web automation | stdio |

See [github.com/modelcontextprotocol/servers](https://github.com/modelcontextprotocol/servers)
for the full list of reference implementations.
