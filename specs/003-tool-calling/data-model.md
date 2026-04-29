# Data Model: Tool Calling & Agentic Loop

**Feature**: 003-tool-calling
**Date**: 2026-04-29

## Entities

### Tool (via `rig::tool::Tool` trait)

Each tool is a Rust struct implementing the `rig::tool::Tool` trait (or generated via `#[rig_tool]` macro).

| Attribute | Type | Description |
|-----------|------|-------------|
| NAME | `&'static str` | Unique tool identifier (e.g., `"file_list"`) |
| description | `String` | Human-readable description for the model |
| parameters | `serde_json::Value` | JSON Schema of accepted arguments |

**Relationships**: Registered with `AgentBuilder` via `.tool()`. Invoked by the model during multi-turn loop.

### FileListArgs

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| path | `String` | No (default: `"."`) | Directory path to list |

**Validation**: Path must exist and be a directory.

### FileReadArgs

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| path | `String` | Yes | File path to read |

**Validation**: Path must exist and be a regular file.

### ShellExecArgs

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| command | `String` | Yes | Shell command to execute |

**Validation**: Command must be non-empty.

### HttpGetArgs

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| url | `String` | Yes | URL to fetch via GET |

**Validation**: URL must parse as a valid URL.

### ToolResult (implicit — tool return values)

Tool functions return `Result<String, ToolError>`:
- **Success**: String output (directory listing, file contents, command output, HTTP response body)
- **Error**: `ToolError::ToolCallError(String)` with descriptive message

### Truncation Constants

| Constant | Value | Description |
|----------|-------|-------------|
| MAX_TOOL_OUTPUT_CHARS | 10,000 | Maximum characters before truncation |
| SHELL_TIMEOUT_SECS | 30 | Shell command execution timeout |
| HTTP_TIMEOUT_SECS | 30 | HTTP request timeout |

## State Transitions

### Agentic Loop (managed by Rig internally)

```
[User Prompt]
    ↓
[Model Receives Prompt + Tool Definitions]
    ↓
[Model Response]
    ├── Text Response → [Done: return to user]
    └── Tool Call(s) → [Execute Tool(s)]
                            ↓
                       [Tool Result(s)]
                            ↓
                       [Feed Results Back to Model]
                            ↓
                       [Model Response] (loop, max N turns)
```

**Loop bound**: `default_max_turns(10)` — if model still calling tools after 10 turns, Rig returns last response.

## Entity Relationships

```
AgentBuilder
    ├── .tool(FileList)
    ├── .tool(FileRead)
    ├── .tool(ShellExec)
    ├── .tool(HttpGet)
    ├── .preamble(system_prompt)
    └── .default_max_turns(10)
         ↓
     Agent<M>
         │
         ├── .prompt("user question").await
         │       ↓
         │   [Internal multi-turn loop]
         │       ├── Model decides tool call → Tool::call() → result fed back
         │       └── Model decides text → return final response
         │
         └── Telemetry: each tool call emits a tracing span
```

## MvError Extensions

No new `MvError` variants needed for this phase. Tool errors are handled within the Rig agent loop via `ToolError`. The existing `CompletionFailed` variant covers agent-level failures.
