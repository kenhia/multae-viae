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

  - id: fetch
    name: Fetch Reference Material
    type: tool
    tool: http_get               # Built-in tool (or any registered MCP tool)
    inputs:
      url: "https://en.wikipedia.org/wiki/{{topic}}"
    output: search_results

  - id: analyze
    name: Analyze Results
    type: prompt
    model: qwen3:8b              # Exact model id only; model-preference
                                 # objects are not yet implemented (Phase 5)
    template: |
      Based on this reference material, analyze the key findings:
      
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

Tool steps execute against the same merged built-in + MCP tool set the
agent sees.

```yaml
- id: fetch
  type: tool
  tool: http_get             # Tool name (built-in or MCP)
  inputs:
    url: "https://example.com/{{page}}"
  output: page_body
  on_error: retry            # skip | fail | retry
  retry:
    max_attempts: 3          # must be >= 1 (validated)
    backoff: exponential     # exponential | fixed
    base_delay_ms: 100       # optional; default 100, delay capped at 30s
```

Retry semantics: only **transient** errors (backend unreachable, completion
failure, tool/MCP call failure) are re-attempted — permanent failures
(validation errors, missing inputs, config mistakes) fail immediately
regardless of `on_error: retry`. Note that each retry **re-executes the
tool**, side effects included: `shell_exec`, or `http_get` against a
non-idempotent endpoint, runs again on every attempt.

#### `transform` — Data Transformation

`extract_json` is currently the only transform operation; unknown operations
are rejected at validation.

```yaml
- id: extract
  type: transform
  operation: extract_json    # Built-in transform (the only one today)
  input: "{{raw_response}}"
  schema:                    # Expected JSON schema
    type: object
    properties:
      title: { type: string }
      points: { type: array, items: { type: string } }
  output: structured_data
```

#### `branch` — Conditional Execution **(shipped — sprint 009)**

`condition` is a **minijinja expression** (the same template language as
`{{…}}`, but written bare — no surrounding braces) evaluated against the
context. The `then` arm runs when it is truthy; the optional `else` arm runs
otherwise. Arms may nest further `branch`/`parallel` steps.

```yaml
- id: route
  type: branch
  condition: "style == 'detailed'"   # bare expression, not {{...}}
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

**Maybe-defined outputs.** Validation tracks which outputs are *definitely*
defined after a branch: an output is only available afterward if **every** arm
defines it (a missing `else` defines nothing). Referencing an output defined in
just one arm is a validation error — that is why both arms above write
`detailed_analysis`.

**Truthiness caveat.** Context values are strings today, so a bare
`condition: "flag"` is truthy whenever the string is non-empty — the literal
string `"false"` is truthy. Prefer explicit comparisons
(`condition: "flag == 'on'"`). Typed values arrive with the Phase 6 `Value`
context migration.

#### `parallel` — Concurrent Execution **(shipped — sprint 009)**

Child steps run **concurrently** (fork-join). Each child executes against an
immutable snapshot of the context taken at the fork, so a child **cannot see a
sibling's output** (referencing one is a validation error); child outputs must
be **disjoint** and merge back into the context at the join, where all of them
become available to later steps. All children run to completion; if any fail,
the step reports every failure.

```yaml
- id: multi_search
  type: parallel
  steps:
    - id: search_web
      type: tool
      tool: http_get
      inputs: { url: "{{web_query_url}}" }
      output: web_results
    - id: search_docs
      type: tool
      tool: file_read
      inputs: { path: "{{docs_path}}" }
      output: doc_results
# After the join, both `web_results` and `doc_results` are in the context.
```

#### `loop` — Iterative Execution **(Not yet implemented — planned Phase 5/6)**

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

#### `workflow` — Nested Workflow **(Not yet implemented — planned Phase 5/6)**

```yaml
- id: sub_task
  type: workflow
  file: workflows/sub-task.yaml
  inputs:
    topic: "{{sub_topic}}"
  output: sub_result
```

### Model Specification

**Implemented today** (sprint 009): an exact model id string, **or** a
`prefer:` list. Both resolve against the `models.yaml` registry; a step naming
an unregistered model (or `prefer` entry) fails with `ModelNotInRegistry`
before the run starts — there is no silent default substitution.

A `prefer:` list resolves through the fallback-chain mechanism: the first
reachable model serves the step, advancing past a backend that is dead or
returns a fallback-eligible error (and each preferred id still honors its own
`fallback:` chain from `models.yaml`). Adaptive strategies and constraints
remain **not yet implemented — planned Phase 7** (layered on this mechanism).

```yaml
# Exact model
model: qwen3:8b

