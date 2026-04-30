# Quickstart: MCP Integration

**Feature**: 004-mcp-integration  
**Date**: 2026-04-30

## Prerequisites

- Rust toolchain (edition 2024)
- Ollama running locally with a model pulled (e.g., `qwen3:4b`)
- Node.js/npm installed (for reference MCP servers)

## 1. Build the project

```bash
cargo build -p mv-cli
```

## 2. Create an MCP servers configuration

Create `mcp-servers.yaml` in the project root:

```yaml
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
```

## 3. Run with MCP tools

```bash
# The CLI auto-discovers mcp-servers.yaml in the current directory
cargo run -p mv-cli -- "List the directories in /tmp"

# Verbose output to see MCP connection details
cargo run -p mv-cli -- -v "List the directories in /tmp"
```

The agent will connect to the filesystem MCP server, discover its tools, and use them to answer the question.

## 4. Verify in telemetry (optional)

If OTLP export is enabled:

```bash
# Start Jaeger
docker run -d --name jaeger \
  -p 16686:16686 \
  -p 4318:4318 \
  jaegertracing/all-in-one:latest

# Run with OTLP export
cargo run -p mv-cli -- --otlp "List the directories in /tmp"

# View traces at http://localhost:16686
```

MCP tool calls appear as spans with `mcp.server.name` and `mcp.transport` attributes.

## 5. Add an HTTP MCP server (optional)

To connect to a remote MCP server, add an HTTP entry:

```yaml
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]

  - name: remote-tools
    transport: http
    url: http://192.168.1.100:8080/mcp
```
