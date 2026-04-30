# multae-viae — many paths

A local-first agentic controller built in Rust that orchestrates multiple LLMs,
tools, and services to act as an always-on AI assistant.

## Vision

- **Local-first**: Models run locally via Ollama/mistral.rs, with cloud fallback
- **Tool-aware**: MCP protocol integration for extensible tool use
- **Multi-model**: Dynamic model routing — right model for each task
- **Observable**: First-class OpenTelemetry telemetry for a companion dashboard
- **Declarative**: YAML-based DSL for workflow/prompt orchestration
- **Extensible**: RAG integration, system monitoring, scheduled tasks

## Research & Design

See the [docs/](docs/) directory for in-depth research and architecture design:

- [Research Overview](docs/00-research-overview.md) — Project vision and key decisions
- [Architecture Design](docs/01-architecture-design.md) — System architecture and data flow
- [Framework Comparison](docs/02-framework-comparison.md) — Rig vs Kalosm vs mistral.rs vs Candle
- [Local Inference](docs/03-local-inference.md) — Ollama, mistral.rs, llama.cpp options
- [MCP Integration](docs/04-mcp-integration.md) — Model Context Protocol and RMCP SDK
- [Telemetry](docs/05-telemetry-observability.md) — OpenTelemetry, tracing, dashboard integration
- [DSL Design](docs/06-dsl-flow-management.md) — YAML-based workflow definition language
- [Model Routing](docs/07-model-routing.md) — Prescriptive, adaptive, and hybrid routing
- [RAG Integration](docs/08-rag-integration.md) — Retrieval-Augmented Generation patterns
- [Roadmap](docs/09-roadmap.md) — Phased implementation plan
- [Investigations](docs/10-investigations.md) — Open questions on tool-use reliability

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Language | Rust |
| Agent Framework | [Rig](https://github.com/0xPlaygrounds/rig) (`rig-core`) |
| Local Inference | [Ollama](https://ollama.com/) + [mistral.rs](https://github.com/EricLBuehler/mistral.rs) |
| MCP | [RMCP](https://github.com/modelcontextprotocol/rust-sdk) |
| Telemetry | [OpenTelemetry](https://github.com/open-telemetry/opentelemetry-rust) + `tracing` |
| ML Framework | [Candle](https://github.com/huggingface/candle) (HuggingFace) |

## Quick Start

### Prerequisites

- **Rust** (stable, edition 2024): `rustup update stable`
- **Ollama** running locally: `ollama serve`
- **Model pulled**: `ollama pull qwen3:4b`
- **just** task runner: `cargo install just`

### Build & Run

```bash
just build                              # Build all crates
just run "What is Rust?"                # Send a prompt
just run "What is Rust?" --json         # JSON output
just run "Hello" -vv 2>debug.log        # Verbose logging
just ci                                 # Format + clippy + test
```

### CLI Usage

```
mv-cli [OPTIONS] <PROMPT>

Options:
  -m, --model <MODEL>      Model name (from registry or built-in)
  -e, --endpoint <URL>     Override model endpoint
  -c, --config <PATH>      Path to models.yaml config file
      --otlp [<ENDPOINT>]  Enable OTLP trace export [default: http://localhost:4318]
  -j, --json               Output response as JSON object
  -v, --verbose            Increase log verbosity (repeat for more: -vv)
  -h, --help               Print help
  -V, --version            Print version
```

### Model Configuration

Create a `models.yaml` in the project root to configure available models:

```yaml
models:
  - id: qwen3:4b
    provider: ollama
    default: true
  - id: qwen3:8b
    provider: ollama
  # Cloud provider (set OPENAI_API_KEY env var)
  # - id: gpt-4o-mini
  #   provider: openai
  #   api_key_env: OPENAI_API_KEY
```

Without a config file, the CLI uses built-in defaults (qwen3:4b via Ollama).

### OpenTelemetry Traces

To view traces in Jaeger:

```bash
# Start Jaeger (all-in-one)
docker run -d --name jaeger \
  -p 16686:16686 -p 4318:4318 \
  jaegertracing/all-in-one:latest

# Send a prompt with tracing enabled
cargo run -p mv-cli -- --otlp "What is Rust?"

# View traces at http://localhost:16686
```

### Tool Calling

The CLI includes built-in tools that the model can invoke automatically during a
conversation. When a question requires local environment interaction, the agent
calls the appropriate tool, receives the result, and incorporates it into a
natural-language response.

**Available tools:**

| Tool | Description | Example Prompt |
|------|-------------|---------------|
| `file_list` | List directory contents | "What files are in the current directory?" |
| `file_read` | Read a file | "What does README.md say?" |
| `shell_exec` | Run a shell command (30s timeout) | "What git branch am I on?" |
| `http_get` | Fetch a URL via HTTP GET (30s timeout) | "What is the title of https://example.com?" |

Tool calling is transparent — the same CLI invocation works for both tool-using
and non-tool-using queries. Tool output is truncated at 10,000 characters. The
agentic loop runs for up to 10 turns before returning.

```bash
# File tools
cargo run -p mv-cli -- "What files are in the current directory?"
cargo run -p mv-cli -- "What does the README say?"

# Shell execution
cargo run -p mv-cli -- "What git branch am I on?"

# HTTP fetch
cargo run -p mv-cli -- "What is the title of https://example.com?"
```

### MCP Server Configuration

Connect external MCP servers to extend the CLI with additional tools. Create an
`mcp-servers.yaml` in the project root (or specify with `--mcp-config`):

```yaml
servers:
  # Local stdio server (spawns a child process)
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    env:
      NODE_ENV: production

  # Remote HTTP server
  - name: remote-rag
    transport: http
    url: http://192.168.1.100:8080/mcp
```

MCP tools merge with built-in tools into a single unified set. The model chooses
the best tool for each task — built-in or MCP — transparently.

```bash
# Use with default config file (mcp-servers.yaml)
cargo run -p mv-cli -- "List files in /tmp"

# Use with explicit config path
cargo run -p mv-cli -- --mcp-config path/to/servers.yaml "Search the database"
```

**Behavior:**

- MCP server failures are logged and skipped — the CLI continues with remaining
  servers and built-in tools
- Built-in tools take precedence over MCP tools with the same name
- All MCP connections are shut down gracefully on CLI exit
- MCP tool calls appear in OpenTelemetry traces when `--otlp` is enabled

