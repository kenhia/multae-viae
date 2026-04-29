# Quickstart: Multi-Model Routing & OpenTelemetry

**Feature**: 002-multi-model-telemetry
**Date**: 2026-04-28

## Prerequisites

1. **Completed Sprint 001** — workspace builds, `just ci` passes
2. **Ollama** running locally with at least one model:
   ```bash
   ollama pull qwen3:4b
   ollama pull qwen3:8b   # optional second model
   ```
3. **Docker** (for Jaeger, optional):
   ```bash
   docker run -d --name jaeger \
     -p 16686:16686 \
     -p 4317:4317 \
     -p 4318:4318 \
     jaegertracing/jaeger:latest
   ```

## Configuration

Create a `models.yaml` in the project root:

```yaml
models:
  - id: qwen3:4b
    provider: ollama
    default: true

  - id: qwen3:8b
    provider: ollama

  # Uncomment to add cloud fallback (requires OPENAI_API_KEY env var)
  # - id: gpt-4o-mini
  #   provider: openai
  #   api_key_env: OPENAI_API_KEY
```

## Basic Usage

```bash
# Uses default model (qwen3:4b) — same as sprint 001
just run "What is Rust?"

# Select a specific model from the registry
cargo run -p mv-cli -- --model qwen3:8b "Explain ownership in Rust"

# JSON output
cargo run -p mv-cli -- --model qwen3:4b --json "Hello"

# No config file needed — falls back to built-in defaults
rm models.yaml
cargo run -p mv-cli -- "Still works without config"
```

## OpenTelemetry Tracing

```bash
# Enable OTLP export (Jaeger must be running on localhost:4318 for HTTP)
cargo run -p mv-cli -- --otlp http://localhost:4318 "What is Rust?"

# Or use environment variable
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318 cargo run -p mv-cli -- "What is Rust?"

# View traces in Jaeger
open http://localhost:16686
# Select service "mv-cli", find traces with gen_ai.completion spans
```

## Cloud Provider

```bash
# Set API key
export OPENAI_API_KEY="sk-..."

# Route to cloud model
cargo run -p mv-cli -- --model gpt-4o-mini "Hello from the cloud"

# Missing API key produces a clear error
unset OPENAI_API_KEY
cargo run -p mv-cli -- --model gpt-4o-mini "Hello"
# Error: API key required for openai. Set OPENAI_API_KEY environment variable.
```

## Development Commands

```bash
just build          # cargo build --workspace
just test           # cargo test --workspace
just check          # cargo clippy --all-targets --all-features -- -D warnings
just ci             # fmt --check + clippy + test
```

## Verification

```bash
# 1. Default model works (backward compatible)
cargo run -p mv-cli -- "Hello"

# 2. Model selection from registry
cargo run -p mv-cli -- --model qwen3:8b "Hello"

# 3. Unknown model error
cargo run -p mv-cli -- --model nonexistent "Hello"
# Error: Model 'nonexistent' not found in registry. Available: qwen3:4b, qwen3:8b

# 4. OTLP traces visible in Jaeger
cargo run -p mv-cli -- --otlp "Hello"
# Check Jaeger at http://localhost:16686

# 5. All tests pass
just ci
```
