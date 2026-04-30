# Implementation Plan: MCP Integration

**Branch**: `004-mcp-integration` | **Date**: 2026-04-30 | **Spec**: [spec.md](spec.md)  
**Input**: Feature specification from `specs/004-mcp-integration/spec.md`

## Summary

Connect the Multae Viae controller to external MCP (Model Context Protocol) servers, enabling dynamic tool discovery and invocation over stdio and HTTP transports. MCP-discovered tools merge into the existing tool registry so the model sees a single unified tool set. The `rmcp` crate (v1.5) provides protocol handling, transport abstractions, and tool discovery. All MCP tool calls are instrumented in OpenTelemetry telemetry.

## Technical Context

**Language/Version**: Rust (edition 2024)  
**Primary Dependencies**: rmcp 1.5 (client feature), rig-core 0.35, tokio 1, serde/serde_json/serde_yml, clap 4, tracing/tracing-subscriber, opentelemetry 0.31  
**Storage**: YAML configuration files (mcp-servers.yaml)  
**Testing**: cargo test (unit + integration), assert_cmd (CLI smoke tests)  
**Target Platform**: Linux (primary), macOS  
**Project Type**: CLI application + core library (Cargo workspace)  
**Performance Goals**: MCP handshake completes within 5 seconds; tool calls add minimal latency over native tool calls  
**Constraints**: Graceful degradation — MCP server failures must not crash the CLI  
**Scale/Scope**: Support 1–10 concurrent MCP server connections per session

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | PASS | Spec at `specs/004-mcp-integration/spec.md` |
| II. Architecture First | PASS | MCP Client layer documented in `docs/01-architecture-design.md` and `docs/04-mcp-integration.md` |
| III. Test-Driven Development | PASS | TDD will be followed; unit tests for config parsing, integration tests for MCP connectivity |
| IV. Code Standards Gate | PASS | Pre-commit checks (fmt, clippy, check, test) enforced |
| V. Documentation from Day One | PASS | Plan, research, data-model, quickstart, contracts produced |
| VI. Quality & Observability | PASS | MCP tool calls instrumented with OpenTelemetry spans |
| VII. Simplicity & Intentional Design | PASS | Only Tools primitive in scope; Resources/Prompts/Sampling deferred |

## Project Structure

### Documentation (this feature)

```text
specs/004-mcp-integration/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── cli.md
└── tasks.md             # Phase 2 output (/speckit.tasks — NOT created by /speckit.plan)
```

### Source Code (repository root)

```text
crates/
├── mv-core/
│   ├── Cargo.toml           # Add rmcp dependency
│   └── src/
│       ├── lib.rs            # Re-export mcp module
│       ├── mcp/
│       │   ├── mod.rs        # MCP module root
│       │   ├── config.rs     # McpServerConfig, YAML parsing
│       │   ├── client.rs     # McpClient — lifecycle, tool discovery, invocation
│       │   └── registry.rs   # Merge MCP tools into Rig's tool system
│       └── tools/            # Existing built-in tools (unchanged)
├── mv-cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs           # Wire MCP client init + tool merging into agent setup
│   └── tests/
│       ├── cli_args.rs
│       ├── cli_logging.rs
│       └── cli_tools.rs      # Extend with MCP integration tests
```

**Structure Decision**: New MCP code lives in `mv-core/src/mcp/` as a new module within the existing core crate. No new crates are added — this follows VII (Simplicity) and keeps the workspace at two crates. The MCP module handles config parsing, client lifecycle, and tool registry merging. The CLI wires it together at startup.

## Complexity Tracking

No constitution violations. No complexity justifications needed.
