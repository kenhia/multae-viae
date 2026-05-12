# Implementation Plan: DSL Workflow Engine

**Branch**: `005-dsl-engine` | **Date**: 2026-04-30 | **Spec**: [spec.md](spec.md)  
**Input**: Feature specification from `specs/005-dsl-engine/spec.md`

## Summary

Build a YAML-based DSL engine that parses, validates, and executes multi-step
workflows. Workflows define sequential steps (prompt, tool, transform) with
template variable interpolation and output passing between steps. The engine
integrates with the existing model registry, tool infrastructure, and MCP
integration, and instruments execution via OpenTelemetry. New `workflow run`
and `workflow validate` CLI subcommands expose the engine to users.

## Technical Context

**Language/Version**: Rust (edition 2024, stable toolchain)  
**Primary Dependencies**: serde_yml (YAML parsing), minijinja (template engine), rig-core (LLM completion), clap (CLI), tracing/opentelemetry (telemetry)  
**Storage**: Filesystem (YAML workflow files, external template files)  
**Testing**: cargo test (unit + integration), assert_cmd (CLI integration)  
**Target Platform**: Linux (primary), macOS (secondary)  
**Project Type**: Library (mv-core) + CLI (mv-cli)  
**Performance Goals**: Workflow validation < 1s for 50 steps; execution time dominated by model/tool latency  
**Constraints**: No new runtime dependencies beyond template engine; reuse existing model/tool infrastructure  
**Scale/Scope**: Single-user CLI; workflows with up to 50 steps

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | PASS | Spec at `specs/005-dsl-engine/spec.md` with acceptance criteria |
| II. Architecture First | PASS | Design doc at `docs/06-dsl-flow-management.md` covers DSL schema and Rust types |
| III. Test-Driven Development | PASS | Will follow red-green-refactor; integration tests for CLI subcommands |
| IV. Code Standards Gate | PASS | All code will pass fmt, clippy, check, test before commit |
| V. Documentation from Day One | PASS | quickstart.md produced in this plan; README/architecture updates in definition of done |
| VI. Quality & Observability | PASS | Workflow execution instrumented with OTel spans per spec FR-017 |
| VII. Simplicity & Intentional Design | PASS | Phase 4 scope limited to sequential execution + 3 step types; advanced types deferred |

## Project Structure

### Documentation (this feature)

```text
specs/005-dsl-engine/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── cli.md           # CLI subcommand contracts
└── tasks.md             # Phase 2 output (/speckit.tasks command)
```

### Source Code (repository root)

```text
crates/
├── mv-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── mcp/                # Existing MCP integration
│       ├── tools/              # Existing built-in tools
│       └── workflow/           # NEW: DSL engine module
│           ├── mod.rs          # Public API: load, validate, execute
│           ├── types.rs        # Workflow, Step, Input, Output types
│           ├── parser.rs       # YAML parsing with strict validation
│           ├── validate.rs     # Structural validation (refs, duplicates, cycles)
│           ├── template.rs     # Template variable interpolation (minijinja)
│           └── engine.rs       # Sequential execution engine
├── mv-cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs             # Extended with `workflow` subcommand group
│   └── tests/
│       ├── cli_args.rs         # Existing
│       ├── cli_logging.rs      # Existing
│       ├── cli_tools.rs        # Existing
│       └── cli_workflow.rs     # NEW: workflow run/validate integration tests
└── workflows/                  # Example workflow files for testing/docs
    └── examples/
        └── research.yaml
```

**Structure Decision**: New `workflow/` module in `mv-core` following the
existing pattern of `mcp/` and `tools/` modules. No new crate — the DSL
engine is a core library concern. CLI subcommands added to the existing
`mv-cli` binary via clap subcommand groups.
