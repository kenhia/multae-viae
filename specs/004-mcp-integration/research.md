# Research: MCP Integration

**Feature**: 004-mcp-integration  
**Date**: 2026-04-30

## R1: rmcp Crate Client Usage

**Task**: Research rmcp client API for connecting to MCP servers and invoking tools.

**Decision**: Use `rmcp` v1.5.0 with the `client` feature flag. The crate provides `ServiceExt` for building client connections over stdio and HTTP transports.

**Rationale**: rmcp is the official Rust MCP SDK maintained by the MCP project. It provides protocol-compliant client/server implementations with async Tokio support, matching the project's existing runtime. The `client` feature provides `list_all_tools()` and `call_tool()` methods.

**Alternatives considered**:
- Manual JSON-RPC implementation over stdio/HTTP — rejected due to protocol complexity (handshake, capabilities negotiation, message framing) and maintenance burden
- `mcp-client` (unofficial) — less maintained, fewer features, not specification-aligned

**Key API patterns**:

```rust
// Stdio transport — spawn a child process
use rmcp::transport::child_process::TokioChildProcess;
use tokio::process::Command;

let transport = TokioChildProcess::new(
    Command::new("npx").args(["-y", "@modelcontextprotocol/server-filesystem", "/tmp"])
)?;
let client = ().serve(transport).await?;
let tools = client.list_all_tools().await?;
let result = client.call_tool(CallToolRequestParams::new("list_directory")).await?;

// HTTP (Streamable HTTP) transport
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;

let transport = StreamableHttpClientTransport::new("http://localhost:8080/mcp");
let client = ().serve(transport).await?;
```

## R2: Tool Registry Merging Strategy

**Task**: Research how to merge MCP-discovered tools into Rig's tool-calling system.

**Decision**: Create wrapper types that adapt MCP tools into Rig's `Tool` trait, then register them on the agent builder alongside built-in tools.

**Rationale**: Rig's agent builder accepts any type implementing the `Tool` trait via `.tool()`. MCP tools have name, description, and JSON Schema for parameters — these map directly to Rig's `ToolDefinition`. The wrapper translates between Rig's tool call format and rmcp's `call_tool` API.

**Alternatives considered**:
- Replace built-in tools with MCP equivalents — rejected because it would require running MCP servers for basic functionality and breaks offline usage
- Separate tool routing layer — rejected as over-engineering; Rig already handles tool selection via the model's tool-calling capability

**Key design**:

```rust
// McpToolAdapter wraps an MCP tool to implement Rig's Tool trait
struct McpToolAdapter {
    server_name: String,
    tool_name: String,
    description: String,
    input_schema: serde_json::Value,
    client: Arc<McpClient>,  // shared reference to the rmcp service
}
```

**Namespacing**: MCP tools are registered with their original names. If a collision occurs with a built-in tool, the built-in takes precedence and a warning is logged. If two MCP servers expose the same tool name, they are namespaced as `server_name.tool_name`.

## R3: MCP Server Configuration Format

**Task**: Research configuration format for declaring MCP servers.

**Decision**: Use a dedicated `mcp-servers.yaml` file at the project root, following the same YAML conventions as `models.yaml`.

**Rationale**: Separate file keeps concerns isolated (model config vs. server config). YAML is already the project's configuration language. The format mirrors the VS Code MCP configuration structure adapted to YAML.

**Alternatives considered**:
- Embed in `models.yaml` — rejected because MCP servers are not models; mixing concerns makes the config harder to understand
- TOML or JSON — rejected because the project already uses YAML for all configuration
- CLI flags for server paths — rejected because multiple servers with complex arguments need structured config

**Config format**:

```yaml
# mcp-servers.yaml
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    env:
      NODE_ENV: production

  - name: rag
    transport: http
    url: http://192.168.1.100:8080/mcp
```

## R4: Transport Mechanisms

**Task**: Research stdio and HTTP transport implementation details.

**Decision**: Support both stdio (child process) and Streamable HTTP transports using rmcp's built-in transport abstractions.

**Rationale**: stdio is the primary transport for local MCP servers (filesystem, git, database tools). HTTP (Streamable HTTP with SSE) enables remote servers on the network (e.g., RAG service). rmcp provides both transports out of the box.

**Alternatives considered**:
- stdio only — rejected because the roadmap explicitly requires HTTP for network services (RAG in Phase 5)
- WebSocket — not part of the MCP specification; Streamable HTTP is the standard remote transport

**Implementation notes**:
- Stdio: spawn child process via `tokio::process::Command`, connect via `TokioChildProcess` transport
- HTTP: connect via `StreamableHttpClientTransport` with configurable URL
- Both transports handle MCP protocol framing (JSON-RPC over the respective channel)
- Environment variables for stdio processes are passed via `Command::envs()`

## R5: Error Handling and Graceful Degradation

**Task**: Research error handling patterns for MCP server failures.

**Decision**: All MCP server errors are non-fatal at the session level. Failed servers are logged and skipped; the session continues with remaining tools.

**Rationale**: Constitution Principle VII (Simplicity) requires defensive coding at system boundaries. MCP servers are external processes/services — they can crash, timeout, or be misconfigured. The agent must remain functional with whatever tools are available.

**Error categories**:
1. **Startup failure** (binary not found, handshake timeout): Log warning, skip server, continue with remaining tools
2. **Mid-session crash** (stdio pipe closed, HTTP connection lost): Mark server's tools as unavailable, return error to model on next invocation attempt
3. **Tool call failure** (server returns error): Convert to ToolResult error, feed back to model for retry or fallback
4. **Config error** (invalid YAML, missing fields): Report at startup with actionable error message, skip invalid entries

## R6: Telemetry Integration

**Task**: Research how to instrument MCP tool calls in the existing OpenTelemetry setup.

**Decision**: Wrap each MCP tool invocation in a tracing span with MCP-specific attributes, using the same `#[tracing::instrument]` pattern as built-in tools.

**Rationale**: The project already uses `tracing` + `tracing-opentelemetry` for all instrumentation. MCP tool calls should appear in the same trace as built-in tool calls, with additional attributes identifying the MCP server and transport.

**Span attributes**:
- `tool.name`: the MCP tool name
- `mcp.server.name`: the configured server name
- `mcp.transport`: "stdio" or "http"
- `tool.result.status`: "ok" or "error"
- Standard span duration is captured automatically
