# Implementation Plan: TRT-LLM Streaming & Hardening

**Branch**: `007-trtllm-hardening` | **Date**: 2026-05-19 | **Spec**: [spec.md](spec.md)
**Input**: Feature specification from `specs/007-trtllm-hardening/spec.md`

## Summary

Promote the TRT-LLM provider added in sprint 006 from "minimum viable" to
daily-driver quality. The work is concentrated in four areas:

1. **Streaming** — wire Rig 0.35's `Agent::stream_prompt()` (available on the
   `CompletionsClient`-backed agent) into a new `--stream` CLI flag for TRT-LLM
   prompts. Tool calling continues to work because `stream_prompt(...).multi_turn(10)`
   drives the same agent loop as `prompt()`.
2. **Actionable 502 mapping** — detect HTTP 502 responses from the proxy at
   the boundary, classify them as "model not loaded", and surface a new
   `MvError::ModelNotLoaded { model, hint }` whose Display string includes
   the exact `just load <model>` command.
3. **Token usage telemetry** — extract `usage.prompt_tokens` /
   `usage.completion_tokens` from completion and streaming responses and
   attach them to the existing `llm_completion` span as
   `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens`.
4. **Stop sequences** — add an optional `stop_sequences: Vec<String>` field
   to `ModelEntry` and forward it on every TRT-LLM request, with a small
   provider-level default applied when the model entry doesn't declare any.

Tool calling, workflow execution, and stream-failure handling get integration
tests (most marked `#[ignore]` so `cargo test` stays green without the proxy,
matching the pattern already used by the live tests in `cli_trtllm.rs`).

## Technical Context

**Language/Version**: Rust 2024 edition, stable 1.95.0
**Primary Dependencies**: rig-core 0.35 (with `derive` + `rmcp` features) —
specifically `openai::CompletionsClient` and the `StreamingPrompt` trait;
reqwest 0.13 (for the existing health check and 502 detection); tokio 1
(`full` features); tracing 0.1 + opentelemetry 0.31 + opentelemetry-otlp
0.31 + tracing-opentelemetry 0.32 (telemetry); serde_yml 0.0.12 (config);
clap 4 (CLI); futures-util (already pulled transitively by Rig — needed for
`StreamExt::next`)
**Storage**: Filesystem (`models.yaml`, workflow YAML)
**Testing**: `cargo test`; CLI integration via `assert_cmd 2` + `predicates 3`
+ `tempfile 3`; live-proxy tests marked `#[ignore]` per existing pattern
**Target Platform**: Linux (NVIDIA GPU for the proxy; client is host-agnostic)
**Project Type**: Library (`mv-core`) + CLI (`mv-cli`) workspace
**Performance Goals**: First streamed token visible within 1 s of request
acceptance (SC-001); telemetry overhead negligible (<1 ms per call)
**Constraints**: No breaking changes to `models.yaml` schema (new fields
optional); existing 123 tests stay green; MCP shutdown discipline in the CLI
preserved (capture result without `?`, call `shutdown_all`, then propagate);
no over-engineering — defensive handling only at the proxy boundary
**Scale/Scope**: Single-user CLI, 1–3 TRT-LLM models in the registry, single
streamed completion at a time

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Spec-Driven Development | PASS | Spec at [spec.md](spec.md); this plan + design docs live under `specs/007-trtllm-hardening/` |
| II. Architecture First | PASS | Extends provider documented in `docs/11-trt-llm-integration.md`; architecture update is part of the polish phase |
| III. Test-Driven Development | PASS | New behavior driven by unit tests in `mv-core` (502 mapping, stop-sequence forwarding, usage extraction) and CLI integration tests in `cli_trtllm.rs` (streaming, tool calling, workflow); live-proxy tests `#[ignore]`d so `cargo test` stays green |
| IV. Code Standards Gate | PASS | `just ci` (fmt + clippy `-D warnings` + test) gates every commit; no new lints expected |
| V. Documentation from Day One | PASS | [quickstart.md](quickstart.md) + [contracts/cli.md](contracts/cli.md) produced in Phase 1; `README.md`, `models.yaml`, `docs/11-trt-llm-integration.md`, `docs/09-roadmap.md` updated during implementation polish |
| VI. Quality & Observability | PASS | New `gen_ai.usage.input_tokens` / `gen_ai.usage.output_tokens` span attributes on every TRT-LLM call; actionable error per Principle VI ("tell the user what they can do about it"); stdout/stderr split preserved |
| VII. Simplicity & Intentional Design | PASS | Reuses Rig's built-in streaming + tool-calling loop; no new abstractions; stop-sequences are a thin optional config field; `ModelNotLoaded` is a focused new error variant rather than overloading `BackendUnreachable` |

No violations. Re-evaluated post-Phase 1: still PASS — design docs introduce
no new abstractions beyond what the spec demands.

## Project Structure

### Documentation (this feature)

```text
specs/007-trtllm-hardening/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   └── cli.md           # Phase 1 output — CLI + config + telemetry contract
├── checklists/
│   └── requirements.md  # already produced by /speckit.specify
└── tasks.md             # Phase 2 output (produced by /speckit.tasks — NOT this command)
```

### Source Code (repository root)

```text
crates/
├── mv-core/
│   ├── src/
│   │   ├── lib.rs                  # ModelEntry: add stop_sequences; MvError: add ModelNotLoaded
│   │   └── trtllm/
│   │       ├── mod.rs              # re-exports
│   │       ├── health.rs           # existing — unchanged (separate liveness probe)
│   │       ├── usage.rs            # NEW — Usage struct + extraction from rig responses
│   │       └── stop.rs             # NEW — default_stop_sequences() + merge with entry
│   └── tests/                       # crate-level integration not currently used
└── mv-cli/
    ├── src/
    │   └── main.rs                 # add --stream flag; add stream_trtllm(); map 502 → ModelNotLoaded
    └── tests/
        └── cli_trtllm.rs           # extend with streaming + tool-calling + 502 + workflow tests
models.yaml                          # add stop_sequences example for llama-fp8
docs/
├── 09-roadmap.md                    # mark Phase 4.5.1 done after merge
└── 11-trt-llm-integration.md        # add streaming + hardening section
```

**Structure Decision**: Provider logic stays in `crates/mv-core/src/trtllm/`.
The streaming entry point itself lives in `crates/mv-cli/src/main.rs` as
`stream_trtllm()` — sibling to the existing `call_trtllm()` — because
streaming requires direct access to the CLI's stdout/stderr writers and
the agent loop is built per request from the `CompletionsClient`. Pure
provider concerns (token-usage extraction, default stop sequences, 502
detection) live as small reusable modules under `mv-core::trtllm` so
`RigPromptExecutor` (workflow path) reuses them without duplicating logic.

## Complexity Tracking

No constitution violations to justify.
