# Data Model: MCP Integration

**Feature**: 004-mcp-integration  
**Date**: 2026-04-30

## Entities

### McpServerConfig

A configured MCP server entry parsed from `mcp-servers.yaml`.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| name | String | Yes | Unique identifier for the server |
| transport | McpTransportType | Yes | "stdio" or "http" |
| command | String | If stdio | Executable to spawn |
| args | Vec\<String\> | No | Arguments for the command |
| env | Map\<String, String\> | No | Environment variables for the child process |
| url | String | If http | URL for the HTTP endpoint |

**Validation rules**:
- `name` must be non-empty and unique across all configured servers
- If `transport` is "stdio", `command` must be present
- If `transport` is "http", `url` must be present and a valid URL
- `env` is only applicable for stdio transport

### McpTransportType

The communication mechanism for connecting to an MCP server.

| Variant | Description |
|---------|-------------|
| Stdio | Child process communicating over stdin/stdout |
| Http | Remote server communicating over Streamable HTTP |

### McpServersConfig

YAML wrapper for the server configuration file.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| servers | Vec\<McpServerConfig\> | Yes | List of MCP server configurations |

**Validation rules**:
- `servers` must contain at least one entry (warn if empty, do not error)
- Duplicate server names are rejected with an error

### McpClient

A runtime connection to a single MCP server. Not persisted — exists only for the duration of a CLI session.

| Field | Type | Description |
|-------|------|-------------|
| name | String | Server name from config |
| transport_type | McpTransportType | How the connection was established |
| service | RunningService | The active rmcp service handle |
| tools | Vec\<McpTool\> | Tools discovered from this server |
| status | McpClientStatus | Current connection status |

**State transitions**:
- `Connecting` → `Connected` (handshake success)
- `Connecting` → `Failed` (handshake timeout or error)
- `Connected` → `Disconnected` (mid-session crash/error)

### McpClientStatus

| Variant | Description |
|---------|-------------|
| Connecting | Handshake in progress |
| Connected | Active and available for tool calls |
| Failed | Could not connect (startup error) |
| Disconnected | Was connected, lost connection mid-session |

### McpTool

A tool discovered from an MCP server, adapted for use in the Rig tool system.

| Field | Type | Description |
|-------|------|-------------|
| name | String | Tool name as reported by the server |
| qualified_name | String | Namespaced name (server_name.tool_name) for collision resolution |
| description | String | Tool description for the model |
| input_schema | serde_json::Value | JSON Schema for tool parameters |
| server_name | String | Which server provides this tool |

### McpToolAdapter

Wrapper that implements Rig's `Tool` trait for an MCP-discovered tool. Not a data entity — it is a runtime adapter that bridges MCP tool calls to the rmcp client.

| Field | Type | Description |
|-------|------|-------------|
| tool | McpTool | The discovered tool metadata |
| client | Arc\<RunningService\> | Shared reference to the rmcp service for invocation |

## Relationships

```text
McpServersConfig (YAML file)
  └── has many → McpServerConfig
                    │
                    ▼ (at runtime, spawns)
                 McpClient
                    ├── has one → McpClientStatus
                    └── has many → McpTool
                                    │
                                    ▼ (wrapped by)
                                 McpToolAdapter → registered in Rig agent
```

## Existing Entities (unchanged)

These entities from prior phases are referenced but not modified:

- **ModelEntry** / **ModelRegistry**: Model configuration (from `models.yaml`). MCP config is separate.
- **Tool trait** (Rig): MCP tools are adapted to this trait via McpToolAdapter.
- **ToolResult** (Rig): MCP tool call results are converted to this format.
