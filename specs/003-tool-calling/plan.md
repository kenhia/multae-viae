# Implementation Plan: Tool Calling & Agentic Loop

**Branch**: `003-tool-calling` | **Date**: 2026-04-29 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/003-tool-calling/spec.md`

## Summary

Extend the CLI from a single-shot prompt/response tool into an agentic system that can call tools (file operations, shell commands, HTTP requests) and loop on results. Rig 0.35's built-in multi-turn agent loop handles the tool calling cycle internally — our work is defining the tools, wiring them into the agent builder, and instrumenting them for telemetry.

## Technical Context

**Language/Version**: Rust, Edition 2024, stable v1.95.0  
**Primary Dependencies**: rig-core 0.35 (with `derive` feature for `#[rig_tool]` macro), tokio 1 (process, time), reqwest (HTTP GET), serde/serde_json  
**Storage**: N/A (file system access via tools, not persistent storage)  
**Testing**: cargo test (unit + integration via assert_cmd)  
**Target Platform**: Linux (primary), macOS  
**Project Type**: CLI application  
**Performance Goals**: Tool calls complete within 30s timeout; agentic loop completes within 10 turns  
**Constraints**: Shell command timeout 30s default; tool output truncation at 10,000 chars  
**Scale/Scope**: 3 built-in tools, single-user CLI

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | ✅ PASS | Spec at `specs/003-tool-calling/spec.md` |
| II. Architecture First | ✅ PASS | Architecture doc update required during polish phase |
| III. TDD | ✅ PASS | Tests written before/alongside tool implementations |
| IV. Code Standards Gate | ✅ PASS | `just ci` (fmt, clippy, test) required before commit |
| V. Documentation from Day One | ✅ PASS | README, architecture doc updates in scope |
| VI. Quality & Observability | ✅ PASS | Tool call telemetry spans with OTel; consistent error messages |
| VII. Simplicity | ✅ PASS | Using Rig's built-in loop instead of custom; `#[rig_tool]` macro reduces boilerplate |

**Post-design re-check**: All gates still pass. Design uses Rig's existing abstractions, no unnecessary complexity added.

## Project Structure

### Documentation (this feature)

```text
specs/003-tool-calling/
├── plan.md              # This file
├── research.md          # Phase 0 output — Rig API research
├── data-model.md        # Phase 1 output — entities and relationships
├── quickstart.md        # Phase 1 output — developer guide
├── contracts/           # Phase 1 output
│   └── cli.md           # Updated CLI contract with tool-calling behavior
└── tasks.md             # Phase 2 output (created by /speckit.tasks)
```

### Source Code (repository root)

```text
crates/
├── mv-core/
│   └── src/
│       ├── lib.rs          # Existing: ModelRegistry, MvError, etc.
│       └── tools/          # NEW: built-in tool implementations
│           ├── mod.rs       # Tool module exports
│           ├── file_list.rs # file_list tool
│           ├── file_read.rs # file_read tool
│           ├── shell_exec.rs# shell_exec tool
│           └── http_get.rs  # http_get tool
├── mv-cli/
│   └── src/
│       └── main.rs         # Modified: agent builder with tools, preamble
│   └── tests/
│       ├── cli_args.rs     # Existing + new tool-related tests
│       └── cli_tools.rs    # NEW: integration tests for tool calling
```

**Structure Decision**: Tools live in `mv-core::tools` module since they are reusable capabilities independent of the CLI. The CLI wires them into the Rig agent builder. This keeps the CLI thin and tools testable in isolation.

## Complexity Tracking

No constitution violations. Design stays within existing workspace structure, adds one sub-module.
