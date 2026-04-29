# CLI Contract: Tool Calling & Agentic Loop

**Feature**: 003-tool-calling
**Date**: 2026-04-29

## CLI Interface Changes

### Unchanged Options

```
mv-cli [OPTIONS] <PROMPT>

Arguments:
  <PROMPT>              The prompt to send to the model

Options:
  -m, --model <MODEL>   Model name (from registry or built-in)
  -e, --endpoint <URL>  Override model endpoint
  -c, --config <PATH>   Path to models.yaml config file
      --otlp [<URL>]    Enable OTLP trace export [default: http://localhost:4318]
  -j, --json            Output response as JSON object
  -v, --verbose         Increase log verbosity (repeat for more: -vv)
  -h, --help            Print help
  -V, --version         Print version
```

No new CLI flags in this phase. Tool calling is always enabled when the model supports it.

## Behavioral Changes

### Before (Sprint 002)

The CLI sends a single prompt to the model and returns the model's text response. No tool calling.

### After (Sprint 003)

The CLI builds an agent with registered tools and a system preamble. When the model decides to call a tool:

1. The tool is executed locally
2. The result is fed back to the model
3. The model generates a final text response incorporating the tool result

This is transparent to the user — the same CLI invocation works for both tool-using and non-tool-using queries.

## Tool Definitions

### `file_list`

- **Description**: List the contents of a directory
- **Parameters**: `path` (string, optional, default `"."`)
- **Returns**: Newline-separated list of file/directory names
- **Errors**: Directory not found, permission denied

### `file_read`

- **Description**: Read the contents of a file
- **Parameters**: `path` (string, required)
- **Returns**: File contents as string (truncated at 10,000 chars)
- **Errors**: File not found, permission denied, binary file

### `shell_exec`

- **Description**: Execute a shell command and return its output
- **Parameters**: `command` (string, required)
- **Returns**: Combined stdout and stderr (truncated at 10,000 chars)
- **Errors**: Command not found, timeout (30s), non-zero exit code (still returns output)

### `http_get`

- **Description**: Fetch a URL via HTTP GET and return the response body
- **Parameters**: `url` (string, required)
- **Returns**: Response body as string (truncated at 10,000 chars)
- **Errors**: Invalid URL, connection failed, timeout (30s), non-2xx status

## Output Format

### Standard output (unchanged)

```
<model response text incorporating tool results>
```

### JSON output (`--json`)

```json
{"response": "<model response text incorporating tool results>"}
```

### Error output (unchanged)

```
Error: <error message>
```

## Telemetry

Each tool invocation within the agentic loop emits a tracing span with:

| Attribute | Example |
|-----------|---------|
| `tool.name` | `"file_list"` |
| `tool.args` | `{"path": "."}` |
| `tool.status` | `"success"` or `"error"` || `tool.duration_ms` | `42` |
These spans appear as children of the parent model call span in Jaeger.

## Agentic Loop Limits

| Parameter | Value | Configurable |
|-----------|-------|-------------|
| Max turns | 10 | Not via CLI (hardcoded default) |
| Shell timeout | 30s | Not via CLI |
| HTTP timeout | 30s | Not via CLI |
| Output truncation | 10,000 chars | Not via CLI |
