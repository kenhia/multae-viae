# TensorRT-LLM Integration Assessment

> **Implementation note (sprint 006):** The initial implementation uses an
> OpenAI-compatible proxy from `trt-llm-explore` on port 8003 (not
> `trtllm-serve` on port 8000 as proposed below). The proxy normalizes
> array-format content, supports SSE streaming, tool calling, and approximate
> token counts. Rig's `CompletionsClient` is used instead of the default
> `openai::Client` to target `/v1/chat/completions`.

## Context

After completing Phase 4 (DSL Engine), we are evaluating whether to integrate
TensorRT-LLM (TRT-LLM) as an inference backend in multae-viae. This would
complement or partially replace the current Ollama-based local inference with
NVIDIA's optimized inference engine.

The evaluation is informed by the companion project `trt-llm-explore`, which has
already built a complete harness for building, serving, and evaluating TRT-LLM
models via Triton Inference Server on an RTX 5090 (32 GB VRAM).

## What trt-llm-explore Provides

The exploration project has validated:

- **Engine building**: Docker-based pipeline to convert HuggingFace models into
  optimized TRT-LLM engines (Llama, Qwen, Phi, Mistral architectures)
- **Quantization**: FP8 and INT4 AWQ quantization with calibration, producing
  significantly smaller VRAM footprints (e.g., Llama 3.1 8B: 16 GB FP16 → 9 GB
  FP8 → 6 GB AWQ)
- **Triton serving**: Docker Compose-based serving via Triton Inference Server
  with KServe v2 API, explicit model loading/unloading, VRAM management
- **Evaluation**: Latency benchmarking (mean/p50/p95/p99), throughput testing
  (concurrent requests), vision model support, cross-variant comparison reports
- **Model registry**: TOML-based configuration with validation, architecture
  detection, and capability tagging

### Models Already Validated

| Model | Architecture | Quant | VRAM | Capabilities |
|-------|-------------|-------|------|-------------|
| Llama 3.1 8B | llama | FP16 | 16 GB | chat |
| Llama 3.1 8B | llama | FP8 | 9 GB | chat |
| Llama 3.1 8B | llama | INT4 AWQ | 6 GB | chat |
| Qwen2.5 Coder 7B | qwen | FP16 | 14 GB | chat, code |
| Qwen2.5 Coder 7B | qwen | FP8 | 5 GB | chat, code |
| Mistral 7B | llama | FP16 | 14 GB | chat |
| Phi-3.5 mini | phi | FP16 | 8 GB | chat |
| LLaVA 1.5 7B | llama | FP16 | 15 GB | vision |
| Qwen2-VL 7B | qwen | FP16 | 22 GB | vision |

## Integration Feasibility: Very High

### Why This Is Straightforward

1. **`trtllm-serve` is OpenAI-compatible**: TRT-LLM's built-in serving command
   (`trtllm-serve`) exposes `/v1/chat/completions` and `/v1/completions`
   endpoints that are fully compatible with the OpenAI API protocol.

2. **Rig already supports custom OpenAI base URLs**: Our `call_openai()` in
   main.rs uses `rig::providers::openai::Client::builder().base_url(endpoint)`,
   which means we can point it at a TRT-LLM server with **zero protocol changes**.

3. **The model registry already supports custom endpoints**: `ModelEntry` has an
   `endpoint` field and the provider system is string-based. Adding
   `provider: "trtllm"` with `endpoint: "http://localhost:8000/v1"` fits the
   existing architecture.

### Minimal Integration Path

The simplest integration is almost trivial:

```yaml
# models.yaml
models:
  - id: llama-3_1-8b-fp8
    provider: trtllm
    endpoint: http://localhost:8000/v1
```

And in the CLI, treat `trtllm` provider the same as `openai` (since the API is
compatible):

```rust
"trtllm" => call_openai(&entry.id, &endpoint, "", prompt, agent_handle).await?,
```

This gets us running in hours, not weeks.

### Full Integration Path

A richer integration would add:

1. **Provider module in mv-core** — `trtllm` provider with:
   - Health checking via Triton's `/v2/health/ready` endpoint
   - Model loading/unloading via Triton's KServe v2 model management API
   - VRAM monitoring via `nvidia-smi` or Triton's `/metrics` endpoint
   - Automatic model loading when a workflow step requests a TRT-LLM model

