# CLI Contract: MCP Integration

**Feature**: 004-mcp-integration  
**Date**: 2026-04-30

## New CLI Flag

### `--mcp-config`

Specifies the path to the MCP servers configuration file.

```text
mv-cli [OPTIONS] <PROMPT>

Options:
  --mcp-config <PATH>   Path to MCP servers YAML config file
                        [default: mcp-servers.yaml]
```

**Behavior**:
- If `--mcp-config` is provided, load MCP servers from the specified path
- If not provided, look for `mcp-servers.yaml` in the current directory
- If the default file does not exist, proceed without MCP servers (no error)
- If an explicit `--mcp-config` path does not exist, report an error and exit

## MCP Server Configuration File

### Format

```yaml
# mcp-servers.yaml
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    env:
      NODE_ENV: production

  - name: database
    transport: stdio
    command: /usr/local/bin/mcp-sqlite
    args: ["--db", "/path/to/database.db"]

  - name: rag
    transport: http
    url: http://192.168.1.100:8080/mcp
```

### Validation errors

| Condition | Output (stderr) | Exit |
|-----------|-----------------|------|
| Explicit config file not found | `error: MCP config file not found: <path>` | Yes |
| Invalid YAML syntax | `error: failed to parse MCP config '<path>': <details>` | Yes |
| Missing `command` for stdio transport | `error: MCP server '<name>': stdio transport requires 'command'` | Yes |
| Missing `url` for http transport | `error: MCP server '<name>': http transport requires 'url'` | Yes |
| Duplicate server name | `error: duplicate MCP server name: '<name>'` | Yes |

### Startup warnings (non-fatal)

| Condition | Output (stderr, -v) |
|-----------|---------------------|
| Server failed to start | `warn: MCP server '<name>' failed to connect: <details>` |
| Server handshake timeout | `warn: MCP server '<name>' handshake timed out after <N>s` |
| Tool name collision with built-in | `warn: MCP tool '<tool>' from '<server>' shadowed by built-in tool` |
| Empty servers list | `warn: MCP config has no servers defined` |

## CLI Output Changes

### Standard output

No changes to the response format. MCP tool calls are transparent to the user — the model response is printed the same way regardless of whether built-in or MCP tools were used.

### JSON output (`--json`)

No schema changes. The response JSON structure remains the same.

### Verbose output (`-v`, `-vv`)

At `-v` verbosity, MCP connection events are logged:

```text
INFO connecting to MCP server 'filesystem' via stdio
INFO MCP server 'filesystem' connected, discovered 3 tools
INFO connecting to MCP server 'rag' via http
WARN MCP server 'rag' failed to connect: connection refused
```

At `-vv` verbosity, individual tool discovery details are logged:

```text
DEBUG MCP server 'filesystem' tool: list_directory
DEBUG MCP server 'filesystem' tool: read_file
DEBUG MCP server 'filesystem' tool: search_files
```

## Examples

### Basic usage with MCP servers

```bash
# Uses default mcp-servers.yaml in current directory
mv-cli "List the files in /tmp"

# Explicit config path
mv-cli --mcp-config ./my-servers.yaml "Search the database for recent entries"
```

### Without MCP servers

```bash
# No mcp-servers.yaml present — works exactly as before
mv-cli "What files are in the current directory?"
```
