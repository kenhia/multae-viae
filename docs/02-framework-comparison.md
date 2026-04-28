# Framework Comparison

## Rust AI/Agent Frameworks

### Rig (`rig-core`) — ⭐ Recommended

**Repository**: [github.com/0xPlaygrounds/rig](https://github.com/0xPlaygrounds/rig)
**Stars**: 7.1k | **License**: MIT | **Status**: Active (multiple releases/week)

Rig is the most complete Rust framework for building LLM-powered applications.
It provides the right abstraction level for this project — high enough to be
productive, low enough to customize.

**Strengths**:
- 20+ model providers under a unified interface (OpenAI, Anthropic, Ollama,
  Groq, Mistral, Gemini, etc.)
- 10+ vector store integrations (Qdrant, LanceDB, MongoDB, SQLite, etc.)
- Pipeline API for composing multi-step operations
- Built-in telemetry with OpenTelemetry GenAI semantic conventions
- Tool calling with `#[tool_macro]` procedural macro
- Agent builder pattern for ergonomic agent construction
- Streaming support
- WASM compatibility (core library)
- Active community with production users (St. Jude, Neon, Dria)

**Weaknesses**:
- Rapidly evolving API — breaking changes expected
- Less control over raw inference compared to lower-level options
- Relatively new (started ~2 years ago)

**Fit for this project**: **Excellent**. Rig handles the agent abstraction layer,
model provider integration, and RAG plumbing. We build the orchestration,
routing, DSL, and telemetry on top of it.

```rust
// Example: Creating an agent with tools
use rig::client::{CompletionClient, ProviderClient};
use rig::completion::Prompt;
use rig::providers::openai;

let client = openai::Client::from_env();
let agent = client
    .agent("gpt-4")
    .preamble("You are a helpful assistant.")
    .tool(my_custom_tool)
    .build();

let response = agent.prompt("What's the weather?").await?;
```

---

### Kalosm (Floneum)

**Repository**: [github.com/floneum/floneum](https://github.com/floneum/floneum)
**Stars**: 2.2k | **License**: Apache-2.0/MIT | **Status**: Active

Kalosm is a multi-modal Rust framework focused on local model execution. It
wraps Candle for inference and provides high-level APIs for text, audio, and
image models.

**Strengths**:
- Pure Rust inference via Candle
- Structured generation with `#[derive(Parse, Schema)]`
- Built-in RAG utilities (document extraction, chunking, vector DB)
- Audio transcription (Whisper)
- No external runtime dependencies

**Weaknesses**:
- Smaller community (2.2k stars, 20 contributors)
- Fewer model provider integrations than Rig
- No MCP support
- Performance lags behind llama.cpp-based solutions

**Fit for this project**: **Good for specific use cases** — particularly
structured generation and local-only scenarios. Could complement Rig for
constrained output generation.

---

### mistral.rs

**Repository**: [github.com/EricLBuehler/mistral.rs](https://github.com/EricLBuehler/mistral.rs)
**Stars**: 7.1k | **License**: MIT | **Status**: Very active

mistral.rs is a high-performance inference engine written in Rust. It's not an
agent framework — it's an inference server with agentic features bolted on.

**Strengths**:
- Best-in-class Rust-native inference performance
- Comprehensive model support (text, vision, video, audio, image generation)
- Built-in MCP client
- Server-side agentic loop with tool dispatch
- ISQ (in-situ quantization) — quantize any HuggingFace model
- Multi-GPU tensor parallelism
- PagedAttention for high throughput
- Hardware auto-tuning (`mistralrs tune`)
- Web UI included

**Weaknesses**:
- Primarily an inference engine, not an orchestration framework
- Less flexible for multi-provider routing
- Candle-based (inherits Candle's model support limitations)

**Fit for this project**: **Excellent as an inference backend**. Can be used
as an embedded inference engine alongside Ollama, or as a standalone server.
The built-in MCP client and agentic loop could serve as a reference
implementation.

---

### Candle (HuggingFace)

**Repository**: [github.com/huggingface/candle](https://github.com/huggingface/candle)
**Stars**: 20.1k | **License**: Apache-2.0/MIT | **Status**: Active

Candle is a minimalist ML framework for Rust — think PyTorch but in Rust.
It's the foundation that mistral.rs, Kalosm, and others build upon.

**Strengths**:
- Pure Rust tensor operations
- CUDA, Metal, and CPU backends
- WASM support
- Extensive model implementations in `candle-transformers`
- GGUF/safetensors/ONNX file format support
- PyTorch-like API

**Weaknesses**:
- Low-level — requires significant ML knowledge to use directly
- No agent/orchestration abstractions
- No built-in tool calling or MCP

**Fit for this project**: **Foundation layer**. Use for custom embedding
models, fine-tuning experiments, or when you need direct tensor operations.
Don't use as the primary agent framework.

---

### Archived/Deprecated (Avoid)

- **rustformers/llm** — Archived June 2024. Recommends Candle-based
  alternatives.
- **HuggingFace TGI** — Archived March 2026. Recommends vLLM, SGLang, or
  llama.cpp.

## Comparison Matrix

| Feature | Rig | Kalosm | mistral.rs | Candle |
|---------|-----|--------|------------|--------|
| Agent abstractions | ✅ Full | ⚠️ Basic | ⚠️ Server-side | ❌ |
| Multi-provider | ✅ 20+ | ❌ Local only | ❌ Local only | ❌ |
| MCP support | ❌ | ❌ | ✅ Client | ❌ |
| Tool calling | ✅ Macro-based | ❌ | ✅ Built-in | ❌ |
| RAG/Vector stores | ✅ 10+ | ✅ Built-in | ❌ | ❌ |
| Streaming | ✅ | ✅ | ✅ | ❌ |
| Telemetry | ✅ OTel | ❌ | ❌ | ❌ |
| Local inference | ✅ Via Ollama | ✅ Native | ✅ Native | ✅ Native |
| Cloud providers | ✅ | ❌ | ❌ | ❌ |
| Structured output | ⚠️ Via JSON schema | ✅ Grammar-based | ✅ Grammar-based | ❌ |
| Pipeline/workflow | ✅ Pipeline API | ❌ | ❌ | ❌ |
| Maturity | Medium | Early | Medium | High |
| Community size | Large (203 contribs) | Small (20) | Medium (80) | Large (258) |

## Recommendation

**Use Rig as the primary agent framework** and **Ollama + mistral.rs as
inference backends**.

This gives you:
1. **Rig**: Agent abstractions, multi-provider support, pipelines, tool calling,
   RAG integration, telemetry
2. **Ollama**: Easy model management, OpenAI-compatible API, broad model support
3. **mistral.rs**: High-performance embedded inference when you need it, MCP
   client reference
4. **RMCP**: Direct MCP protocol integration
5. **Candle**: Low-level tensor ops if needed for custom work

The key insight is that these are **complementary, not competing**. Rig handles
orchestration, Ollama/mistral.rs handle inference, RMCP handles MCP, and Candle
is the escape hatch for custom ML work.