2. **Model registry enhancement** — Additional fields for TRT-LLM models:
   - `architecture`, `quant`, `expected_vram_gb` (from trt-llm-explore's TOML)
   - Capability tags for routing decisions

3. **Locality inference** — `trtllm` maps to `Locality::Local`

4. **Telemetry** — TRT-LLM-specific span attributes:
   - Quantization type, engine variant
   - VRAM usage before/after inference
   - Triton metrics integration

5. **Tool calling support** — `trtllm-serve` supports tool parsing for Qwen3,
   DeepSeek, and other architectures via `--tool_parser`

## Advantages Over Ollama

| Aspect | Ollama | TRT-LLM |
|--------|--------|---------|
| **Throughput** | Good (llama.cpp) | Excellent (TensorRT kernels, in-flight batching) |
| **Latency** | Good | Better (optimized CUDA graphs, kernel fusion) |
| **Quantization control** | GGUF presets | FP8, INT4 AWQ with calibration |
| **Multi-model serving** | One-at-a-time (default) | Explicit load/unload, concurrent models |
| **VRAM management** | Automatic | Explicit, predictable |
| **Vision models** | Supported | Supported (two-stage pipeline) |
| **Batching** | Sequential | In-flight batching, continuous batching |
| **Setup complexity** | Very easy (`ollama pull`) | Complex (Docker, engine builds, model repo) |
| **Model ecosystem** | Huge (GGUF Hub) | Growing (HuggingFace + converter) |

### Key Insight

**These are complementary, not competing.** Ollama excels at convenience and
breadth of model support. TRT-LLM excels at performance and control. The right
answer is to support both:

- **Ollama** for quick experimentation, broad model access, and simple setups
- **TRT-LLM** for production workloads, benchmarked models, and when you need
  maximum throughput or specific quantization

## Risks and Considerations

1. **Hardware dependency**: TRT-LLM requires NVIDIA GPU with CUDA. Ollama
   supports CPU-only fallback. This is fine for our RTX 5090 target but reduces
   portability.

2. **Setup complexity**: Engine builds are slow (10-30 minutes per model) and
   require the Triton container. This is a one-time cost per model variant, but
   the operational overhead is real.

3. **Two-project coordination**: The engine building lives in `trt-llm-explore`.
   We need to decide whether to keep that separate or pull build orchestration
   into multae-viae. **Recommendation: Keep separate.** multae-viae only needs
   to *consume* pre-built engines via Triton, not *build* them.

4. **API key requirement**: `trtllm-serve` accepts any API key string (it's not
   validated). We just need to send a non-empty string.

5. **`trtllm-serve` vs Triton**: There are two serving paths:
   - **`trtllm-serve`** (newer): OpenAI-compatible, simpler setup, recommended
   - **Triton Inference Server** (used in trt-llm-explore): KServe v2 API,
     more control, existing infrastructure
   
   For multae-viae integration, `trtllm-serve` is preferred because it speaks
   OpenAI protocol. The trt-llm-explore project can evolve to use `trtllm-serve`
   as well.

## Roadmap Placement: Phase 4.5

### Why Not Later?

- The integration work is small (days, not weeks)
- It would immediately benefit Phase 5 (Advanced Routing) by giving the router
  a high-performance local backend with known capabilities
- Phase 5's adaptive routing benefits from having multiple backends with
  different performance profiles

### Why Not Part of Phase 5?

- Phase 5 is focused on routing algorithms and RAG — adding a new provider
  there muddies the scope
- TRT-LLM provider is independent of routing logic
- Testing TRT-LLM as a provider gives us benchmark data we can use to design
  Phase 5's routing algorithms

### Proposed Phase 4.5: TRT-LLM Provider (1-2 weeks)

**Goal**: Add TRT-LLM as a local inference provider alongside Ollama.

#### Tasks

1. Add `trtllm` provider to `Locality::from_provider()` → `Local`
2. Add `trtllm` default endpoint → `http://localhost:8000/v1`
3. Route `trtllm` provider through OpenAI-compatible client in CLI
4. Add `trtllm-serve` health check integration
5. Extend models.yaml schema with optional TRT-LLM metadata (architecture,
   quant, expected_vram_gb)
6. Add Triton model load/unload commands (CLI subcommands or automatic)
7. Instrument TRT-LLM calls with provider-specific telemetry attributes
8. Update documentation (models.yaml examples, setup guide)
9. End-to-end test: workflow with TRT-LLM model
10. Update trt-llm-explore to support `trtllm-serve` as alternative to Triton

#### Dependencies

- RTX 5090 with TRT-LLM engines already built (via trt-llm-explore)
- `trtllm-serve` running locally (or Triton with model loaded)

#### Deliverable

```yaml
# models.yaml
models:
  - id: qwen3:8b
    provider: ollama

  - id: llama-3_1-8b-fp8
    provider: trtllm
    endpoint: http://localhost:8000/v1
    # Optional TRT-LLM metadata
    architecture: llama
    quant: fp8
    expected_vram_gb: 9
```

```bash
# Start TRT-LLM server (from trt-llm-explore or trtllm-serve)
trtllm-serve meta-llama/Meta-Llama-3.1-8B-Instruct --port 8000

# Use with multae-viae
cargo run -p mv-cli -- -m llama-3_1-8b-fp8 "Explain Rust ownership"

# Use in a workflow
cargo run -p mv-cli -- workflow run workflows/examples/research.yaml \
  --input topic="Rust async"
```

## Decision

**Recommendation: Yes, integrate as Phase 4.5.**

The integration effort is low (OpenAI-compatible API means near-zero protocol
work), the benefit is high (access to optimized, quantized models with
predictable performance), and the timing is right (before routing work in
Phase 5 that would benefit from multiple provider backends).

## Sprint 007: Streaming & Hardening

Sprint 007 extends the TRT-LLM provider with token streaming, sharper
error mapping, telemetry, and stop-sequence plumbing.

### `--stream` flag

The `prompt` subcommand accepts `--stream` to print model output to
stdout as deltas arrive instead of buffering the full response. Streaming
is only supported for `provider: trtllm` entries; passing `--stream` on
any other provider exits with code 1 and the message
`streaming is only supported for TRT-LLM models in this release`.

Passing `--json --stream` together emits a warning to stderr and falls
back to buffered JSON output (`--json` wins).

```bash
mv-cli -m llama-fp8 --stream --no-tools "Explain Rust ownership"
```

### Streaming and tools: `--no-tools`

The TRT-LLM proxy streams a tool call as plain-text `content`
(`{"name": "file_list", ...}`) rather than as an executable `tool_calls`
delta, so no OpenAI client (rig included) can run a tool mid-stream — the
model just emits never-executed tool-call text and loops. Built-in and MCP
tools are attached to every request by default, so:

- **`--stream` (default, tools attached)** → falls back to the **buffered**
  path, which performs the real tool round-trip. A note is printed to
  stderr: `note: --stream falls back to buffered output because tools are
  attached ...`.
- **`--stream --no-tools`** → genuine token streaming with no tools
  attached (built-in *or* MCP). This is the form to use for streaming
  demos and for prompts that don't need tools.

`--no-tools` is a general `prompt` flag (it works for any provider); on
TRT-LLM it is what unlocks real streaming.

### `Run: just load <id>` hint on 502

When the TRT-LLM proxy is reachable but returns HTTP 502 (typically because
no model is loaded), the CLI maps the rig error to
`MvError::ModelNotLoaded` whose Display includes
`Run: just load <model-id>` as an actionable hint.

Two hardening fixes make this reliable against the real proxy:

- **502 classifier ordering.** The proxy's 502 body wraps a Triton
  `"...is not found"` string and rig surfaces it as an `HttpError`, both of
  which previously shadowed the 502 mapping (the hint never fired). The
  classifier now evaluates the TRT-LLM 502 branch first; a genuine
  connection refusal carries no `502` and still maps to
  `BackendUnreachable`.
- **Streaming preflight.** rig's streaming layer swallows a 502 (it logs an
  SSE parse error and ends the turn empty), so the streaming path queries
  `/v1/models` first (`trtllm::health::served_model_present`) and surfaces
  the same `just load` hint for an unloaded model instead of silent empty
  output.

The buffered, streaming, and workflow paths all share the one classifier.

### Token-usage telemetry attributes

Both the buffered and streaming TRT-LLM paths now record OTel
`gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens` span
attributes whenever the proxy supplies non-zero counts. The attributes
are also visible in stderr fmt output at `-vv` (which enables
`FmtSpan::CLOSE`). The fmt layer emits ANSI colour only when stderr is a
real terminal, so redirected/piped logs stay plain text (ANSI escapes
between a field name and its `=` previously corrupted log files and
broke substring tooling).

### Stop-sequence configuration

Each `provider: trtllm` model entry may declare a `stop_sequences:` list.
When omitted, multae-viae sends the provider-default set
(`</s>`, `<|im_end|>`, `<|eot_id|>`) via `additional_params: {"stop":
[...]}`. The helper lives in `mv_core::trtllm::stop` and is invoked by
both the buffered and streaming agent builders.

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
