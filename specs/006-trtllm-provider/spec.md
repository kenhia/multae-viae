# Feature Specification: TensorRT-LLM Provider

**Feature Branch**: `006-trtllm-provider`
**Created**: 2026-05-12
**Status**: Draft
**Input**: User description: "Phase 4.5 from docs/09-roadmap.md — Add TensorRT-LLM as a high-performance local inference provider alongside Ollama."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Send a Prompt to a TRT-LLM Model (Priority: P1)

A user configures a TRT-LLM model in `models.yaml` with `provider: trtllm` and
an endpoint pointing to a running `trtllm-serve` instance. The user sends a
prompt via the CLI and receives a response from the TRT-LLM-served model, just
as they would with an Ollama or OpenAI model.

**Why this priority**: This is the core value proposition — using TRT-LLM
optimized models through the same CLI interface. Everything else builds on this
working.

**Independent Test**: Start `trtllm-serve` with a model (e.g., Llama 3.1 8B
FP8), add a `trtllm` entry to `models.yaml`, run
`mv-cli -m llama-3_1-8b-fp8 "Explain Rust ownership"`, and verify a coherent
response is returned.

**Acceptance Scenarios**:

1. **Given** a `models.yaml` with a `trtllm` provider entry and a running
   `trtllm-serve` instance, **When** the user runs `mv-cli -m <model-id>
   "Hello"`, **Then** the CLI sends the prompt via the OpenAI-compatible API
   and prints the model's response.
2. **Given** a `trtllm` model is configured with a custom endpoint, **When**
   the user sends a prompt, **Then** the request is sent to that endpoint (not
   the default Ollama endpoint).
3. **Given** a `trtllm` model is set as the default model, **When** the user
   runs `mv-cli "Hello"` without specifying a model, **Then** the TRT-LLM
   model is used.
4. **Given** the `trtllm-serve` instance is not running, **When** the user
   sends a prompt, **Then** the CLI reports a clear connection error.

---

### User Story 2 - Use TRT-LLM Models in Workflows (Priority: P2)

A user authors a YAML workflow that specifies a TRT-LLM model for one or more
prompt steps. The workflow engine routes those steps through the TRT-LLM
provider and returns the results, just as it does for Ollama models.

**Why this priority**: Workflows are the primary execution model for multi-step
tasks. TRT-LLM models must work seamlessly within the workflow engine to be
useful beyond single prompts.

**Independent Test**: Create a workflow with `model: llama-3_1-8b-fp8` on a
prompt step, run it via `mv-cli workflow run`, and verify the step executes
against the TRT-LLM server.

**Acceptance Scenarios**:

1. **Given** a workflow YAML with a prompt step specifying a `trtllm`-backed
   model, **When** the workflow executes, **Then** the prompt step sends the
   request to the TRT-LLM endpoint and captures the response.
2. **Given** a workflow uses the default model which is a `trtllm` model,
   **When** the workflow runs without per-step model overrides, **Then** all
   prompt steps use the TRT-LLM model.

---

### User Story 3 - Health Check Before Prompt (Priority: P3)

Before sending a prompt to a TRT-LLM model, the system checks whether the
serving endpoint is healthy. If the server is unreachable or unhealthy, the user
gets a clear, actionable error before waiting for a timeout.

**Why this priority**: `trtllm-serve` is a service that must be started
manually. A fast health check prevents confusing timeout errors and tells the
user exactly what to do.

**Independent Test**: Stop the `trtllm-serve` service, run a prompt targeting a
`trtllm` model, and verify the CLI reports the server is unavailable with a hint
to start it.

**Acceptance Scenarios**:

1. **Given** a `trtllm` model is configured, **When** the user sends a prompt
   and the server is healthy, **Then** the prompt executes normally.
2. **Given** a `trtllm` model is configured, **When** the user sends a prompt
   and the server is unreachable, **Then** the CLI reports the server is not
   available and suggests how to start it.

---

### User Story 4 - TRT-LLM Telemetry Attributes (Priority: P4)

When a prompt is sent to a TRT-LLM model, the OpenTelemetry span includes
provider-specific attributes (provider name, quantization type, architecture) so
traces can distinguish TRT-LLM calls from Ollama or cloud calls.

