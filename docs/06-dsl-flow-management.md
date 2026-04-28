# DSL & Flow Management

## Vision

A YAML-based DSL for defining agent workflows — similar in spirit to Azure
DevOps Pipeline YAML but for agent controller operations. The DSL defines the
**what** (steps, models, tools, prompts) while the engine handles the **how**
(execution, error recovery, telemetry).

## Design Principles

1. **Declarative**: Describe what should happen, not how to implement it
2. **Composable**: Workflows can include other workflows
3. **Overridable**: Steps can specify constraints while allowing the engine to
   choose details (like model selection)
4. **Validated**: Rust's type system validates DSL at parse time, not at runtime
5. **Versionable**: YAML files live in version control alongside code

## DSL Schema

### Top-Level Structure

```yaml
# workflow.yaml
name: research-and-summarize
version: "1.0"
description: Research a topic and produce a structured summary

# Default settings for all steps
defaults:
  model: qwen3:8b
  temperature: 0.7
  max_tokens: 2048

# Input parameters for the workflow
inputs:
  - name: topic
    type: string
    required: true
    description: The topic to research
  - name: depth
    type: enum
    values: [shallow, medium, deep]
    default: medium

# Workflow steps
steps:
  - id: plan
    name: Create Research Plan
    type: prompt
    model: qwen3:8b
    template: |
      Create a research plan for the topic: {{topic}}
      Research depth: {{depth}}
      
      Output a numbered list of research questions.
    output: research_plan

  - id: search
    name: Search for Information
    type: tool
    tool: web_search
    inputs:
      query: "{{topic}} {{research_plan}}"
      limit: 10
    output: search_results

  - id: analyze
    name: Analyze Results
    type: prompt
    model:
      prefer: [qwen3:8b, gpt-4]
      strategy: adaptive
      constraints:
        min_context_window: 8192
    template: |
      Based on these search results, analyze the key findings:
      
      {{search_results}}
      
      Focus on the research plan:
      {{research_plan}}
    output: analysis

  - id: summarize
    name: Create Summary
    type: prompt
    model: qwen3:4b  # Smaller model is fine for summarization
    temperature: 0.3
    template: |
      Create a structured summary from this analysis:
      {{analysis}}
      
      Format as markdown with sections.
    output: summary
    
# Output definition
outputs:
  - name: summary
    from: summary
  - name: research_plan
    from: research_plan
```

### Step Types

#### `prompt` — LLM Completion

```yaml
- id: generate
  type: prompt
  model: qwen3:8b           # Specific model
  temperature: 0.7
  max_tokens: 1024
  template: |               # Inline template
    {{system_prompt}}
    User: {{input}}
  # OR
  template_file: templates/generate.md   # External template
  output: generated_text
```

#### `tool` — Tool Invocation

```yaml
- id: search
  type: tool
  tool: web_search           # Tool name (from MCP or built-in)
  inputs:
    query: "{{search_query}}"
    limit: 5
  output: search_results
  on_error: skip             # skip | fail | retry
  retry:
    max_attempts: 3
    backoff: exponential
```

#### `transform` — Data Transformation

```yaml
- id: extract
  type: transform
  operation: extract_json    # Built-in transform
  input: "{{raw_response}}"
  schema:                    # Expected JSON schema
    type: object
    properties:
      title: { type: string }
      points: { type: array, items: { type: string } }
  output: structured_data
```

#### `branch` — Conditional Execution

```yaml
- id: check_complexity
  type: branch
  condition: "{{analysis.complexity}} > 0.8"
  then:
    - id: deep_dive
      type: prompt
      model: gpt-4           # Use a bigger model for complex tasks
      template: "Deep analysis of: {{analysis}}"
      output: detailed_analysis
  else:
    - id: quick_summary
      type: prompt
      model: qwen3:4b
      template: "Quick summary of: {{analysis}}"
      output: detailed_analysis
```

#### `parallel` — Concurrent Execution

```yaml
- id: multi_search
  type: parallel
  steps:
    - id: search_web
      type: tool
      tool: web_search
      inputs: { query: "{{topic}}" }
      output: web_results
    - id: search_docs
      type: tool
      tool: rag_search
      inputs: { query: "{{topic}}" }
      output: doc_results
  output:
    web: web_results
    docs: doc_results
```

#### `loop` — Iterative Execution

```yaml
- id: refine
  type: loop
  max_iterations: 3
  steps:
    - id: evaluate
      type: prompt
      model: qwen3:8b
      template: "Evaluate this draft: {{draft}}\nScore 1-10 and suggest improvements."
      output: evaluation
    - id: improve
      type: prompt
      model: qwen3:8b
      template: "Improve this draft based on feedback:\n{{draft}}\n{{evaluation}}"
      output: draft
  exit_condition: "{{evaluation.score}} >= 8"
```

