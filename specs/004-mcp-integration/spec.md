# Feature Specification: MCP Integration

**Feature Branch**: `004-mcp-integration`  
**Created**: 2026-04-30  
**Status**: Draft  
**Input**: User description: "Phase 3 from docs/09-roadmap.md — Connect to MCP servers for external tool access."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Connect to a Local MCP Server and Use Its Tools (Priority: P1)

A user configures a local MCP server (e.g., the reference filesystem server) in a YAML configuration file. When the user starts the CLI and asks a question that requires a tool provided by that server, the system automatically spawns the MCP server process, discovers its available tools, and invokes them as part of the agentic loop — just as it would with built-in tools.

**Why this priority**: This is the foundational capability of MCP integration. Without stdio-based server connectivity and tool discovery, no other MCP feature is possible.

**Independent Test**: Configure the reference filesystem MCP server in the YAML config, run `mv-cli "List the files in /tmp"`, and verify the agent uses the MCP filesystem server's tool to answer accurately.

**Acceptance Scenarios**:

1. **Given** an MCP server is configured in the servers YAML file with a stdio transport, **When** the CLI starts, **Then** the system spawns the server process and completes the MCP protocol handshake.
2. **Given** a connected MCP server exposes a `list_directory` tool, **When** the user asks a question that requires listing directory contents, **Then** the agent discovers and invokes the MCP tool and includes the result in its response.
3. **Given** a configured MCP server fails to start (e.g., binary not found), **When** the CLI starts, **Then** the system logs a warning and continues operating with remaining tools (built-in and other servers).

---

### User Story 2 - MCP Tools Appear Alongside Built-in Tools (Priority: P2)

A user has both built-in tools (file_list, shell_exec, etc.) and MCP server tools available. The model sees all tools — built-in and MCP-provided — as a unified set and can choose whichever is most appropriate for the query. The user does not need to know or specify whether a tool is built-in or comes from an MCP server.

**Why this priority**: Seamless integration into the existing tool registry is essential for a coherent user experience. Without it, MCP tools would be isolated from the agentic loop.

**Independent Test**: Configure an MCP server that provides a tool with a unique capability not covered by built-in tools, ask a question requiring that capability, and verify the agent selects and invokes the MCP tool.

**Acceptance Scenarios**:

1. **Given** both built-in tools and MCP server tools are available, **When** the model receives the tool list, **Then** it sees a single unified list containing all tools with their descriptions and parameter schemas.
2. **Given** an MCP tool has the same name as a built-in tool, **When** the system merges tools, **Then** the conflict is resolved deterministically (built-in tools take precedence) and a warning is logged.
3. **Given** multiple MCP servers expose tools with the same name, **When** tools are merged, **Then** the duplicates are namespaced as `server_name.tool_name` to avoid collisions, and a warning is logged.

---

### User Story 3 - Connect to a Remote MCP Server over HTTP (Priority: P3)

A user configures a remote MCP server accessible over HTTP (e.g., a RAG service running on the local network). The system connects to the remote server, discovers its tools, and makes them available in the same unified tool set.

**Why this priority**: HTTP transport extends MCP beyond local-only servers, enabling network services like RAG or shared databases. It is lower priority because stdio covers the most common local development use case.

**Independent Test**: Start a reference MCP server on a local HTTP endpoint, configure it in the YAML config, and verify the agent can discover and invoke its tools.

**Acceptance Scenarios**:

1. **Given** an MCP server is configured with an HTTP transport URL, **When** the CLI starts, **Then** the system connects to the remote server and completes the MCP protocol handshake.
2. **Given** a remote MCP server is unreachable (network error, timeout), **When** the CLI starts, **Then** the system logs a warning and continues operating with remaining tools.
3. **Given** a remote MCP server becomes unavailable mid-session, **When** the agent attempts to invoke one of its tools, **Then** the system returns a clear error to the model and the model can fall back to other tools or inform the user.

---

### User Story 4 - MCP Tool Calls Appear in Telemetry (Priority: P4)

A developer or operator inspects traces in Jaeger after a session that involved MCP tool calls. Each MCP tool invocation appears as a span with attributes identifying the server name, tool name, transport type, arguments, result status, and duration.

**Why this priority**: Observability of MCP calls is essential for debugging and performance analysis, but it builds on the existing telemetry infrastructure and can be layered on after core connectivity works.

**Independent Test**: Run a query that triggers an MCP tool call, then verify in Jaeger that a span exists with the expected MCP-specific attributes.

**Acceptance Scenarios**:

