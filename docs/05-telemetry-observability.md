# Telemetry & Observability

## Philosophy

Telemetry is a **first-class requirement**, not an afterthought. Every
meaningful operation should emit traces, metrics, and structured logs. This
enables:

1. **Debugging**: Understand exactly what the agent did, which model it called,
   which tools it invoked, and why
2. **Performance**: Identify bottlenecks (slow model calls, network latency to
   RAG, tool execution time)
3. **Learning**: Replay agent sessions to understand decision patterns
4. **Dashboard**: Feed a companion development/debugging dashboard (separate
   project)

## Architecture

```
┌─────────────────────────────────────────────────┐
│               Multae Viae Controller            │
│                                                 │
│  ┌──────────────────────────────────────────┐   │
│  │           tracing (Rust crate)           │   │
│  │  Structured spans, events, fields        │   │
│  └─────────────────┬────────────────────────┘   │
│                    │                            │
│  ┌─────────────────▼────────────────────────┐   │
│  │       tracing-subscriber layers          │   │
│  │                                          │   │
│  │  ┌─────────────┐   ┌───────────────────┐ │   │
│  │  │   Console   │   │ tracing-          │ │   │
│  │  │   Output    │   │ opentelemetry     │ │   │
│  │  │  (fmt layer)│   │ (OTel layer)      │ │   │
│  │  └─────────────┘   └───────┬───────────┘ │   │
│  └────────────────────────────┼─────────────┘   │
│                               │                 │
│  ┌────────────────────────────▼──────────────┐  │
│  │         OpenTelemetry SDK                 │  │
│  │  ┌──────────┐ ┌──────────┐ ┌───────────┐  │  │
│  │  │  Traces  │ │ Metrics  │ │   Logs    │  │  │
│  │  └────┬─────┘ └────┬─────┘ └─────┬─────┘  │  │
│  └───────┼─────────────┼─────────────┼───────┘  │
└──────────┼─────────────┼─────────────┼──────────┘
           │             │             │
           ▼             ▼             ▼
    ┌──────────────────────────────────────┐
    │       OTLP Exporter (gRPC/HTTP)      │
    └──────────────────┬───────────────────┘
                       │
                       ▼
    ┌──────────────────────────────────────┐
    │     OpenTelemetry Collector          │
    │     (or direct to backend)           │
    └──────────────────┬───────────────────┘
                       │
           ┌───────────┼───────────┐
           ▼           ▼           ▼
     ┌──────────┐ ┌──────────┐ ┌──────────┐
     │  Jaeger  │ │Prometheus│ │Dashboard │
     │ (traces) │ │(metrics) │ │ (custom) │
     └──────────┘ └──────────┘ └──────────┘
```

## Rust Telemetry Stack

### Core Crates

| Crate | Purpose | Version |
|-------|---------|---------|
| `tracing` | Structured instrumentation framework | Stable |
| `tracing-subscriber` | Composable subscriber/layer system | Stable |
| `tracing-opentelemetry` | Bridge tracing spans → OTel spans | 0.32.x |
| `opentelemetry` | OTel API (traces, metrics, logs) | 0.29.x (API stable) |
| `opentelemetry-sdk` | OTel SDK implementation | Stable for logs/metrics |
| `opentelemetry-otlp` | OTLP exporter (gRPC and HTTP) | RC |
| `opentelemetry-appender-tracing` | Bridge tracing logs → OTel logs | Stable |
| `opentelemetry-semantic-conventions` | Standard attribute names | Stable |

### Setup Example

```rust
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::WithExportConfig;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

fn init_telemetry() -> anyhow::Result<()> {
    // OTLP exporter for traces
    let otlp_exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_tonic()
        .with_endpoint("http://localhost:4317")
        .build()?;

    let tracer_provider = SdkTracerProvider::builder()
        .with_batch_exporter(otlp_exporter)
        .build();

    let tracer = tracer_provider.tracer("multae-viae");

    // OpenTelemetry layer for tracing
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    // Console output layer
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_ids(true);

    // Compose layers
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(fmt_layer)
        .with(otel_layer)
        .init();

    Ok(())
}
```

## What to Instrument

### Workflow Execution

```rust
#[tracing::instrument(
    name = "workflow.execute",
    fields(
        workflow.name = %workflow.name,
        workflow.id = %workflow.id,
        workflow.steps = workflow.steps.len(),
    )
)]
async fn execute_workflow(&self, workflow: &Workflow) -> Result<Output> {
    // Each step gets its own child span
    for step in &workflow.steps {
        self.execute_step(step).await?;
    }
}
```

