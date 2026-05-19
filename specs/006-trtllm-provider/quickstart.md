# Quickstart: TensorRT-LLM Provider

## Prerequisites

- multae-viae built: `cargo build -p mv-cli`
- `trtllm-serve` installed (via TensorRT-LLM pip package or Docker)
- A model available (HuggingFace checkpoint or pre-built TRT-LLM engine)
- NVIDIA GPU with appropriate drivers

## 1. Start the TRT-LLM Server

```bash
# Serve a model from HuggingFace (downloads and builds automatically)
trtllm-serve meta-llama/Meta-Llama-3.1-8B-Instruct \
  --served_model_name llama-3_1-8b \
  --port 8000

# Or serve a pre-built engine from trt-llm-explore
trtllm-serve $TRTLLM_HOME/engines/llama-3_1-8b-fp8 \
  --served_model_name llama-3_1-8b-fp8 \
  --port 8000
```

Verify the server is healthy:

```bash
curl http://localhost:8000/health
```

## 2. Configure the Model

Add a `trtllm` model entry to `models.yaml`:

```yaml
models:
  - id: qwen3:8b
    provider: ollama
    default: true

  - id: llama-3_1-8b-fp8
    provider: trtllm
```

The `id` must match the `--served_model_name` used when starting the server.
If you did not set `--served_model_name`, use the full HuggingFace path and
add a `served_name` field:

```yaml
  - id: llama-fp8
    provider: trtllm
    served_name: meta-llama/Meta-Llama-3.1-8B-Instruct
```

## 3. Send a Prompt

```bash
cargo run -p mv-cli -- -m llama-3_1-8b-fp8 "Explain Rust ownership"
```

## 4. Use in a Workflow

Workflows can specify TRT-LLM models per step:

```yaml
name: research-trtllm
version: "1.0"
defaults:
  model: llama-3_1-8b-fp8
inputs:
  - name: topic
    type: string
    required: true
steps:
  - id: research
    type: prompt
    output: questions
    template: "Generate 5 research questions about: {{topic}}"
  - id: answers
    type: prompt
    output: answers
    template: "Answer these questions concisely:\n{{questions}}"
outputs:
  - name: answers
    from: answers
```

```bash
cargo run -p mv-cli -- workflow run workflow.yaml --input topic="GPU inference"
```

## 5. Verbose Logging with Telemetry

```bash
cargo run -p mv-cli -- -v -m llama-3_1-8b-fp8 "Hello"
```

With OTLP export:

```bash
cargo run -p mv-cli -- --otlp -m llama-3_1-8b-fp8 "Hello"
```

TRT-LLM spans will show `gen_ai.system = "trtllm"` in Jaeger.

## Common Issues

| Problem | Solution |
|---------|----------|
| `TRT-LLM server not reachable` | Start the server: `trtllm-serve <model>` |
| `model not found` on the server | Check `--served_model_name` matches `id` in `models.yaml` |
| `connection refused` on port 8000 | Verify the port matches the `endpoint` in `models.yaml` |
| Slow first response | TRT-LLM warms up on first request; subsequent requests are faster |
