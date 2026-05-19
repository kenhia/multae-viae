# Data Model: TensorRT-LLM Provider

## Modified Entities

### ModelEntry

Extended with optional fields for provider-specific metadata.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| id | String | yes | — | User-facing model identifier (CLI `-m` flag) |
| provider | String | yes | — | Provider name: `ollama`, `openai`, `trtllm` |
| locality | Locality? | no | inferred | `local` or `cloud` (inferred from provider) |
| api_key_env | String? | no | — | Env var name for API key (OpenAI only) |
| endpoint | String? | no | provider default | Base URL for the provider API |
| default | bool | no | false | Whether this is the default model |
| served_name | String? | no | — | Server-side model name (sent in API calls instead of `id`) |
| architecture | String? | no | — | Model architecture (e.g., `llama`, `qwen`, `phi`) |
| quant | String? | no | — | Quantization type (e.g., `fp16`, `fp8`, `int4_awq`) |
| expected_vram_gb | u32? | no | — | Expected VRAM usage in GB |

**New fields**: `served_name`, `architecture`, `quant`, `expected_vram_gb`

### Locality

No structural changes. Add `"trtllm"` to the `from_provider()` match as
`Local`.

| Provider | Default Locality | Default Endpoint |
|----------|-----------------|-----------------|
| ollama | Local | `http://localhost:11434` |
| openai | Cloud | `https://api.openai.com/v1` |
| trtllm | Local | `http://localhost:8000/v1` |

## New Types

### HealthCheckResult

Simple result type for the pre-prompt health check.

| Variant | Fields | Description |
|---------|--------|-------------|
| Healthy | — | Server responded with 200 |
| Unhealthy | status: u16, body: String | Server responded with non-200 |
| Unreachable | error: String | Connection failed |

## Configuration Examples

### Minimal TRT-LLM entry

```yaml
models:
  - id: llama-3_1-8b-fp8
    provider: trtllm
```

Uses default endpoint `http://localhost:8000/v1`. The `id` must match the
`--served_model_name` used when starting `trtllm-serve`.

### Full TRT-LLM entry with metadata

```yaml
models:
  - id: llama-fp8
    provider: trtllm
    endpoint: http://localhost:8000/v1
    served_name: meta-llama/Meta-Llama-3.1-8B-Instruct
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
```

Here `id` is a short user-friendly name, and `served_name` is the actual model
name expected by the server.

### Mixed provider configuration

```yaml
models:
  - id: qwen3:8b
    provider: ollama
    default: true

  - id: llama-fp8
    provider: trtllm
    served_name: meta-llama/Meta-Llama-3.1-8B-Instruct
    architecture: llama
    quant: fp8
    expected_vram_gb: 9

  - id: gpt-4o-mini
    provider: openai
    api_key_env: OPENAI_API_KEY
```