### Model Calls (GenAI Semantic Conventions)

Rig already supports OpenTelemetry GenAI semantic conventions. The key
attributes to capture:

| Attribute | Example | Description |
|-----------|---------|-------------|
| `gen_ai.system` | `openai` | Provider name |
| `gen_ai.request.model` | `gpt-4` | Requested model |
| `gen_ai.response.model` | `gpt-4-0613` | Actual model used |
| `gen_ai.usage.input_tokens` | `150` | Prompt tokens |
| `gen_ai.usage.output_tokens` | `42` | Completion tokens |
| `gen_ai.request.temperature` | `0.7` | Temperature setting |
| `gen_ai.response.finish_reason` | `stop` | Why generation stopped |

```rust
#[tracing::instrument(
    name = "gen_ai.completion",
    fields(
        gen_ai.system = %provider,
        gen_ai.request.model = %model,
        gen_ai.usage.input_tokens,
        gen_ai.usage.output_tokens,
    )
)]
async fn call_model(&self, provider: &str, model: &str, prompt: &str) -> Result<String> {
    let response = self.client.complete(prompt).await?;

    // Record token usage
    tracing::Span::current()
        .record("gen_ai.usage.input_tokens", response.usage.input_tokens)
        .record("gen_ai.usage.output_tokens", response.usage.output_tokens);

    Ok(response.text)
}
```

### Tool Invocations

```rust
#[tracing::instrument(
    name = "tool.call",
    fields(
        tool.name = %name,
        tool.source,          // "mcp", "built-in", "custom"
        tool.duration_ms,
        tool.success,
    )
)]
async fn call_tool(&self, name: &str, args: Value) -> Result<ToolResult> {
    let start = Instant::now();
    let result = self.registry.execute(name, args).await;

    let span = tracing::Span::current();
    span.record("tool.duration_ms", start.elapsed().as_millis() as u64);
    span.record("tool.success", result.is_ok());

    result
}
```

### Model Routing Decisions

```rust
#[tracing::instrument(
    name = "router.select",
    fields(
        router.strategy,       // "prescriptive", "adaptive", "hybrid"
        router.selected_model,
        router.candidates,
        router.reason,
    )
)]
async fn select_model(&self, task: &Task) -> Result<ModelSelection> {
    // ...routing logic...
}
```

## Custom Metrics

Define custom metrics for the dashboard:

```rust
use opentelemetry::metrics::{Counter, Histogram, Meter};

struct ControllerMetrics {
    requests_total: Counter<u64>,
    model_call_duration: Histogram<f64>,
    tool_call_duration: Histogram<f64>,
    tokens_used: Counter<u64>,
    active_workflows: UpDownCounter<i64>,
    cache_hits: Counter<u64>,
    cache_misses: Counter<u64>,
}
```

## Dashboard Integration

The companion dashboard project will consume telemetry data via:

1. **OTLP**: Direct from the controller or via an OTel Collector
2. **Prometheus scrape endpoint**: For metrics (if using Prometheus backend)
3. **WebSocket**: For real-time streaming of events

### Recommended Backend Stack for Dashboard

| Component | Purpose |
|-----------|---------|
| **Jaeger** or **Tempo** | Trace storage and querying |
| **Prometheus** | Metrics storage and alerting |
| **Loki** | Log aggregation |
| **Grafana** | Visualization (or custom dashboard) |

Alternatively, for a simpler setup during development:

| Component | Purpose |
|-----------|---------|
| **opentelemetry-stdout** | Print traces/metrics/logs to console |
| **Jaeger all-in-one** | Single Docker container for trace visualization |

### Development Setup

```bash
# Start Jaeger all-in-one for trace visualization
docker run -d --name jaeger \
  -p 16686:16686 \    # Jaeger UI
  -p 4317:4317 \      # OTLP gRPC
  -p 4318:4318 \      # OTLP HTTP
  jaegertracing/jaeger:latest
```

Then point the controller's OTLP exporter at `http://localhost:4317` and
view traces at `http://localhost:16686`.

## Observability Tools Worth Exploring

| Tool | Type | Notes |
|------|------|-------|
| **Langfuse** | LLM observability | Open source, AI-specific traces/evals |
| **OpenLIT** | OTel-native monitoring | Purpose-built for LLMs and GPUs |
| **Opik** | Debug & evaluate | LLM application monitoring |
| **MLflow Tracing** | ML observability | Open source, auto-tracing |
| **Lunary** | LLM analytics | PII masking, cost tracking |

These can be explored later for the dashboard project, but the core OTel
instrumentation in the controller should make it compatible with any of them.
