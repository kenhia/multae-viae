# Model Routing Strategies

## The Problem

Different tasks benefit from different models. A code generation task may
perform best on Qwen 2.5 Coder, while a reasoning task needs Phi-4 or GPT-4,
and a simple summarization runs fine on a small 4B model. The model router
decides which model to use for each step.

## Three Routing Strategies

### 1. Prescriptive Routing

The workflow DSL explicitly specifies which model to use for each step. No
intelligence in the router — it's a simple lookup.

```yaml
steps:
  - id: generate_code
    type: prompt
    model: qwen2.5-coder:7b    # Exact model
    template: "Write a function that..."
```

**Pros**: Deterministic, predictable, easy to debug
**Cons**: Rigid, doesn't adapt to availability or load
**Best for**: Production workflows where consistency matters

### 2. Adaptive Routing

The router selects the model based on task metadata, available models, and
runtime conditions. The DSL provides constraints and hints, but the router
makes the final decision.

```yaml
steps:
  - id: analyze
    type: prompt
    model:
      strategy: adaptive
      constraints:
        capabilities: [tool_calling]
        min_context_window: 8192
        locality: local
      hints:
        domain: code
        complexity: high
```

**Pros**: Flexible, adapts to conditions, can optimize for cost/speed
**Cons**: Less predictable, harder to debug, may choose suboptimally
**Best for**: Exploration, learning, general-purpose agent mode

### 3. Hybrid Routing

The DSL provides a preference list and constraints. The router tries the
preferred models in order, falling back based on availability and constraints.

```yaml
steps:
  - id: analyze
    type: prompt
    model:
      prefer: [qwen3:8b, llama3.1:8b]
      fallback: gpt-4
      constraints:
        max_latency_ms: 5000
```

**Pros**: Best of both worlds — predictable preferences with graceful fallback
**Cons**: More complex configuration
**Best for**: Most real-world scenarios

## Router Architecture

```rust
trait ModelRouter {
    /// Select a model for a given task and constraints
    async fn select(
        &self,
        task: &TaskMetadata,
        spec: &ModelSpec,
        available: &[ModelInfo],
    ) -> Result<ModelSelection>;
}

struct ModelSelection {
    model: ModelId,
    provider: ProviderId,
    reason: String,  // Why this model was selected (for telemetry)
}

struct TaskMetadata {
    domain: Option<TaskDomain>,      // code, reasoning, creative, general
    complexity: Option<Complexity>,   // low, medium, high
    input_tokens: Option<usize>,     // Estimated input size
    output_format: Option<Format>,   // text, json, markdown
    requires: Vec<Capability>,       // tool_calling, json_mode, vision
}

enum TaskDomain {
    Code,
    Reasoning,
    Creative,
    Summarization,
    Translation,
    DataExtraction,
    General,
}

enum Capability {
    ToolCalling,
    JsonMode,
    Vision,
    Audio,
    Streaming,
    LargeContext,  // > 32k tokens
}
```

## Model Registry

The router needs to know what models are available and their capabilities:

```rust
struct ModelRegistry {
    models: Vec<ModelInfo>,
}

struct ModelInfo {
    id: ModelId,
    provider: ProviderId,
    context_window: usize,
    capabilities: Vec<Capability>,
    domains: Vec<TaskDomain>,      // What it's good at
    locality: Locality,            // Local or Cloud
    cost_per_input_token: Option<f64>,
    cost_per_output_token: Option<f64>,
    avg_latency_ms: Option<u64>,   // Measured at runtime
    quality_scores: HashMap<TaskDomain, f64>,  // Benchmarked quality
}

enum Locality {
    Local,    // Running on this machine (Ollama, mistral.rs)
    Network,  // Running on local network
    Cloud,    // Remote API
}
```

### Populating the Registry

1. **Static configuration**: YAML file listing known models and their
   properties
2. **Dynamic discovery**: Query Ollama (`/api/tags`) for locally available
   models
3. **Runtime measurement**: Track actual latency and success rates per model

