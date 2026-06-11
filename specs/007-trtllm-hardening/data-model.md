# Data Model: TRT-LLM Streaming & Hardening

## Modified Entities

### `mv_core::ModelEntry`

Add one optional field — non-breaking with respect to all existing
`models.yaml` files.

| Field | Type | Required | Default | Description |
|-------|------|----------|---------|-------------|
| id | String | yes | — | (unchanged) user-facing model identifier |
| provider | String | yes | — | (unchanged) `ollama` \| `openai` \| `trtllm` |
| locality | Locality? | no | inferred | (unchanged) |
| api_key_env | String? | no | — | (unchanged) |
| endpoint | String? | no | provider default | (unchanged) |
| default | bool | no | false | (unchanged) |
| served_name | String? | no | — | (unchanged) server-side name |
| architecture | String? | no | — | (unchanged) |
| quant | String? | no | — | (unchanged) |
| expected_vram_gb | u32? | no | — | (unchanged) |
| **stop_sequences** | **Vec<String>?** | **no** | **None → provider default applied at request time** | **NEW** — per-model stop sequences forwarded to the proxy as the OpenAI `stop` request parameter |

Helper to be added:

```rust
impl ModelEntry {
    /// Effective stop sequences for this model.
    /// Returns the explicit list if set, otherwise the provider-level default
    /// (currently TRT-LLM only — None for other providers).
    pub fn effective_stop_sequences(&self) -> Option<Vec<String>> { ... }
}
```

### `mv_core::MvError`

Add one new variant (kept distinct from `BackendUnreachable` per research R3):

| Variant | Fields | Display format |
|---------|--------|----------------|
| `ModelNotLoaded` | `model: String`, `hint: String` | `"Model '{model}' is not loaded on the TRT-LLM proxy. {hint}"` |

The `hint` is always populated with `"Run: just load <model>"` by the
CLI mapper; the field is kept generic so future callers can override.

## New Types (in `mv_core::trtllm`)

### `mv_core::trtllm::usage::Usage`

A small, deserializer-tolerant pair of counters extracted from proxy
responses. Lives in `mv-core` so both the buffered and streaming paths
share the extraction logic and serde quirks (R5).

| Field | Type | Source |
|-------|------|--------|
| input_tokens | u64 | proxy `usage.prompt_tokens` |
| output_tokens | u64 | proxy `usage.completion_tokens` |

```rust
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Usage {
    /// Pull usage out of a Rig response that implements GetTokenUsage,
    /// coercing any float/string proxy quirks to u64.
    pub fn from_rig<R: rig::completion::GetTokenUsage>(r: &R) -> Option<Self>;
}
```

A `Deserialize` impl tolerates `prompt_tokens` arriving as integer, float,
or numeric string (spec edge case "floats or strings").

### `mv_core::trtllm::stop`

| Item | Kind | Description |
|------|------|-------------|
| `default_stop_sequences()` | `fn() -> Vec<String>` | Provider-level default (`["</s>", "<|im_end|>", "<|eot_id|>"]`) |
| `request_stop_value(entry)` | `fn(&ModelEntry) -> Option<serde_json::Value>` | Builds the JSON `{"stop": [...]}` snippet for `additional_params`, returning `None` when the merged list is empty (currently never, but future-proof) |

## CLI types (in `crates/mv-cli/src/main.rs`)

### `PromptArgs` — extend

Add one flag:

```rust
/// Stream tokens to stdout as they arrive (TRT-LLM models only).
#[arg(long)]
stream: bool,
```

`WorkflowRunArgs` is **not** extended (research R8).

## Telemetry Schema

### Existing span — extended

The `#[tracing::instrument]` on `call_trtllm()` (and the new
`stream_trtllm()`) gains two fields, declared up-front as `Empty` so they
are part of the span schema even when the proxy omits usage data:

| Attribute | Type | When populated |
|-----------|------|----------------|
| `gen_ai.system` | string | always — `"trtllm"` (unchanged) |
| `gen_ai.request.model` | string | always — `entry.model_name()` (unchanged) |
| `trtllm.architecture` | string | always (unchanged) |
| `trtllm.quant` | string | always (unchanged) |
| `trtllm.expected_vram_gb` | u64 | always (unchanged) |
| **`gen_ai.usage.input_tokens`** | **u64** | **when proxy returns `usage.prompt_tokens`** |
| **`gen_ai.usage.output_tokens`** | **u64** | **when proxy returns `usage.completion_tokens`** |

No new spans are introduced — the existing `llm_completion` span covers both
buffered and streaming paths.

## Configuration Examples

### Minimal — uses default stop sequences

```yaml
models:
  - id: llama-fp8
    provider: trtllm
    served_name: llama-3_1-8b-fp8
```

### With per-model overrides

```yaml
models:
  - id: llama-fp8
    provider: trtllm
    served_name: llama-3_1-8b-fp8
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
    stop_sequences:
      - "<|eot_id|>"
      - "<|end_of_text|>"
```

### Mixed registry — only TRT-LLM gets default stop sequences

```yaml
models:
  - id: qwen3:8b
    provider: ollama
    default: true
    # stop_sequences: irrelevant for ollama in this sprint

  - id: llama-fp8
    provider: trtllm
    served_name: llama-3_1-8b-fp8
    # stop_sequences omitted → provider default applied at request time
```
