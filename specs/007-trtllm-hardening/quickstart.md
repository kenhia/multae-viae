# Quickstart: TRT-LLM Streaming & Hardening

This quickstart picks up from
[sprint 006's quickstart](../006-trtllm-provider/quickstart.md). It assumes
you already have the TRT-LLM proxy running at `http://localhost:8003/v1`
(via `just up` in `trt-llm-explore`) and at least one model entry in
`models.yaml` with `provider: trtllm`.

## 1. Stream a prompt

```bash
cargo run -p mv-cli -- -m llama-fp8 --stream "Explain Rust ownership"
```

Tokens appear on stdout as the proxy emits them. The process exits `0`
on clean termination.

## 2. See the actionable error when a model is not loaded

With the proxy running but no model loaded:

```bash
cargo run -p mv-cli -- -m llama-fp8 "hi"
# → Error: Model 'llama-fp8' is not loaded on the TRT-LLM proxy. Run: just load llama-fp8
```

Load the model and retry:

```bash
just load llama-fp8   # in trt-llm-explore
cargo run -p mv-cli -- -m llama-fp8 "hi"
```

## 3. Configure per-model stop sequences

Append to `models.yaml`:

```yaml
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

Omitting `stop_sequences` keeps the provider default
(`["</s>", "<|im_end|>", "<|eot_id|>"]`) — sufficient for the shipped
models. No existing `models.yaml` needs to change.

## 4. Tool calling through TRT-LLM

Tool calling works exactly like the Ollama and OpenAI paths — no extra
flag, the built-in tools are auto-registered:

```bash
cargo run -p mv-cli -- -m llama-fp8 "list the files in the current directory"
# → assistant invokes the file_list tool, then answers with real file names
```

This also works with `--stream`:

```bash
cargo run -p mv-cli -- -m llama-fp8 --stream "list the files in the current directory"
```

The tool round-trip happens silently between assistant turns; the final
assistant text streams to stdout.

## 5. Token usage in telemetry

Run with OTLP export:

```bash
cargo run -p mv-cli -- --otlp -m llama-fp8 "Hello"
```

In Jaeger, open the `llm_completion` span. Alongside the existing
`gen_ai.system = "trtllm"` and `trtllm.*` attributes you will now see:

- `gen_ai.usage.input_tokens` = e.g. `12`
- `gen_ai.usage.output_tokens` = e.g. `48`

Both attributes also appear on streaming calls (recorded when the
terminal stream item arrives).

## 6. Use TRT-LLM in a workflow

`workflows/examples/research.yaml` (or any existing workflow) — just set
`defaults.model` to a TRT-LLM model:

```yaml
name: research-trtllm
version: "1.0"
defaults:
  model: llama-fp8
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: research
    type: prompt
    output: questions
    template: "Generate 5 research questions about: {{topic}}"
outputs:
  - name: questions
    from: questions
```

```bash
cargo run -p mv-cli -- workflow run workflows/examples/research-trtllm.yaml \
  --input topic="GPU inference"
```

Workflow steps use the buffered path. `--stream` is **not** valid on
`workflow run` in this release.

## 7. Run the test suite

Offline (no proxy required — runs the default test set):

```bash
just ci
```

Live tests against a running proxy with a loaded model:

```bash
cargo test --workspace -- --ignored
```

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `streaming is only supported for TRT-LLM models in this release` | Drop `--stream` or switch to a `provider: trtllm` model. |
| `Model '<id>' is not loaded …` | Run `just load <id>` in the `trt-llm-explore` checkout. |
| `Cannot reach model backend at http://localhost:8003/v1` | Start the proxy: `just up` in `trt-llm-explore`. |
| Stream prints partial text then errors | Network/proxy issue; the partial output is real and the trailing error explains the cause. Retry. |
| No `gen_ai.usage.*` attributes on the span | The proxy did not return a `usage` block. Check the proxy version. |
