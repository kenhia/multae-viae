# Research: TensorRT-LLM Provider Integration

## Overview

Research findings for integrating `trtllm-serve` as a provider in multae-viae.
Most questions were pre-answered by the assessment at
`docs/11-trt-llm-integration.md`; this document captures the remaining
technical details needed for implementation.

## R1: OpenAI API Compatibility via Rig

**Decision**: Reuse `call_openai()` with a placeholder API key

**Rationale**: `trtllm-serve` exposes `/v1/chat/completions` and
`/v1/completions` with full OpenAI protocol compatibility. Rig's
`openai::Client::builder().api_key(key).base_url(url).build()` works directly.
The server requires an `Authorization` header but accepts any non-empty value
(NVIDIA docs use `"tensorrt_llm"` as the example key).

**Alternatives considered**:
- Custom HTTP client via reqwest: Unnecessary — Rig's OpenAI client handles
  serialization, streaming, tool calling, and error parsing
- New Rig provider: Over-engineered — the protocol is identical to OpenAI

**Implementation**: Add `"trtllm"` arm to the provider match in `run_prompt()`
and `RigPromptExecutor`, calling `call_openai()` with
`api_key = "tensorrt_llm"`. No API key env var required.

## R2: Health Check Endpoint

**Decision**: `GET /health` with a 2-second timeout

**Rationale**: `trtllm-serve` exposes three management endpoints:
- `/health` — returns 200 when the server is ready
- `/metrics` — runtime iteration stats (GPU memory, KV cache)
- `/version` — server version info

The `/health` endpoint is the right target for a quick liveness check. It does
not require the `/v1` prefix — it sits at the server root (e.g.,
`http://localhost:8000/health`).

**Alternatives considered**:
- `/v1/models` (OpenAI models list): Works but returns model data we don't need
  for a simple health check
- No health check at all: Poor UX — connection timeouts from reqwest/Rig take
  30+ seconds

**Implementation**: `reqwest::Client::new().get(health_url).timeout(2s).send()`
in a new `trtllm::health` module. Called before the first prompt to a `trtllm`
model. The health URL is derived from the model endpoint by stripping `/v1`
and appending `/health`.

## R3: Model Name Mapping

**Decision**: Use `--served_model_name` on the server side or match the model
path exactly

**Rationale**: `trtllm-serve` uses the HuggingFace model path as the model name
by default (e.g., `meta-llama/Meta-Llama-3.1-8B-Instruct`). This is awkward as
a model ID in `models.yaml`. Two solutions:

1. Start the server with `--served_model_name llama-3_1-8b-fp8` to set a short
   alias
2. Use the full HF path as the model ID in `models.yaml` (works but verbose)

The `models.yaml` `id` field is what the user types on the CLI. The actual model
name sent to the API must match what the server expects. We need a way to map
between them.

**Implementation**: Add an optional `served_name` field to `ModelEntry`. If set,
this is sent as the model name in the API call instead of the `id` field. This
is useful for all providers (e.g., an Ollama model with `id: "code"` could map
to `served_name: "qwen2.5-coder:7b"`), but is particularly needed for TRT-LLM
where the server-side name is a HuggingFace path.

## R4: Model Registry Metadata Fields

**Decision**: Add optional `architecture`, `quant`, `expected_vram_gb` fields

**Rationale**: These fields are informational — they don't change runtime
behavior but enable:
- Telemetry span attributes for trace analysis
- Future routing decisions (Phase 5)
- CLI `--list-models` output enrichment

Since they are optional and default to `None`, existing `models.yaml` files
are not affected.

**Alternatives considered**:
- Separate TRT-LLM config file: Unnecessary complexity — the model registry
  already supports per-model configuration
- Nested `trtllm` object in model entry: Over-structured for 3 optional fields

## R5: Telemetry Attributes

**Decision**: Set `gen_ai.system = "trtllm"` and include metadata fields as span
attributes

**Rationale**: The existing code sets `gen_ai.system = "ollama"` or
`gen_ai.system = "openai"` via `#[tracing::instrument]` attributes. For
TRT-LLM, we set `gen_ai.system = "trtllm"`. If the model entry has metadata
fields (architecture, quant), include them as additional span attributes.

## R6: `trtllm-serve` Default Endpoint

**Decision**: `http://localhost:8000/v1`

**Rationale**: `trtllm-serve` defaults to port 8000. The OpenAI-compatible
endpoints live under `/v1`. This matches the pattern used in NVIDIA's own
documentation (`base_url="http://localhost:8000/v1"`).

Note: The health endpoint is at the root (`/health`), not under `/v1`. The
health check function must strip `/v1` from the configured endpoint to
construct the health URL.
