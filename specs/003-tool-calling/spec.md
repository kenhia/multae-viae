# Feature Specification: Tool Calling & Agentic Loop

**Feature Branch**: `003-tool-calling`  
**Created**: 2026-04-29  
**Status**: Draft  
**Input**: Phase 2 from docs/09-roadmap.md — Agent can call tools and use results in responses.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Ask a Question That Requires a Tool (Priority: P1)

A user asks the CLI a question whose answer requires interacting with the local environment — for example, listing files in a directory or reading a file's contents. The agent recognises that it needs a tool, invokes the appropriate tool, receives the result, and incorporates it into a natural-language response.

**Why this priority**: This is the core value proposition of the feature. Without the model being able to call tools and loop on results, there is no agentic behaviour.

**Independent Test**: Run `mv-cli "What files are in the current directory?"` and verify the response accurately reflects the actual directory contents.

**Acceptance Scenarios**:

1. **Given** the agent has a `file_list` tool registered, **When** the user asks "What files are in the current directory?", **Then** the agent invokes `file_list`, receives a directory listing, and responds with a human-readable summary of the files.
2. **Given** the agent has a `file_read` tool registered, **When** the user asks "What does README.md say?", **Then** the agent invokes `file_read` with path `README.md`, receives file contents, and summarises them.
3. **Given** the model decides no tool is needed, **When** the user asks a general knowledge question like "What is Rust?", **Then** the agent responds directly without invoking any tool.

---

### User Story 2 - Execute a Shell Command via Tool (Priority: P2)

A user asks the CLI to perform a task that requires running a shell command — for example, checking the current git branch or counting lines in a file. The agent invokes a `shell_exec` tool, receives the output, and presents the results.

**Why this priority**: Shell execution dramatically expands the agent's capabilities beyond read-only file operations, enabling real development workflows.

**Independent Test**: Run `mv-cli "What git branch am I on?"` and verify the response contains the actual current branch name.

**Acceptance Scenarios**:

1. **Given** the agent has a `shell_exec` tool registered, **When** the user asks "What git branch am I on?", **Then** the agent invokes `shell_exec` with `git branch --show-current`, and responds with the branch name.
2. **Given** the user asks something that could be answered by a shell command, **When** the agent invokes `shell_exec`, **Then** the command runs with a timeout and the agent handles both success and failure outputs gracefully.

---

### User Story 3 - Fetch Information from a URL (Priority: P3)

A user asks the CLI to retrieve information from a URL. The agent invokes an `http_get` tool, receives the response body, and summarises or presents the content.

**Why this priority**: HTTP access extends the agent beyond local-only operations, but is less critical than file and shell access for the initial agentic use case.

**Independent Test**: Run `mv-cli "What is the title of https://example.com?"` and verify the response references the page title.

**Acceptance Scenarios**:

1. **Given** the agent has an `http_get` tool registered, **When** the user asks about a URL, **Then** the agent invokes `http_get` with the URL and incorporates the response body into its answer.
2. **Given** the target URL returns an error (404, timeout), **When** the agent invokes `http_get`, **Then** it reports the failure to the user in a clear message rather than crashing.

---

### Edge Cases

- What happens when the model hallucinates a tool name that does not exist in the registry?
- What happens when a tool invocation times out or returns an error?
- What happens when the model enters an infinite tool-calling loop (e.g., repeatedly calling the same tool)?
- What happens when the model returns a tool call with malformed arguments?
- What happens when `shell_exec` is invoked with a command that produces very large output?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a tool registry that holds a set of named tools, each with a description, parameter schema, and an invocation handler.
- **FR-002**: System MUST ship with at least four built-in tools: `file_list` (list directory contents), `file_read` (read a file), `shell_exec` (run a shell command), and `http_get` (fetch a URL via HTTP GET).
- **FR-003**: System MUST integrate with the model's tool-calling capability so the model can decide which tool to invoke and with what arguments.
- **FR-004**: System MUST implement an agentic loop: send prompt to model → if model requests a tool call, execute the tool → feed tool result back to the model → repeat until the model produces a final text response.
- **FR-005**: System MUST enforce a maximum iteration limit on the agentic loop to prevent runaway tool-calling cycles.
- **FR-006**: System MUST validate tool call arguments against the tool's parameter schema before invocation.
- **FR-007**: System MUST format tool results and inject them back into the conversation context for the model.
- **FR-008**: System MUST instrument each tool call in telemetry, recording tool name, arguments, result status, and duration as span attributes.
- **FR-009**: System MUST handle tool invocation errors gracefully — feeding the error back to the model as context rather than terminating the session.
- **FR-010**: System MUST enforce a timeout on `shell_exec` invocations to prevent hanging commands.
- **FR-011**: System MUST truncate tool results that exceed a defined maximum length before injecting them into the model context.
- **FR-012**: System MUST reject tool calls to tool names not present in the registry, feeding a clear error message back to the model.

### Key Entities

- **Tool**: A named capability with a description, parameter schema, and invocation handler. Identified by a unique string name.
- **ToolCall**: A request from the model to invoke a specific tool with specific arguments. Contains tool name and a map of argument values.
- **ToolResult**: The outcome of a tool invocation — either a success with string output, or a failure with an error message.
- **AgenticLoop**: The iterative cycle of model prompt → tool call → tool result → model prompt, bounded by a maximum iteration count.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can ask questions requiring local environment interaction and receive accurate answers that reflect actual system state.
- **SC-002**: The agentic loop completes within 10 iterations for typical single-tool queries.
- **SC-003**: Tool call failures (timeouts, errors, unknown tools) are handled without crashing — the user always receives a coherent response.
- **SC-004**: Every tool invocation is visible in telemetry traces with tool name, duration, and outcome, viewable in Jaeger.
- **SC-005**: Shell commands complete or time out within a bounded period, defaulting to 30 seconds.
- **SC-006**: Queries that do not require tools continue to work as before with no regression in response quality or latency.

## Assumptions

- The models used (Ollama, OpenAI) support tool/function calling in their APIs, and Rig exposes this capability.
- `shell_exec` runs commands in the user's default shell; no sandboxing is required for this phase (security hardening is a future concern).
- Tool result truncation uses a simple character limit; smarter summarisation is out of scope.
- The `http_get` tool performs a simple GET request; authentication, headers, and POST are out of scope for this phase.
- The agentic loop maximum iteration default is 10; this is configurable but not exposed as a CLI flag in this phase.
- Built-in tools are compiled into the binary; dynamic tool loading is deferred to Phase 3 (MCP Integration).
