# Multae Viae — Research Overview

> *"Many paths"* — an agentic controller that explores multiple approaches to
> AI-assisted task execution.

## Project Vision

Build a local-first agentic controller in Rust that can:

- Load and run LLMs locally, with the ability to fall back to cloud providers
  for larger or proprietary models
- Orchestrate multi-step workflows using tools, MCP servers, and RAG
- Switch models dynamically based on task requirements
- Expose first-class telemetry for a companion dashboard
- Define agent workflows via a YAML-based DSL
- Operate as an always-on local assistant: monitoring, task handling, and
  acting as a "second brain"

## Research Summary

This document set covers the following areas:

| Document | Topic |
|----------|-------|
| [01 — Architecture Design](01-architecture-design.md) | High-level system architecture, component breakdown, data flow |
| [02 — Framework Comparison](02-framework-comparison.md) | Evaluation of Rust AI/agent frameworks (Rig, Kalosm, mistral.rs, etc.) |
| [03 — Local Inference](03-local-inference.md) | Options for running models locally (Ollama, llama.cpp, Candle, mistral.rs) |
| [04 — MCP Integration](04-mcp-integration.md) | Model Context Protocol architecture and the Rust SDK (RMCP) |
| [05 — Telemetry & Observability](05-telemetry-observability.md) | OpenTelemetry in Rust, tracing, metrics, and dashboard integration |
| [06 — DSL & Flow Management](06-dsl-flow-management.md) | YAML-based DSL design for prompt/flow orchestration |
| [07 — Model Routing](07-model-routing.md) | Strategies for dynamic model selection and routing |
| [08 — RAG Integration](08-rag-integration.md) | Retrieval-Augmented Generation patterns and network RAG |
| [09 — Roadmap](09-roadmap.md) | Phased implementation plan |

## Key Decisions & Recommendations

### Primary Language: Rust

Rust is the right choice here. The ecosystem for AI/ML in Rust has matured
significantly:

- **Candle** (HuggingFace) — Pure Rust ML framework, 20k+ stars, actively
  maintained, GPU support via CUDA/Metal
- **Rig** — Purpose-built Rust library for LLM-powered applications with 20+
  provider integrations, built-in telemetry, and RAG support
- **RMCP** — Official Rust MCP SDK from the Model Context Protocol organization
- **mistral.rs** — High-performance Rust inference engine with built-in MCP
  client, tool calling, and agentic loops
- **OpenTelemetry Rust** — Production-grade telemetry with traces, metrics, and
  logs

### Recommended Architecture Stack

| Layer | Recommendation | Rationale |
|-------|---------------|-----------|
| **Agent Framework** | **Rig** (`rig-core`) | Most complete Rust agent framework; 20+ providers, pipelines, tool calling, RAG, telemetry, active community |
| **Local Inference** | **Ollama** (primary) + **mistral.rs** (embedded) | Ollama for ease of model management; mistral.rs for embedded Rust-native inference |
| **MCP** | **RMCP** (`rmcp` crate) | Official Rust SDK, mature, supports stdio + HTTP transports |
| **Telemetry** | **tracing** + **opentelemetry-rust** + **tracing-opentelemetry** | Industry standard; bridges Rust tracing to OTel collectors |
| **ML Tensors** | **Candle** | For any custom model work, embeddings, or fine-tuning |
| **DSL Parser** | **serde_yaml** + custom types | Leverage Rust's type system for validated DSL configurations |
| **Vector Store** | **Qdrant** or **LanceDB** (via Rig integrations) | Both have Rig integration crates ready to use |

### Why Not Pure Python/TypeScript?

- **Performance**: Rust's zero-cost abstractions and memory safety make it ideal
  for an always-on agent that must be resource-efficient
- **Reliability**: No GC pauses, no runtime surprises — critical for a
  long-running local service
- **Ecosystem**: The Rust AI ecosystem has reached a tipping point with
  production-grade crates
- **Learning**: Building in Rust forces deeper understanding of the underlying
  mechanics, which aligns with the learning goals

## Research Date

This research was conducted on **April 28, 2026**. The AI/ML ecosystem moves
fast — framework versions, capabilities, and recommendations should be
re-evaluated periodically.
