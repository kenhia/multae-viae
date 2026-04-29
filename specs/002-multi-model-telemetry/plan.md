# Implementation Plan: Multi-Model Routing & OpenTelemetry Observability

**Branch**: `002-multi-model-telemetry` | **Date**: 2026-04-28 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `/specs/002-multi-model-telemetry/spec.md`

## Summary

Add a YAML-based model registry to support multiple models across local
(Ollama) and cloud (OpenAI-compatible) providers, with prescriptive routing
via the `--model` flag. Integrate OpenTelemetry tracing to export spans for
model calls to an OTLP-compatible collector (e.g., Jaeger), with GenAI
semantic convention attributes.

## Technical Context

**Language/Version**: Rust (stable, edition 2024)  
**Primary Dependencies**: rig-core 0.35 (Ollama + OpenAI providers built-in), tokio, clap, tracing, serde, serde_json, serde_yml, opentelemetry 0.31, opentelemetry-sdk 0.31, opentelemetry-otlp 0.31, tracing-opentelemetry 0.32  
**Storage**: YAML config file (`models.yaml`) — read-only at startup  
**Testing**: cargo test (unit + integration), assert_cmd for CLI  
**Target Platform**: Linux (primary), macOS/Windows (secondary)  
**Project Type**: CLI application + library  
**Performance Goals**: Config loading < 10ms; OTLP export non-blocking (background batch)  
**Constraints**: Zero-config must still work (backward compatible); OTLP failure must not block prompt execution  
**Scale/Scope**: Single developer, 2 crates, ~500 LOC added  

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | ✅ PASS | Spec at `specs/002-multi-model-telemetry/spec.md` |
| II. Architecture First | ✅ PASS | Architecture at `docs/01-architecture-design.md`; model routing design at `docs/07-model-routing.md`; telemetry design at `docs/05-telemetry-observability.md` |
| III. Test-Driven Development | ✅ PASS | Tests written first per TDD |
| IV. Code Standards Gate | ✅ PASS | `just ci` (fmt --check, clippy -D warnings, test) |
| V. Documentation from Day One | ✅ PASS | README and docs updated during polish phase |
| VI. Quality & Observability | ✅ PASS | OpenTelemetry integration is the core deliverable; errors remain actionable |
| VII. Simplicity | ✅ PASS | Prescriptive routing only (no adaptive/hybrid); two providers only (Ollama + OpenAI-compatible); no new crates |

**Gate result**: PASS — no violations.

## Project Structure

### Documentation (this feature)

```text
specs/002-multi-model-telemetry/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
└── tasks.md             # Phase 2 output (/speckit.tasks command)
```

### Source Code (repository root)

```text
Cargo.toml               # Workspace root
justfile                  # Task runner
models.yaml               # Example model registry config (NEW)
crates/
├── mv-core/
│   ├── Cargo.toml        # + serde_yml dependency
│   └── src/
│       └── lib.rs         # + ModelEntry, ModelRegistry, Provider, config loading
└── mv-cli/
    ├── Cargo.toml         # + opentelemetry, opentelemetry-sdk, opentelemetry-otlp, tracing-opentelemetry
    ├── src/
    │   └── main.rs        # + --config, --otlp flags; registry-based model resolution; OTel init
    └── tests/
        ├── cli_args.rs    # + registry and config flag tests
        └── cli_logging.rs # existing
```

**Structure Decision**: Continue with existing two-crate layout. Model registry
and config types live in `mv-core` (they are reusable). OTel initialization and
CLI flags live in `mv-cli` (they are CLI-specific). No new crates needed.

## Complexity Tracking

> No violations — section intentionally empty.

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| [e.g., 4th project] | [current need] | [why 3 projects insufficient] |
| [e.g., Repository pattern] | [specific problem] | [why direct DB access insufficient] |
