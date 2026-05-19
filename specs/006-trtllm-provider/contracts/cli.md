# CLI Contract: TensorRT-LLM Provider

## Changes to Existing Commands

### `mv-cli prompt` / `mv-cli <prompt-text>`

No CLI flag changes. The `trtllm` provider is selected by configuring a model
with `provider: trtllm` in `models.yaml` and selecting it via `-m <model-id>`.

**New behavior**: When the selected model has `provider: trtllm`, the CLI:

1. Performs a health check against the TRT-LLM endpoint
2. If unhealthy, reports an actionable error and exits
3. If healthy, sends the prompt via the OpenAI-compatible API

**Error output** (stderr, when server is unavailable):

```text
Error: TRT-LLM server not reachable at http://localhost:8000
  Hint: Start the server with: trtllm-serve <model-path>
```

### `mv-cli workflow run`

No CLI flag changes. Workflow prompt steps that reference a `trtllm`-backed
model are automatically routed through the TRT-LLM provider.

### `mv-cli workflow validate`

No changes. Validation does not contact model backends.

## Model Configuration Contract

### models.yaml Schema

New optional fields on model entries (backward compatible):

```yaml
models:
  - id: <string>           # required — user-facing model name
    provider: <string>     # required — "ollama" | "openai" | "trtllm"
    endpoint: <string>     # optional — provider API base URL
    served_name: <string>  # optional — server-side model name (NEW)
    architecture: <string> # optional — model architecture (NEW)
    quant: <string>        # optional — quantization type (NEW)
    expected_vram_gb: <u32> # optional — expected VRAM in GB (NEW)
    api_key_env: <string>  # optional — env var for API key (openai only)
    default: <bool>        # optional — default model flag
    locality: <string>     # optional — "local" | "cloud" (auto-inferred)
```

### Provider Defaults

| Provider | Default Endpoint | API Key | Locality |
|----------|-----------------|---------|----------|
| ollama | `http://localhost:11434` | none | local |
| openai | `https://api.openai.com/v1` | from `api_key_env` | cloud |
| trtllm | `http://localhost:8000/v1` | `"tensorrt_llm"` (hardcoded) | local |

## Telemetry Contract

### Span Attributes for TRT-LLM Calls

| Attribute | Value | Condition |
|-----------|-------|-----------|
| `gen_ai.system` | `"trtllm"` | always |
| `gen_ai.request.model` | model ID | always |
| `trtllm.architecture` | architecture string | if configured |
| `trtllm.quant` | quantization string | if configured |
| `trtllm.expected_vram_gb` | VRAM in GB | if configured |
