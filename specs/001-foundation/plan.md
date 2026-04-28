# Implementation Plan: Foundation — Workspace & Local Model Integration

**Branch**: `001-foundation` | **Date**: 2026-04-28 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/001-foundation/spec.md`

## Summary

Establish a Cargo workspace with two crates (`mv-core`, `mv-cli`) that can
send a natural-language prompt to a locally-running Ollama instance via the
Rig framework and print the response. Includes structured logging via
`tracing` and a `justfile` for common development commands.

## Technical Context

**Language/Version**: Rust (stable, edition 2024)
**Primary Dependencies**: rig-core 0.35, tokio, clap, tracing, serde, serde_json
**Storage**: N/A (no persistence in this phase)
**Testing**: cargo test (unit + integration)
**Target Platform**: Linux (primary), macOS/Windows (secondary)
**Project Type**: CLI application + library
**Performance Goals**: CLI startup to first output token < 30 seconds
**Constraints**: Must work with zero configuration files; sensible defaults only
**Scale/Scope**: Single developer, 2 crates, ~500-1000 LOC

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | ✅ PASS | Spec at `specs/001-foundation/spec.md` |
| II. Architecture First | ✅ PASS | Architecture at `docs/01-architecture-design.md` |
| III. Test-Driven Development | ✅ PASS | Tests will be written first per TDD |
| IV. Code Standards Gate | ✅ PASS | `cargo fmt`, `clippy`, `cargo test` in justfile |
| V. Documentation from Day One | ✅ PASS | README update planned |
| VI. Quality & Observability | ✅ PASS | `tracing` from day one; errors to stderr |
| VII. Simplicity | ✅ PASS | Only 2 crates; no abstractions beyond what spec requires |

**Gate result**: PASS — no violations.

## Project Structure

### Documentation (this feature)

```text
specs/001-foundation/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
└── tasks.md             # Phase 2 output (/speckit.tasks command)
```

### Source Code (repository root)

```text
Cargo.toml               # Workspace root
justfile                  # Task runner (build, check, test, run)
crates/
├── mv-core/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs        # Core types: error types, BackendConfig
└── mv-cli/
    ├── Cargo.toml
    └── src/
        └── main.rs       # CLI entry point: arg parsing, tracing setup, prompt→response
```

**Structure Decision**: Cargo workspace with crates under `crates/` directory.
Two crates only — `mv-core` (library) and `mv-cli` (binary). Future crates
(`mv-engine`, `mv-router`, etc.) will be added under `crates/` in later phases.

## Complexity Tracking

> **Fill ONLY if Constitution Check has violations that must be justified**

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