**Why this priority**: Observability is a core project principle. Distinguishing
providers in traces is important for performance analysis and debugging, but it
does not block core functionality.

**Independent Test**: Send a prompt to a TRT-LLM model with OTLP tracing
enabled, inspect the trace in Jaeger, and verify TRT-LLM-specific attributes
appear on the span.

**Acceptance Scenarios**:

1. **Given** OTLP export is enabled, **When** a prompt is sent to a `trtllm`
   model, **Then** the span includes `gen_ai.system = "trtllm"` and any
   available model metadata (architecture, quantization).
2. **Given** a workflow executes multiple steps across providers, **When**
   traces are inspected, **Then** TRT-LLM steps are visually distinguishable
   from Ollama steps by their attributes.

---

### Edge Cases

- What happens when the TRT-LLM server is running but no model is loaded? The
  system should report the error from the server's response (typically a 404 or
  model-not-found error).
- What happens when `models.yaml` has a `trtllm` entry but the endpoint URL is
  malformed? The system should fail at connection time with a clear error.
- What happens when the TRT-LLM server returns an unexpected response format?
  Since `trtllm-serve` is OpenAI-compatible, this should not happen, but any
  deserialization errors should be surfaced clearly.
- What happens when both Ollama and TRT-LLM have models with the same logical
  name? The model ID in `models.yaml` is the disambiguator — each entry has a
  unique ID.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST support `provider: trtllm` in `models.yaml` model
  entries
- **FR-002**: System MUST route `trtllm` provider requests through the
  OpenAI-compatible chat completions API (`/v1/chat/completions`)
- **FR-003**: System MUST default the `trtllm` endpoint to
  `http://localhost:8000/v1` when no explicit endpoint is configured
- **FR-004**: System MUST classify `trtllm` as a local provider
  (`Locality::Local`)
- **FR-005**: System MUST send a non-empty API key string when calling
  `trtllm-serve` (the server accepts any value but requires the header)
- **FR-006**: System MUST support optional TRT-LLM metadata fields in model
  entries: `architecture`, `quant`, `expected_vram_gb`
- **FR-007**: System MUST perform a health check against the TRT-LLM endpoint
  before sending the first prompt to a `trtllm` model
- **FR-008**: System MUST include `gen_ai.system = "trtllm"` in OpenTelemetry
  spans for TRT-LLM model calls
- **FR-009**: System MUST support TRT-LLM models in workflow prompt steps (same
  model resolution as direct prompts)
- **FR-010**: System MUST surface connection errors and server errors with
  actionable messages (e.g., "TRT-LLM server not reachable — start it with
  `trtllm-serve <model>`")
- **FR-011**: System MUST support tool-calling through TRT-LLM models that have
  tool parser support (Qwen3, DeepSeek)

### Key Entities

- **ModelEntry**: Extended with optional fields `architecture` (String), `quant`
  (String), `expected_vram_gb` (u32) for TRT-LLM metadata
- **Provider**: Logical grouping — `ollama`, `openai`, `trtllm` — that
  determines API protocol, default endpoint, and locality

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can send prompts to TRT-LLM models using the same CLI
  syntax as Ollama models
- **SC-002**: Workflows execute prompt steps against TRT-LLM models without
  modification to workflow YAML (only the model ID changes)
- **SC-003**: When the TRT-LLM server is unavailable, the user receives an
  error within 5 seconds (not a 30-second timeout)
- **SC-004**: TRT-LLM model calls appear in OpenTelemetry traces with
  distinguishable provider attributes
- **SC-005**: All existing tests continue to pass (no regression in Ollama or
  OpenAI provider paths)

## Assumptions

- `trtllm-serve` is the serving mechanism (not raw Triton KServe v2), because
  it provides OpenAI-compatible endpoints
- Engine building and model preparation happen in the companion
  `trt-llm-explore` project — this project only consumes pre-built engines
- The user has an NVIDIA GPU with appropriate drivers; CPU-only inference is not
  supported for TRT-LLM
- `trtllm-serve` accepts any non-empty API key string (no real authentication)
- The OpenAI-compatible API from `trtllm-serve` is sufficiently compatible with
  Rig's OpenAI client for chat completions and tool calling
- Model IDs in `models.yaml` are user-chosen strings, not constrained to match
  HuggingFace repo names or Triton model names — the user maps them as they
  see fit
