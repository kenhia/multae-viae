# Research: Multi-Model Routing & OpenTelemetry Observability

**Feature**: 002-multi-model-telemetry
**Date**: 2026-04-28

## R1: YAML Configuration Parsing

**Decision**: Use `serde_yml` 0.0.12  
**Rationale**: `serde_yaml` (by dtolnay) is deprecated and no longer maintained.
`serde_yml` is the actively maintained fork with an identical API surface.  
**Alternatives considered**:
- `serde_yaml` 0.9.34 — deprecated, no security patches
- `toml` — TOML is less expressive for nested model config; YAML matches
  the DSL format planned for Phase 4

## R2: OpenTelemetry Crate Versions

**Decision**: Use the OpenTelemetry 0.31.x family  
**Rationale**: Latest stable release line (April 2026). The `tracing-opentelemetry`
0.32.x bridge is compatible with `opentelemetry` 0.31.x (it runs one version ahead
per its documented convention since 0.26).  

| Crate | Version | Purpose |
|-------|---------|---------|
| `opentelemetry` | 0.31 | OTel API (trait definitions) |
| `opentelemetry-sdk` | 0.31 | SDK implementation (TracerProvider, BatchExporter) |
| `opentelemetry-otlp` | 0.31 | OTLP exporter (gRPC via tonic) |
| `tracing-opentelemetry` | 0.32 | Bridge `tracing` spans → OTel spans |

**Alternatives considered**:
- OpenTelemetry 0.29.x (documented in `docs/05-telemetry-observability.md`) —
  two major versions behind, no reason to use older versions
- `opentelemetry-stdout` — useful for debugging but not sufficient for
  Jaeger visualization; could add later as a diagnostic feature

## R3: Rig Provider Architecture

**Decision**: Use `rig::providers::openai` module from `rig-core` 0.35  
**Rationale**: Rig 0.35 ships with 18+ provider modules built in, including
`rig::providers::ollama` (already in use) and `rig::providers::openai`. No
separate crate or feature flag needed. Both providers implement the
`CompletionClient` trait, so the CLI can use either interchangeably.  

**Key implementation detail**: The OpenAI provider requires an API key. Rig's
`openai::Client::from_env()` reads `OPENAI_API_KEY` from the environment.
The Ollama provider uses `Nothing` as the API key (no auth needed).

**Alternatives considered**:
- Separate HTTP client for OpenAI — unnecessary; Rig already handles it
- Adding `rig-anthropic` or other provider crates — out of scope per spec

## R4: OTLP Graceful Degradation

**Decision**: Initialize the OTLP exporter inside a `Result`; on failure,
log a warning and proceed without OTel export.  
**Rationale**: FR-008 requires that prompts succeed even when the collector
is unreachable. The `opentelemetry-otlp` `SpanExporter::builder()` can fail
at initialization if the endpoint is malformed. At runtime, the
`BatchSpanProcessor` handles export failures silently — spans are dropped,
not retried indefinitely.  

**Implementation approach**:
1. If `--otlp` is set (or `OTEL_EXPORTER_OTLP_ENDPOINT` env var exists),
   attempt to initialize the OTLP exporter
2. If initialization fails, warn and fall back to tracing-only (no OTel layer)
3. If initialization succeeds but the collector is unreachable at export time,
   the batch processor silently drops spans — no user impact

## R5: Span Flush on Exit

**Decision**: Call `opentelemetry::global::shutdown_tracer_provider()` before
process exit.  
**Rationale**: FR-009 requires pending spans to be flushed. The
`SdkTracerProvider::shutdown()` method flushes the batch processor and waits
for pending exports. This must be called before `main()` returns. Using
`opentelemetry::global::shutdown_tracer_provider()` is the idiomatic approach.  

**Implementation approach**: After the `run()` function completes (success or
error), call shutdown before `process::exit()`.

## R6: Model Registry Design

**Decision**: Simple YAML file with a flat list of model entries, loaded at
startup into an in-memory `Vec<ModelEntry>` with lookup by model ID.  
**Rationale**: Prescriptive routing (Phase 1) only needs ID-based lookup.
No need for scoring, ranking, or capability matching yet. Keep it simple
per Constitution Principle VII.  

**Config file search order**:
1. `--config <path>` flag (explicit)
2. `./models.yaml` (current working directory)
3. No file found → use built-in defaults (backward compatible)

**Alternatives considered**:
- HashMap-based registry — premature for a small number of models; Vec with
  linear scan is simpler and sufficient
- TOML config — less natural for lists of models with nested attributes
- Runtime discovery via Ollama API (`/api/tags`) — deferred to Phase 5;
  adds complexity and network dependency at startup
