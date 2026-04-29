# Research: Tool Calling & Agentic Loop

**Feature**: 003-tool-calling
**Date**: 2026-04-29

## R1: Rig Tool Calling API

**Question**: How does rig-core 0.35 support tool calling, and what does the integration surface look like?

**Decision**: Use Rig's built-in `Tool` trait and `#[rig_tool]` macro with the agent's built-in multi-turn agentic loop.

**Rationale**: Rig 0.35 has native tool-calling support at every level:
- `rig::tool::Tool` trait: `NAME`, `Args` (Deserialize), `Output` (Serialize), `Error`, with `definition()` and `call()` methods
- `#[rig_tool]` attribute macro (requires `derive` feature): transforms a plain function into a `Tool` impl automatically, generating args struct and definition from function signature and doc comments
- `AgentBuilder::tool(impl Tool)` / `.tools(Vec<Box<dyn ToolDyn>>)`: adds tools to agent
- `AgentBuilder::default_max_turns(usize)`: configures max multi-turn iterations
- The agentic loop is **internal to Rig** — calling `.prompt().await` on a tool-equipped agent triggers: model → tool call → tool result → model → ... → final text
- `PromptRequest::max_turns(N)` allows per-request override of max iterations
- `rig::tool::ToolError` for tool execution errors
- `rig::tool::ToolSet` / `ToolSetBuilder` for managing tool collections

**Alternatives considered**:
- Manual agentic loop (call model, parse tool calls, execute, resubmit): Rejected — Rig handles this internally, writing our own would duplicate effort and miss Rig's error handling
- Use `rig::tool::ToolEmbedding` for dynamic tool selection: Deferred — overkill for 3 static tools; relevant for Phase 3 MCP with many tools

## R2: `#[rig_tool]` vs Manual `Tool` impl

**Question**: Should tools be defined with the `#[rig_tool]` macro or manual `Tool` trait implementations?

**Decision**: Use `#[rig_tool]` macro for all built-in tools.

**Rationale**:
- The macro generates the `Args` struct, `ToolDefinition`, and `Tool` impl from the function signature
- Parameter descriptions are specified via `params(name = "description")` in the attribute
- Tool description via `description = "..."` in the attribute
- Error type is `rig::tool::ToolError` (via `ToolCallError` variant for custom errors)
- Much less boilerplate than manual impl

**Requirements**:
- Add `derive` feature to `rig-core` dependency: `rig-core = { version = "0.35", features = ["derive"] }`

## R3: Agentic Loop Control

**Question**: How to control the agentic loop depth and handle runaway tool calling?

**Decision**: Use `AgentBuilder::default_max_turns(10)` for the default, no CLI flag in this phase.

**Rationale**:
- Rig's `default_max_turns` directly maps to our FR-005 (max iteration limit)
- Default of 10 is reasonable for typical single-tool queries (SC-002)
- Per-request override available via `.prompt(msg).max_turns(N).await` for future flexibility
- If model exceeds max turns, Rig returns the last response — no crash, graceful degradation

## R4: Tool Error Handling

**Question**: How should tool errors be propagated back to the model?

**Decision**: Return `ToolError::ToolCallError(String)` from tool functions; Rig feeds error text back to the model automatically.

**Rationale**:
- `rig::tool::ToolError` has a `ToolCallError(String)` variant for custom error messages
- When a tool returns `Err`, Rig feeds the error message back to the model as a tool result, allowing the model to retry or explain the failure
- This satisfies FR-009 (graceful error handling) and FR-012 (unknown tool rejection) without custom loop logic

## R5: Shell Execution Safety

**Question**: How to implement `shell_exec` with timeout and output truncation?

**Decision**: Use `tokio::process::Command` with `tokio::time::timeout` for bounded execution, and truncate stdout/stderr to a configurable max length.

**Rationale**:
- `tokio::process::Command` integrates with the async runtime already in use
- `tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())` provides clean timeout
- Output truncation: if stdout + stderr > max chars, truncate with `...[truncated]` suffix
- Default timeout: 30 seconds (SC-005)
- Default max output: 10,000 characters (FR-011)

**Alternatives considered**:
- `std::process::Command`: Blocking, would need `spawn_blocking` wrapper — less clean than native async
- No timeout: Rejected — hanging commands would block the agent indefinitely

## R6: HTTP GET Tool

**Question**: What HTTP client to use for the `http_get` tool?

**Decision**: Use `reqwest` (already a transitive dependency via rig-core/opentelemetry-otlp).

**Rationale**:
- `reqwest` is already in the dependency tree (rig-core uses it internally)
- Adding `reqwest` as a direct dependency adds no new compilation cost
- Simple GET with timeout: `reqwest::Client::new().get(url).timeout(Duration::from_secs(30)).send().await`
- Response body truncation: same approach as shell_exec (max chars)

## R7: Telemetry for Tool Calls

**Question**: How to instrument tool calls in OpenTelemetry traces?

**Decision**: Wrap each tool's `call()` with `#[tracing::instrument]` and add semantic convention span attributes.

**Rationale**:
- The `#[rig_tool]` macro generates the `call()` method — we can add `#[tracing::instrument]` to the tool function itself
- Span attributes: `tool.name`, `tool.status` (success/error), duration is automatic from span
- This composes with the existing OTel layer from sprint 002
- FR-008 requires: tool name, arguments, result status, duration — all achievable with tracing spans

## R8: Ollama Tool Calling Support

**Question**: Does Ollama support tool/function calling?

**Decision**: Yes — Ollama supports OpenAI-compatible tool calling for models that support it (e.g., qwen3, llama3.1+, mistral).

**Rationale**:
- Ollama's `/api/chat` endpoint accepts `tools` parameter in OpenAI-compatible format since Ollama 0.3.0+
- Rig's Ollama provider passes tool definitions through to the API
- Qwen3 models (our defaults) support tool calling
- No special configuration needed — Rig handles the format translation