```yaml
# models.yaml
models:
  - id: qwen3:8b
    provider: ollama
    context_window: 32768
    capabilities: [tool_calling, json_mode]
    domains: [general, code, reasoning]
    locality: local

  - id: qwen2.5-coder:7b
    provider: ollama
    context_window: 32768
    capabilities: [tool_calling]
    domains: [code]
    locality: local
    quality_scores:
      code: 0.9
      general: 0.6

  - id: gpt-4
    provider: openai
    context_window: 128000
    capabilities: [tool_calling, json_mode, vision]
    domains: [general, code, reasoning, creative]
    locality: cloud
    cost_per_input_token: 0.00003
    cost_per_output_token: 0.00006
```

## Adaptive Routing Algorithm

```
Input: TaskMetadata, ModelSpec (constraints + hints), ModelRegistry

1. Filter models by hard constraints:
   - Required capabilities
   - Minimum context window
   - Locality preference (local/cloud/any)
   - Maximum cost threshold

2. Score remaining models:
   - Domain match (0.0 - 1.0): How well does the model match the task domain?
   - Complexity match: Is the model powerful enough for the complexity?
   - Latency score: Favor lower latency (especially for interactive tasks)
   - Cost score: Favor lower cost (weighted by user preference)
   - Locality bonus: Prefer local models (configurable weight)
   - Historical performance: Success rate and quality for similar tasks

3. Rank by weighted score and select top candidate

4. If top candidate fails, try next in ranking (with backoff)
```

## Meta-Routing: Using a Model to Choose the Model

An advanced strategy where a small, fast model analyzes the task and recommends
which model to use:

```yaml
steps:
  - id: classify_task
    type: prompt
    model: qwen3:4b        # Small model for classification
    temperature: 0.1
    template: |
      Classify this task and recommend the best model.
      
      Task: {{user_request}}
      
      Available models:
      {{#each available_models}}
      - {{this.id}}: Good at {{this.domains}}, context: {{this.context_window}}
      {{/each}}
      
      Respond with JSON: {"model": "...", "reason": "..."}
    output: model_selection
    
  - id: execute_task
    type: prompt
    model: "{{model_selection.model}}"  # Use the recommended model
    template: "{{user_request}}"
```

**Tradeoffs**:
- Adds latency (extra model call for classification)
- The classifier model might make poor choices
- Useful for learning: observe what the model recommends and refine

**Recommendation**: Start with hybrid routing. Add meta-routing later as an
experimental feature to explore and learn from.

## Cost-Aware Routing

When cloud models are available, track and optimize costs:

```rust
struct CostTracker {
    budget_remaining: f64,          // Daily/monthly budget
    spent_today: f64,
    cost_by_model: HashMap<ModelId, f64>,
}

impl ModelRouter {
    async fn select_with_budget(
        &self,
        task: &TaskMetadata,
        spec: &ModelSpec,
        cost_tracker: &CostTracker,
    ) -> Result<ModelSelection> {
        // If budget is low, strongly prefer local models
        let locality_weight = if cost_tracker.budget_remaining < 1.0 {
            10.0  // Heavily favor local
        } else {
            1.0   // Normal weighting
        };
        
        // ... rest of routing logic
    }
}
```

## Telemetry for Routing

Every routing decision should be traced:

```rust
#[tracing::instrument(
    name = "router.select",
    fields(
        router.strategy = %strategy,
        router.candidates = %candidates.len(),
        router.selected = tracing::field::Empty,
        router.reason = tracing::field::Empty,
        router.scores = tracing::field::Empty,
    )
)]
async fn select(&self, ...) -> Result<ModelSelection> {
    // ... routing logic ...
    
    let span = tracing::Span::current();
    span.record("router.selected", &selection.model.as_str());
    span.record("router.reason", &selection.reason.as_str());
}
```

This enables the dashboard to show:
- Which models were considered for each step
- Why a particular model was chosen
- How routing decisions correlate with output quality
- Cost breakdown by model over time