# Preferred list — try in order, first reachable serves (shipped, sprint 009)
model:
  prefer: [qwen3:8b, llama3.1:8b, gpt-4]

# Adaptive selection with constraints (Not yet implemented — planned Phase 7)
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
```

### Prompt Templates

The template engine is **minijinja** (Jinja2 dialect) with **strict
undefined behavior**: referencing an undefined variable is an error, not an
empty string. Conditionals use `{% if %}` — Handlebars-style `{{#if}}`
syntax is invalid and errors.

```yaml
# Inline
template: |
  You are a {{role}}.
  
  {% if context %}
  Context:
  {{context}}
  {% endif %}
  
  User request: {{input}}

# External file (relative to the workflow file's directory)
template_file: prompts/research-assistant.md
```

```yaml
# With system/user message separation
# (Not yet implemented — planned Phase 5/6; today each prompt step sends a
# single user message under the shared system preamble)
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
    Branch(BranchStep),       // shipped, sprint 009
    #[serde(rename = "parallel")]
    Parallel(ParallelStep),   // shipped, sprint 009
    // Planned Phase 6: Loop, SubWorkflow
}
```

The implemented types live in `crates/mv-core/src/workflow/types.rs`. A prompt
step's `model` is a `ModelSpec` (untagged `Single(String)` | `Prefer { prefer:
Vec<String> }`); `Step::output()` returns `Option<&str>` because `branch` and
`parallel` are control-flow steps with no single output of their own.

### Validation

Workflows are validated structurally after parsing
(`crates/mv-core/src/workflow/validate.rs`, surfaced by
`mv-cli workflow validate`). What it checks today:

- Non-empty `steps`; no duplicate step ids; no duplicate output names
  (a step output shadowing a workflow input is a warning, not an error)
- Prompt steps have exactly one of `template` / `template_file`
- Template syntax and references, using minijinja's own parser
  (`undeclared_variables()`) — so filters (`{{ x | upper }}`) and
  `{% if %}` blocks validate correctly. Every referenced variable must
  resolve to a prior step output or a workflow input; self-references are
  circular-reference errors. `template_file` contents are validated too
  (when the workflow's directory is known), as are template strings nested
  inside tool-step input values
- Transform `operation` is a known transform (`extract_json`)
- Retry config: `max_attempts >= 1`
- Workflow `outputs[].from` references an existing step (top-level or nested
  in a branch/parallel) whose output is **definitely defined on every
  execution path** — mapping from a step in a single branch arm, or from a
  branch/parallel step itself (which has no single output), is an error
- `branch`: the `condition` compiles as a minijinja expression and its
  variables resolve; the `then` arm is non-empty; **maybe-defined analysis** —
  an output is only available after the branch if every arm defines it
- `parallel`: children are validated against the pre-fork context only (a
  reference to a sibling's output fails); child outputs must be disjoint; the
  step has at least one child
- Step ids and output names are checked across the whole tree (branch/parallel
  arms included); the same output name in a branch's `then` and `else` is the
  recommended pattern, not a duplicate — but an arm step reusing an output
  name from an enclosing scope is a duplicate (it would silently overwrite)
- A `model: { prefer: [...] }` list (on a step or in `defaults`) must not be
  empty

**Not checked**: tool names. A `tool:` value is only resolved at runtime
against the merged built-in + MCP tool set — a typo'd tool name passes
`workflow validate` and fails at execution.

### Template Engine

**Chosen: `minijinja`** (strict undefined behavior; the same engine parses
templates during validation and renders them at execution, so the two can
never disagree). Options considered:

| Crate | Approach | Outcome |
|-------|----------|---------|
| `minijinja` | Minimal Jinja2 | ✅ **Chosen** — lightweight, fast, own parser reusable for validation |
| `handlebars` | Full Handlebars implementation | Not chosen |
| `tera` | Jinja2-like templates | Not chosen |
| Custom | Simple `{{var}}` replacement | Rejected — validation and rendering drift |

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

1. **Phase 1 (MVP)**: Sequential steps, exact models, inline templates —
   **shipped** (sprints 005–008, plus `template_file`, transforms, and retry)
2. **Branching, parallel execution, model preference lists** —
   **shipped** (sprint 009)
3. **Adaptive model routing, loop constructs, nested workflows** — adaptive
   routing in Phase 7; `loop` and nested `workflow` steps in Phase 6
4. **Event triggers, runtime overrides** — Phase 6+
5. **Visual editor** in the dashboard project
