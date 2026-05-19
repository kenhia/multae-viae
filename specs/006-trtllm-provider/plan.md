# Implementation Plan: TensorRT-LLM Provider

**Branch**: `006-trtllm-provider` | **Date**: 2026-05-12 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/006-trtllm-provider/spec.md`

## Summary

Add TensorRT-LLM (`trtllm`) as a local inference provider alongside Ollama and
OpenAI. The integration leverages `trtllm-serve`'s OpenAI-compatible API, which
means the protocol-level work is minimal — Rig's existing `openai::Client` with
a custom `base_url` handles the request/response format. The work centers on
provider registration, health checking, model registry metadata extensions,
telemetry attributes, and documentation.

## Technical Context

**Language/Version**: Rust (edition 2024, stable toolchain)  
**Primary Dependencies**: rig-core (OpenAI client for API calls), reqwest
(health checks), serde/serde_yml (model registry), clap (CLI), tracing/
opentelemetry (telemetry)  
**Storage**: Filesystem (models.yaml configuration)  
**Testing**: cargo test (unit + integration), assert_cmd (CLI integration)  
**Target Platform**: Linux (primary, NVIDIA GPU required for TRT-LLM)  
**Project Type**: Library (mv-core) + CLI (mv-cli)  
**Performance Goals**: Health check completes in < 2 seconds; prompt routing
adds < 1 ms overhead  
**Constraints**: No new heavy dependencies; reuse Rig's OpenAI client; no
engine building in this project  
**Scale/Scope**: Single-user CLI; 1-3 TRT-LLM models configured alongside
existing Ollama models  

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | PASS | Spec at `specs/006-trtllm-provider/spec.md` with acceptance criteria |
| II. Architecture First | PASS | Design doc at `docs/11-trt-llm-integration.md`; architecture update in definition of done |
| III. Test-Driven Development | PASS | Unit tests for provider routing, health check; integration tests for CLI |
| IV. Code Standards Gate | PASS | All code will pass fmt, clippy, check, test before commit |
| V. Documentation from Day One | PASS | quickstart.md produced in this plan; README/models.yaml examples updated |
| VI. Quality & Observability | PASS | TRT-LLM spans with `gen_ai.system = "trtllm"` per FR-008 |
| VII. Simplicity & Intentional Design | PASS | Reuses Rig's OpenAI client — no new abstraction layers |

## Project Structure

### Documentation (this feature)

```text
specs/006-trtllm-provider/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   └── cli.md           # CLI contract changes
└── tasks.md             # Phase 2 output (via /speckit.tasks)
```

### Source Code (repository root)

```text
crates/
├── mv-core/
│   └── src/
│       ├── lib.rs           # ModelEntry, Locality, ModelRegistry (modified)
│       └── trtllm/          # New: TRT-LLM provider module
│           ├── mod.rs        # Module re-exports
│           └── health.rs     # Health check client
├── mv-cli/
│   └── src/
│       └── main.rs          # Provider dispatch (modified)
│   └── tests/
│       └── cli_trtllm.rs    # TRT-LLM CLI integration tests
```

**Structure Decision**: TRT-LLM provider logic lives in a new `trtllm` module
within `mv-core`. The health check is the only new functionality — the actual
LLM call reuses `call_openai()` since `trtllm-serve` speaks OpenAI protocol.

## Complexity Tracking

No constitution violations to justify.
