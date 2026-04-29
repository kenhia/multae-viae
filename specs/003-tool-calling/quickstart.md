# Quickstart: Tool Calling & Agentic Loop

**Feature**: 003-tool-calling
**Date**: 2026-04-29

## Prerequisites

- Rust stable v1.95.0+ (edition 2024)
- Ollama running locally with a tool-calling model (qwen3:4b or qwen3:8b)
- Workspace built: `cargo build -p mv-cli`

## Try It

### File listing

```bash
cargo run -p mv-cli -- "What files are in the current directory?"
```

The agent calls the `file_list` tool, receives the directory listing, and responds with a summary.

### Read a file

```bash
cargo run -p mv-cli -- "What does the README say?"
```

The agent calls `file_read` with `README.md` and summarises the contents.

### Shell command

```bash
cargo run -p mv-cli -- "What git branch am I on?"
```

The agent calls `shell_exec` with `git branch --show-current` and responds with the branch name.

### HTTP fetch

```bash
cargo run -p mv-cli -- "What is the title of https://example.com?"
```

The agent calls `http_get`, fetches the page, and extracts the title.

### No tool needed

```bash
cargo run -p mv-cli -- "What is Rust?"
```

The agent responds directly without calling any tools (same as before).

## With Telemetry

```bash
# Start Jaeger
docker run -d --name jaeger \
  -p 16686:16686 -p 4318:4318 \
  jaegertracing/all-in-one:latest

# Ask a question that triggers tool use
cargo run -p mv-cli -- --otlp "What files are in the current directory?"

# View traces at http://localhost:16686
# Look for service "mv-cli" — you'll see tool call spans nested under the model call
```

## Development

### Add the `derive` feature

```toml
# crates/mv-core/Cargo.toml
rig-core = { version = "0.35", features = ["derive"] }
```

### Define a tool with `#[rig_tool]`

```rust
use rig::tool::ToolError;
use rig_derive::rig_tool;

#[rig_tool(
    description = "List the contents of a directory",
    params(path = "Directory path to list (default: current directory)")
)]
fn file_list(path: Option<String>) -> Result<String, ToolError> {
    let dir = path.unwrap_or_else(|| ".".to_string());
    let entries = std::fs::read_dir(&dir)
        .map_err(|e| ToolError::ToolCallError(format!("Cannot read directory '{dir}': {e}")))?;
    // ...
}
```

### Wire tools into the agent

```rust
let agent = client
    .agent(model)
    .preamble("You are a helpful assistant with access to local tools.")
    .tool(FileList)
    .tool(FileRead)
    .tool(ShellExec)
    .tool(HttpGet)
    .default_max_turns(10)
    .build();

let response = agent.prompt(prompt).await?;
```

### Run tests

```bash
# All tests
cargo test --workspace

# Just tool unit tests
cargo test -p mv-core tools

# CI gate
just ci
```