#### `workflow` — Nested Workflow

```yaml
- id: sub_task
  type: workflow
  file: workflows/sub-task.yaml
  inputs:
    topic: "{{sub_topic}}"
  output: sub_result
```

### Model Specification

Models can be specified at different levels of specificity:

```yaml
# Exact model
model: qwen3:8b

# Preferred list with fallback
model:
  prefer: [qwen3:8b, llama3.1:8b, gpt-4]
  
# Adaptive selection with constraints
model:
  strategy: adaptive          # prescriptive | adaptive | hybrid
  constraints:
    min_context_window: 8192
    max_cost_per_token: 0.001
    capabilities: [tool_calling, json_mode]
    locality: local            # local | cloud | any
  hints:
    domain: code               # code | general | reasoning | creative
    complexity: high

# Prescriptive per environment
model:
  strategy: prescriptive
  local: qwen3:8b
  cloud: gpt-4
```

### Prompt Templates

Templates use Handlebars-style variable interpolation:

```yaml
# Inline
template: |
  You are a {{role}}.
  
  {{#if context}}
  Context:
  {{context}}
  {{/if}}
  
  User request: {{input}}

# External file
template_file: prompts/research-assistant.md

# With system/user message separation
messages:
  - role: system
    content: "You are a helpful research assistant."
  - role: user
    content: "Research the following topic: {{topic}}"
```

## Rust Implementation

### Parsing

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
struct Workflow {
    name: String,
    version: String,
    description: Option<String>,
    defaults: Option<WorkflowDefaults>,
    inputs: Vec<WorkflowInput>,
    steps: Vec<Step>,
    outputs: Vec<WorkflowOutput>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type")]
enum Step {
    #[serde(rename = "prompt")]
    Prompt(PromptStep),
    #[serde(rename = "tool")]
    Tool(ToolStep),
    #[serde(rename = "transform")]
    Transform(TransformStep),
    #[serde(rename = "branch")]
    Branch(BranchStep),
    #[serde(rename = "parallel")]
    Parallel(ParallelStep),
    #[serde(rename = "loop")]
    Loop(LoopStep),
    #[serde(rename = "workflow")]
    SubWorkflow(SubWorkflowStep),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum ModelSpec {
    Exact(String),
    Preferred { prefer: Vec<String> },
    Adaptive {
        strategy: RoutingStrategy,
        constraints: Option<ModelConstraints>,
        hints: Option<ModelHints>,
    },
}
```

### Validation

Validate workflows at parse time using Rust's type system:

```rust
impl Workflow {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        
        // Check all output references point to valid step outputs
        for output in &self.outputs {
            if !self.steps.iter().any(|s| s.id() == output.from) {
                errors.push(ValidationError::MissingStepOutput(output.from.clone()));
            }
        }
        
        // Check for circular dependencies
        // Check template variables are resolvable
        // Check tool names exist in registry
        
        if errors.is_empty() { Ok(()) } else { Err(errors) }
    }
}
```

### Template Engine

Use a lightweight template engine. Options:

| Crate | Approach | Recommendation |
|-------|----------|----------------|
| `handlebars` | Full Handlebars implementation | ✅ If you need conditionals/loops in templates |
| `tera` | Jinja2-like templates | Good alternative |
| `minijinja` | Minimal Jinja2 | ✅ Lightweight, fast |
| Custom | Simple `{{var}}` replacement | Fine for MVP |

## Comparison to ADO Pipeline YAML

| ADO Pipeline | Multae Viae DSL | Notes |
|--------------|-----------------|-------|
| `trigger` | (event system, future) | Could add event triggers later |
| `pool` | `model` | Which model/resource to use |
| `stages` | Top-level grouping | Could add later |
| `jobs` | `steps` with `parallel` | Parallel execution |
| `steps` | `steps` | Sequential execution |
| `task` | `type: tool` | Named operation |
| `script` | `type: prompt` / `type: tool` | Flexible execution |
| `template` | `type: workflow` | Reusable components |
| `variables` | `inputs` + `outputs` | Data flow |
| `condition` | `type: branch` | Conditional logic |

## Evolution Path

1. **Phase 1 (MVP)**: Sequential steps, exact models, inline templates
2. **Phase 2**: Branching, parallel execution, model preferences
3. **Phase 3**: Adaptive model routing, loop constructs, nested workflows
4. **Phase 4**: Event triggers, conditional execution, runtime overrides
5. **Phase 5**: Visual editor in the dashboard project
