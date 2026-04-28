# Local Inference Options

## Overview

Local inference is the foundation of the "local-first" philosophy. The goal is
to run models on your own hardware for privacy, latency, and cost reasons, while
retaining the ability to call cloud models when local capacity is insufficient.

## Option 1: Ollama — ⭐ Recommended Primary

**Repository**: [github.com/ollama/ollama](https://github.com/ollama/ollama)
**Stars**: 170k | **Written in**: Go (wraps llama.cpp)

Ollama is the de facto standard for local LLM management and inference. It
provides a Docker-like experience for models: `ollama pull`, `ollama run`.

### Why Ollama as Primary

1. **Model management**: Pull, list, remove models with simple commands
2. **OpenAI-compatible REST API**: Drop-in replacement for cloud providers
3. **Broad model support**: Llama, Gemma, Qwen, DeepSeek, Mistral, Phi, and
   hundreds more via GGUF
4. **Automatic GPU detection**: CUDA, Metal, ROCm support out of the box
5. **Rig integration**: Rig has a native Ollama provider (`rig::providers::ollama`)
6. **Always running**: Ollama runs as a system service, always ready
7. **Modelfile**: Customizable model configurations (system prompts, parameters)

### API Example

```bash
# Pull a model
ollama pull qwen3:4b

# Chat via REST API
curl http://localhost:11434/api/chat -d '{
  "model": "qwen3:4b",
  "messages": [{"role": "user", "content": "Hello!"}],
  "stream": false
}'
```

### Integration with Rig

```rust
use rig::providers::ollama;
use rig::completion::Prompt;
use rig::client::{CompletionClient, ProviderClient};

let client = ollama::Client::new("http://localhost:11434");
let agent = client.agent("qwen3:4b")
    .preamble("You are a helpful assistant.")
    .build();

let response = agent.prompt("Explain Rust's ownership model").await?;
```

### Limitations

- Written in Go, not Rust (minor concern — used as a service)
- Less control over inference parameters than direct llama.cpp
- Single-model-at-a-time by default (can be configured)

---

## Option 2: mistral.rs — Embedded Rust Inference

**Repository**: [github.com/EricLBuehler/mistral.rs](https://github.com/EricLBuehler/mistral.rs)
**Stars**: 7.1k | **Written in**: Rust (Candle-based)

mistral.rs provides a Rust-native inference engine that can be embedded
directly in the controller binary or run as a separate server.

### Why mistral.rs as Secondary

1. **Pure Rust**: Embeds directly in your binary — no external process
2. **Hardware auto-tuning**: `mistralrs tune` benchmarks your hardware
3. **ISQ**: Quantize any HuggingFace model to optimal format automatically
4. **MCP client built-in**: Ready for tool-calling workflows
5. **Agentic loop**: Server-side tool execution and result feeding
6. **Multi-model**: Load/unload models at runtime
7. **OpenAI-compatible API**: Same interface as Ollama

### Rust SDK Example

```rust
use mistralrs::{IsqType, TextMessageRole, TextMessages, MultimodalModelBuilder};

let model = MultimodalModelBuilder::new("google/gemma-4-E4B-it")
    .with_isq(IsqType::Q4K)
    .with_logging()
    .build()
    .await?;

let messages = TextMessages::new()
    .add_message(TextMessageRole::User, "Hello!");

let response = model.send_chat_request(messages).await?;
```

### When to Use mistral.rs Over Ollama

- When you need embedded inference (no external process)
- For specialized quantization control (per-layer topology)
- When building the inference directly into the controller binary
- For multi-GPU tensor parallelism scenarios

---

## Option 3: llama.cpp — Direct FFI

**Repository**: [github.com/ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp)
**Stars**: 107k | **Written in**: C/C++

The original and most widely-used local inference engine. Ollama wraps it
internally. Direct usage offers maximum control.

### Rust Bindings

Several Rust wrapper crates exist:

| Crate | Approach | Status |
|-------|----------|--------|
| `llama-cpp-2` | Raw bindings (follows C++ API) | Active |
| `llama_cpp` | Safe high-level wrapper | Active |
| `drama_llama` | Rust-idiomatic high-level wrapper | Active |

### When to Use Direct llama.cpp

- When you need cutting-edge features before they appear in Ollama
- For maximum performance tuning
- For custom sampling strategies
- For RPC-based distributed inference across machines

### Recommendation

**Don't start here.** Use Ollama or mistral.rs first. Drop to direct llama.cpp
bindings only if you need features those don't expose.

---

## Option 4: Candle — Custom Model Execution

Use Candle directly when you need to:

- Run custom model architectures not supported elsewhere
- Implement custom inference pipelines
- Build embedding models for RAG
- Experiment with model internals for learning

---

## Cloud Fallback Providers

When local models are insufficient (parameter count, specialized capabilities):

| Provider | Access via Rig | Models |
|----------|---------------|--------|
| OpenAI | ✅ `rig::providers::openai` | GPT-4, GPT-4o, o1-preview |
| Anthropic | ✅ `rig::providers::anthropic` | Claude 3.5, Claude 4 |
| Google | ✅ `rig::providers::gemini` | Gemini 2.5 Pro/Flash |
| Groq | ✅ `rig::providers::groq` | Fast inference of open models |
| Together | ✅ `rig::providers::together` | Wide open model selection |
| DeepSeek | ✅ `rig::providers::deepseek` | DeepSeek R1, V3 |
| OpenRouter | ✅ `rig::providers::openrouter` | Aggregator — access many providers |

## Model Selection Guide

| Task | Recommended Local Model | Size | Notes |
|------|------------------------|------|-------|
| General chat | Qwen 3 4B/8B | 4-8B | Good all-rounder |
| Code generation | Qwen 2.5 Coder 7B | 7B | Specialized for code |
| Reasoning | Phi-4 14B | 14B | Strong reasoning |
| Embeddings | nomic-embed-text | 137M | Fast, good quality |
| Tool calling | Qwen 3 8B | 8B | Strong function calling |
| Summarization | Gemma 3 4B | 4B | Efficient for summaries |
| Vision | Gemma 4 E4B | 4B | Multimodal text+image |

## Hardware Considerations

### Minimum Viable Setup
- 16GB RAM, modern CPU
- Can run 4B-8B quantized models (Q4_K_M) comfortably
- Ollama + Rig is sufficient

### Recommended Setup
- 32GB+ RAM, GPU with 8GB+ VRAM (or Apple Silicon with unified memory)
- Can run 14B-30B quantized models
- Enables multiple model loading

### Power User Setup
- 64GB+ RAM, GPU with 24GB+ VRAM
- Can run 70B+ quantized models
- Multiple simultaneous models
- Tensor parallelism across GPUs