1. **Given** an MCP tool call completes successfully, **When** the user views the trace in Jaeger, **Then** the span includes attributes for server name, tool name, transport type, and duration.
2. **Given** an MCP tool call fails, **When** the user views the trace, **Then** the span records the error status and error message.

---

### Edge Cases

- What happens when an MCP server is configured but the executable is not installed on the system?
- What happens when an MCP server crashes mid-session after tools have been discovered?
- What happens when an MCP server returns a tool list with tools that have invalid or empty parameter schemas?
- What happens when two MCP servers expose tools with the same name?
- What happens when the MCP protocol handshake times out?
- What happens when an MCP tool call returns extremely large output?
- What happens when the server configuration YAML file is missing or contains invalid syntax?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST support configuring MCP servers in a YAML file, specifying server name, transport type, and transport-specific parameters.
- **FR-002**: System MUST support the stdio transport — spawning a local process and communicating over stdin/stdout using the MCP protocol.
- **FR-003**: System MUST support the HTTP transport — connecting to a remote server via HTTP POST and Server-Sent Events (Streamable HTTP).
- **FR-004**: System MUST perform the MCP protocol handshake (initialize/initialized) with each configured server at startup.
- **FR-005**: System MUST discover available tools from each connected MCP server by calling `tools/list`.
- **FR-006**: System MUST merge MCP-discovered tools into the existing tool registry so the model sees a single unified tool set.
- **FR-007**: System MUST handle tool name collisions when merging — built-in tools take precedence over MCP tools, and MCP tools from different servers are namespaced by server name.
- **FR-008**: System MUST invoke MCP tools by sending `tools/call` requests to the appropriate server when the model selects an MCP tool.
- **FR-009**: System MUST convert MCP tool results into the existing ToolResult format for injection back into the agentic loop.
- **FR-010**: System MUST handle MCP server startup failures gracefully — logging a warning and continuing with available tools.
- **FR-011**: System MUST handle MCP server crashes or disconnections mid-session — marking the server's tools as unavailable and reporting errors to the model.
- **FR-012**: System MUST enforce a configurable timeout on the MCP protocol handshake, defaulting to 5 seconds. The timeout MAY be overridden via a field in the server YAML configuration.
- **FR-013**: System MUST enforce the existing tool output truncation on MCP tool results before injecting them into model context.
- **FR-014**: System MUST instrument MCP tool calls in telemetry, recording server name, tool name, transport type, arguments, result status, and duration as span attributes.
- **FR-015**: System MUST cleanly shut down MCP server connections when the CLI session ends — sending protocol shutdown and terminating spawned processes.
- **FR-016**: System MUST support configuring environment variables to pass to stdio-based MCP server processes.

### Key Entities

- **McpServerConfig**: A configured MCP server with a name, transport type (stdio or HTTP), and transport-specific parameters (command/args for stdio; URL for HTTP).
- **McpClient**: A connection to a single MCP server, managing protocol lifecycle (handshake, tool discovery, tool invocation, shutdown).
- **McpTransport**: The communication mechanism — either stdio (child process stdin/stdout) or HTTP (Streamable HTTP with SSE).
- **McpTool**: A tool discovered from an MCP server, containing name, description, and input schema. Adapted into the existing tool registry format.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can configure MCP servers in a YAML file and the system connects to them automatically at startup.
- **SC-002**: MCP tools are usable in the agentic loop identically to built-in tools — users cannot distinguish the source of a tool from the interaction.
- **SC-003**: The system successfully connects to both stdio-based and HTTP-based MCP servers.
- **SC-004**: Server startup failures and mid-session disconnections are handled without crashing — the user always receives a coherent response.
- **SC-005**: Every MCP tool invocation is visible in telemetry traces with server name, tool name, transport type, and duration.
- **SC-006**: Existing built-in tools continue to function with no regression when MCP servers are configured.
- **SC-007**: The system connects to reference MCP servers (e.g., filesystem, git) without custom adaptation.

## Assumptions

- The `rmcp` crate (v1.5+) is used as the MCP client library, providing protocol handling, transport abstractions, and tool discovery.
- MCP servers conform to the MCP specification; the system does not need to handle non-compliant servers beyond basic error reporting.
- Authentication for HTTP-based MCP servers is out of scope for this phase; only unauthenticated HTTP connections are supported.
- MCP Resources and Prompts primitives are out of scope for this phase; only the Tools primitive is integrated.
- Sampling (server-initiated LLM requests) is out of scope for this phase.
- The server configuration file follows the same YAML conventions as the existing `models.yaml` configuration.
- Stdio-based MCP servers are expected to be installed and available on the user's PATH or specified with absolute paths.
- The existing tool output truncation limit (10,000 characters) applies equally to MCP tool results.
