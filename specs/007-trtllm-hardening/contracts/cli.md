# CLI Contract: TRT-LLM Streaming & Hardening

This contract extends [sprint 006's CLI contract](../../006-trtllm-provider/contracts/cli.md).
Everything below is additive — no flag, schema field, or telemetry attribute
from sprint 006 is removed or repurposed.

## Changes to Existing Commands

### `mv-cli prompt` / `mv-cli <prompt-text>`

#### New flag

| Flag | Type | Default | Description |
|------|------|---------|-------------|
| `--stream` | bool | `false` | Stream tokens to stdout as they arrive. Currently only supported when the selected model has `provider: trtllm`. |

#### Stream behavior (TRT-LLM models)

1. Health check runs unchanged.
2. The CLI builds the agent against `openai::CompletionsClient` and calls
   `agent.stream_prompt(prompt).multi_turn(10).await`.
3. As assistant text deltas arrive, each delta is written to **stdout** and
   flushed immediately.
4. A single trailing newline is written after the stream terminates cleanly.
5. The `gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens` span
   attributes are recorded from the terminal stream item before the span
   closes.
6. Process exit code: `0` on clean stream termination, `1` on any stream
   error (including partial output already printed).

#### Stream behavior (non-TRT-LLM models)

Sprint 007 does not implement streaming for `ollama` or `openai`. Passing
`--stream` against such a model produces:

```text
Error: streaming is only supported for TRT-LLM models in this release
```

…on **stderr** with exit code `1`. (Future sprints may lift this.)

#### Interaction with `--json`

When both `--stream` and `--json` are set, the CLI emits the following
warning to **stderr** and falls back to the buffered JSON output path:

```text
warning: --json overrides --stream; falling back to buffered JSON output
```

Exit code follows the buffered path (`0` on success, non-zero on error).
Rationale: incremental token chunks are not valid JSON; emitting a
single JSON object after the full response is the only sensible
combination.

#### Non-stream behavior (no `--stream` flag)

Unchanged from sprint 006 — single buffered response with the same span
schema (now including the new `gen_ai.usage.*` attributes when the proxy
supplies them).

### `mv-cli workflow run`

No CLI flag changes. Workflow prompt steps that resolve to a TRT-LLM model
use the buffered path and benefit from:

- New `gen_ai.usage.*` span attributes
- New `ModelNotLoaded` error mapping (surfaced via `MvError` Display)
- Stop sequences forwarded on every request

`--stream` is **not** accepted on `workflow run` (research R8).

### `mv-cli workflow validate`

Unchanged.

## Error Output Contracts

### Model not loaded (HTTP 502 from proxy)

```text
Error: Model 'llama-fp8' is not loaded on the TRT-LLM proxy. Run: just load llama-fp8
```

- Emitted to **stderr**.
- Exit code `1`.
- The model identifier in the message is the configured `id` (not
  `served_name`) so the user runs the same name they typed.

### Proxy unreachable (connection refused)

Unchanged from sprint 006:

```text
Error: Cannot reach model backend at http://localhost:8003/v1. TRT-LLM server not reachable (<reason>). Start the server with: trtllm-serve <model-path>
```

### Other 5xx (e.g. HTTP 500)

Falls through the existing classifier:

```text
Error: Model returned an error: <stringified rig error including status code>
```

No `just load …` hint — that is reserved for 502.

### Stream interrupted mid-response

Partial assistant text remains on **stdout** (with no synthetic prefix or
suffix). The classified error is appended on **stderr**, exit code `1`.

## Model Configuration Contract

`models.yaml` gains one optional field on every model entry:

```yaml
models:
  - id: <string>
    provider: <string>
    # ... existing fields unchanged ...
    stop_sequences: [<string>, ...]  # optional (NEW)
```

Provider defaults applied when `stop_sequences` is omitted:

| Provider | Default Stop Sequences |
|----------|------------------------|
| trtllm | `["</s>", "<|im_end|>", "<|eot_id|>"]` |
| ollama | none (sprint 007 does not forward) |
| openai | none (sprint 007 does not forward) |

## Telemetry Contract

### Span attributes on TRT-LLM calls (`llm_completion` span)

| Attribute | Value | Condition |
|-----------|-------|-----------|
| `gen_ai.system` | `"trtllm"` | always |
| `gen_ai.request.model` | resolved model name (`served_name` if set, else `id`) | always |
| `trtllm.architecture` | string | if configured (unchanged) |
| `trtllm.quant` | string | if configured (unchanged) |
| `trtllm.expected_vram_gb` | u64 | if configured (unchanged) |
| `gen_ai.usage.input_tokens` | u64 | **NEW** — when proxy returns `usage.prompt_tokens` |
| `gen_ai.usage.output_tokens` | u64 | **NEW** — when proxy returns `usage.completion_tokens` |

The two new attributes apply identically to buffered and streaming calls.
When the proxy omits `usage`, the fields are absent (not zero) on the
exported span.
