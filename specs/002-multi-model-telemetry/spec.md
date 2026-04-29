# Feature Specification: Multi-Model Routing & OpenTelemetry Observability

**Feature Branch**: `002-multi-model-telemetry`
**Created**: 2026-04-28
**Status**: Draft
**Input**: Phase 1 from docs/09-roadmap.md — Call multiple models, basic model routing, OpenTelemetry traces.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Configure and Select Models from a Registry (Priority: P1)

A developer maintains a YAML configuration file that lists available models with their provider, capabilities, and locality. When the CLI starts, it loads this registry and uses the specified model (via `--model` flag or config default) to handle the prompt. If the requested model is not in the registry, the system reports a clear error. This lets the developer manage multiple models — local and cloud — from a single configuration file.

**Why this priority**: Without a model registry, the system cannot support multiple models. This is the foundation for all routing and multi-model functionality.

**Independent Test**: Create a `models.yaml` with two models listed, run `mv-cli --model qwen3:8b "Hello"` and confirm it uses the correct model; run with an unknown model name and confirm a clear error.

**Acceptance Scenarios**:

1. **Given** a `models.yaml` file with `qwen3:4b` and `qwen3:8b` listed, **When** the user runs `mv-cli "Hello"`, **Then** the default model from the config is used and a response is printed.
2. **Given** a `models.yaml` file exists, **When** the user runs `mv-cli --model qwen3:8b "Hello"`, **Then** the `qwen3:8b` model is used.
3. **Given** a `models.yaml` file exists, **When** the user runs `mv-cli --model nonexistent "Hello"`, **Then** an error message indicates the model is not in the registry.
4. **Given** no `models.yaml` file exists, **When** the user runs `mv-cli "Hello"`, **Then** the system falls back to built-in defaults (current behavior) and works without configuration.

---

### User Story 2 - View Traces of Model Calls in Jaeger (Priority: P2)

A developer working on the project wants to observe what happens during a model call — which model was selected, how long the call took, and how many tokens were used. When OpenTelemetry tracing is enabled (via flag or environment variable), the CLI exports trace data to an OTLP-compatible collector. The developer opens Jaeger to see spans for the model interaction, with attributes following GenAI semantic conventions.

**Why this priority**: Observability is critical for debugging multi-model interactions and understanding performance, but the system is functional without it.

**Independent Test**: Start Jaeger all-in-one locally, run `mv-cli --otlp "What is Rust?"`, then open Jaeger UI and confirm a trace appears with model call spans, model name, and timing.

**Acceptance Scenarios**:

1. **Given** Jaeger is running on `localhost:4317`, **When** the user runs `mv-cli --otlp "What is Rust?"`, **Then** a trace is exported and visible in Jaeger showing the model call span.
2. **Given** OTLP export is enabled, **When** a model call completes, **Then** the trace span includes attributes: model name, provider, prompt length, response length, and duration.
3. **Given** OTLP export is enabled but the collector is unreachable, **When** the user runs a prompt, **Then** the prompt still completes successfully and a warning is logged about the failed export.
4. **Given** OTLP export is not enabled (default), **When** the user runs a prompt, **Then** no OTLP export occurs and behavior matches the current system.

---

### User Story 3 - Route a Prompt to a Cloud Fallback Provider (Priority: P3)

A developer configures both local (Ollama) and cloud (OpenAI-compatible) models in the registry. When the user specifies a cloud model via `--model`, the system routes the request to the appropriate cloud provider. This enables hybrid local/cloud usage from a single CLI tool.

**Why this priority**: Cloud fallback expands model choice beyond locally-running Ollama, but the system is fully usable with only local models.

**Independent Test**: Add an OpenAI-compatible model to `models.yaml`, run `mv-cli --model gpt-4o-mini "Hello"`, and confirm the response comes from the cloud provider.

**Acceptance Scenarios**:

1. **Given** a `models.yaml` with an OpenAI-compatible model configured (with API key from environment), **When** the user runs `mv-cli --model gpt-4o-mini "Hello"`, **Then** the request is routed to the OpenAI API and a response is printed.
2. **Given** a cloud model is configured but the API key is missing from the environment, **When** the user specifies that model, **Then** a clear error indicates the API key is required.
3. **Given** a cloud model is configured, **When** OTLP is enabled and the user sends a prompt, **Then** trace spans include the provider type (cloud vs local) as an attribute.

---

### Edge Cases

- What happens when `models.yaml` is malformed or has invalid syntax? → Clear parse error with file path and line context.
- What happens when multiple models share the same ID? → First match wins, warning logged.
- What happens when OTLP export is enabled but initializing the exporter fails (e.g., bad endpoint URL)? → Warning logged, CLI proceeds without OTLP.
- What happens when a cloud provider returns a rate-limit error? → Error reported as `CompletionFailed` with details.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST load model definitions from a `models.yaml` configuration file.
- **FR-002**: System MUST support a model registry that stores model ID, provider, and locality (local/cloud) for each configured model.
- **FR-003**: System MUST resolve the `--model` flag against the registry and report a clear error if the model is not found.
- **FR-004**: System MUST fall back to built-in defaults when no `models.yaml` is present (backward compatibility).
- **FR-005**: System MUST support Ollama as a local provider and at least one OpenAI-compatible cloud provider via Rig.
- **FR-006**: System MUST export OpenTelemetry traces via OTLP when the `--otlp` flag or `OTEL_EXPORTER_OTLP_ENDPOINT` environment variable is set.
- **FR-007**: System MUST instrument model calls with spans that include: model name, provider, prompt length, response length, and call duration.
- **FR-008**: System MUST gracefully handle OTLP collector unavailability — prompts succeed even when traces cannot be exported.
- **FR-009**: System MUST flush pending trace spans before process exit.
- **FR-010**: System MUST read cloud provider API keys from environment variables, never from the config file.

### Key Entities

- **ModelEntry**: A configured model — ID, provider name, locality (local/cloud), and optional metadata (context window, capabilities).
- **ModelRegistry**: Collection of ModelEntry values loaded from `models.yaml`, with lookup by model ID.
- **Provider**: An inference backend (e.g., Ollama, OpenAI). Each provider knows how to create a Rig client for its models.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can send prompts to any model listed in their configuration file by name.
- **SC-002**: Users can add a new model to the configuration and use it immediately without code changes.
- **SC-003**: Traces for model calls are visible in Jaeger within 5 seconds of prompt completion when OTLP is enabled.
- **SC-004**: Each trace span includes model name, provider, and timing information.
- **SC-005**: The CLI works identically to the previous sprint when no `models.yaml` exists and `--otlp` is not used.
- **SC-006**: Users can route prompts to both local (Ollama) and cloud (OpenAI-compatible) providers from the same CLI.

## Assumptions

- Ollama is the primary local provider; other local inference backends (mistral.rs) are deferred to later phases.
- Prescriptive routing only — the user explicitly picks the model via `--model` flag. Adaptive and hybrid routing are deferred to Phase 5.
- The OTLP endpoint defaults to `http://localhost:4317` (standard gRPC) when enabled.
- Jaeger all-in-one is the recommended trace viewer for development; any OTLP-compatible backend works.
- Cloud provider support is limited to OpenAI-compatible APIs via Rig's existing provider support. Adding more providers is future work.
- `models.yaml` lives in the current working directory or a path specified via `--config`. XDG/system-wide config paths are future work.
- Model capabilities and quality scores in the registry are informational only in this phase — they are not used for routing decisions until adaptive routing in Phase 5.
- GenAI semantic conventions for OpenTelemetry are followed where stable; experimental conventions are adopted with a note.
