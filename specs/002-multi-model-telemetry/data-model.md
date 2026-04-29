# Data Model: Multi-Model Routing & OpenTelemetry Observability

**Feature**: 002-multi-model-telemetry
**Date**: 2026-04-28

## Entities

### ModelEntry

A single model definition from the configuration file.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | String | Yes | Model identifier used in `--model` flag (e.g., `qwen3:8b`, `gpt-4o-mini`) |
| `provider` | String | Yes | Provider name: `ollama` or `openai` |
| `locality` | Locality | No | `local` or `cloud` (default: inferred from provider) |
| `api_key_env` | String | No | Environment variable name for API key (e.g., `OPENAI_API_KEY`). Required for cloud providers. |
| `endpoint` | String | No | Custom endpoint URL. Default varies by provider. |
| `default` | bool | No | Whether this is the default model (at most one). Default: `false`. |

### Locality (enum)

Classifies where the model runs.

| Variant | Description |
|---------|-------------|
| `local` | Running on this machine (Ollama, mistral.rs) |
| `cloud` | Remote API (OpenAI, Anthropic, etc.) |

Default inference: `ollama` → `local`, `openai` → `cloud`.

### ModelRegistry

In-memory collection of models loaded from configuration.

| Field | Type | Description |
|-------|------|-------------|
| `models` | Vec<ModelEntry> | All configured models |
| `default_model` | Option<String> | ID of the default model (from `default: true` entry, or first entry if none marked) |

**Operations**:
- `load(path) → Result<ModelRegistry>`: Parse YAML file into registry
- `get(id) → Option<&ModelEntry>`: Look up model by ID
- `default() → &ModelEntry`: Return the default model
- `built_in() → ModelRegistry`: Return hardcoded defaults (backward compat)

### BackendConfig (existing, extended)

The existing `BackendConfig` struct continues to hold runtime connection
settings. It is populated from the resolved `ModelEntry` at runtime.

No structural changes — the registry resolves which model to use, then
`BackendConfig` is built from the selected `ModelEntry`.

## Configuration File Format

```yaml
# models.yaml
models:
  - id: qwen3:4b
    provider: ollama
    default: true

  - id: qwen3:8b
    provider: ollama

  - id: gpt-4o-mini
    provider: openai
    api_key_env: OPENAI_API_KEY
    endpoint: https://api.openai.com/v1
```

### Validation Rules

1. `id` must be unique across all entries (warn on duplicates, first wins)
2. `provider` must be one of: `ollama`, `openai`
3. At most one entry may have `default: true`
4. Cloud providers must have `api_key_env` set
5. If no `default: true` entry, the first model in the list is the default
6. If no `endpoint` is specified, use provider defaults:
   - `ollama`: `http://localhost:11434`
   - `openai`: `https://api.openai.com/v1`

## New Error Variants

Added to `MvError` in `mv-core`:

| Variant | Message | When |
|---------|---------|------|
| `ConfigParseError { path, details }` | `"Failed to parse config '{path}': {details}"` | YAML parse failure or validation error |
| `ModelNotInRegistry { model }` | `"Model '{model}' not found in registry. Available: ..."` | `--model` specifies unknown ID |
| `ApiKeyMissing { provider, env_var }` | `"API key required for {provider}. Set {env_var} environment variable."` | Cloud provider selected without API key |

## Telemetry Span Attributes

Model call spans (`gen_ai.completion`) include these attributes:

| Attribute | Type | Source |
|-----------|------|--------|
| `gen_ai.system` | String | Provider name from registry (e.g., `ollama`, `openai`) |
| `gen_ai.request.model` | String | Model ID from registry |
| `gen_ai.request.prompt_length` | u64 | Character count of prompt |
| `gen_ai.response.length` | u64 | Character count of response |
| `mv.model.locality` | String | `local` or `cloud` |
| `mv.model.endpoint` | String | Endpoint URL used |

## State Transitions

```
CLI startup
  │
  ├─ --config <path> set? ──► Load YAML from path
  │                              │
  │                              ├─ Parse OK ──► ModelRegistry
  │                              └─ Parse fail ──► ConfigParseError (exit 1)
  │
  ├─ ./models.yaml exists? ──► Load YAML from ./models.yaml
  │                              │
  │                              ├─ Parse OK ──► ModelRegistry
  │                              └─ Parse fail ──► ConfigParseError (exit 1)
  │
  └─ No config found ──► ModelRegistry::built_in() (backward compat)
       │
       ▼
  Resolve model:
    --model flag set? ──► registry.get(model_id)
                            ├─ Found ──► ModelEntry
                            └─ Not found ──► ModelNotInRegistry (exit 1)
    No --model flag ──► registry.default()
       │
       ▼
  Check API key (if cloud):
    api_key_env set? ──► Read env var
                           ├─ Present ──► Proceed
                           └─ Missing ──► ApiKeyMissing (exit 1)
       │
       ▼
  Create provider client ──► Send prompt ──► Print response
```
