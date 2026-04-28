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
  -m, --model <MODEL>      Model name [default: qwen3:4b]
  -e, --endpoint <URL>     Ollama endpoint [default: http://localhost:11434]
  -j, --json               Output response as JSON object
  -v, --verbose            Increase log verbosity (repeat for more: -vv)
  -h, --help               Print help
  -V, --version            Print version
```

