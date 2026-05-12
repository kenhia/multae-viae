# Data Model: DSL Workflow Engine

**Feature**: 005-dsl-engine  
**Date**: 2026-04-30

## Entities

### Workflow

A named, versioned definition of a multi-step process.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| name | String | Yes | Human-readable workflow name |
| version | String | Yes | Semantic version string |
| description | String | No | Workflow purpose description |
| defaults | WorkflowDefaults | No | Default settings for all steps |
| inputs | Vec\<WorkflowInput\> | No | Input parameters (empty = no inputs) |
| steps | Vec\<Step\> | Yes | Ordered list of steps to execute |
| outputs | Vec\<WorkflowOutput\> | No | Output mappings (empty = last step output) |

### WorkflowDefaults

Default settings inherited by all steps unless overridden.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| model | String | No | Default model for prompt steps |
| temperature | f64 | No | Default temperature for prompt steps |
| max_tokens | u64 | No | Default max tokens for prompt steps |

### WorkflowInput

A named parameter provided by the user at runtime.

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| name | String | Yes | Parameter name (used as template variable) |
| type | InputType | Yes | Parameter type: string or enum |
| required | bool | No | Whether the input must be provided (default: false) |
| description | String | No | Human-readable description |
| default | String | No | Default value when not provided |
| values | Vec\<String\> | No | Allowed values when type is enum |

### InputType (enum)

- `string` — Free-form text value
- `enum` — Value must be one of the defined `values`

### Step (tagged union on `type` field)

A single unit of work. Discriminated by the `type` field.

**Common fields (all step types)**:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| id | String | Yes | Unique identifier within the workflow |
| name | String | No | Human-readable step name |
| type | StepType | Yes | Discriminator: prompt, tool, or transform |
| output | String | Yes | Name under which this step's result is stored |

### PromptStep (type: prompt)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| model | String | No | Model to use (overrides workflow default) |
| temperature | f64 | No | Temperature (overrides workflow default) |
| max_tokens | u64 | No | Max tokens (overrides workflow default) |
| template | String | No | Inline prompt template with `{{var}}` interpolation |
| template_file | String | No | Path to external template file (relative to workflow file) |

Exactly one of `template` or `template_file` MUST be set (validated at parse time).

### ToolStep (type: tool)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| tool | String | Yes | Tool name (built-in or MCP tool identifier) |
| inputs | Map\<String, Value\> | No | Tool input parameters (values support `{{var}}` interpolation) |
| on_error | ErrorAction | No | Error handling: skip, fail, retry (default: fail) |
| retry | RetryConfig | No | Retry configuration (only when on_error is retry) |

### TransformStep (type: transform)

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| operation | String | Yes | Transform operation name (e.g., extract_json) |
| input | String | Yes | Input expression with `{{var}}` interpolation |
| schema | Value | No | Expected output shape for validation (simple serde_json::Value structural comparison, not full JSON Schema draft validation) |

### ErrorAction (enum)

- `skip` — Log warning, set output to empty string, continue
- `fail` — Stop workflow with error (default)
- `retry` — Retry with configured backoff

### RetryConfig

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| max_attempts | u32 | No | Maximum retry attempts (default: 3) |
| backoff | BackoffStrategy | No | Backoff strategy (default: exponential) |

### BackoffStrategy (enum)

- `exponential` — Exponential backoff (default)
- `fixed` — Fixed delay between retries

### WorkflowOutput

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| name | String | Yes | Output name displayed to user |
| from | String | Yes | Step ID whose output this maps to |

## Runtime Entities (not serialized)

### ExecutionContext

Runtime state during workflow execution. Not part of the YAML schema.

| Field | Type | Description |
|-------|------|-------------|
| inputs | HashMap\<String, String\> | Workflow input values from CLI |
| outputs | HashMap\<String, String\> | Step ID → output text, accumulated during execution |

Template variable resolution order: outputs first, then inputs.

### ValidationError

Errors detected during structural validation.

| Variant | Description |
|---------|-------------|
| DuplicateStepId | Two or more steps share the same ID |
| UnresolvableReference | Template references a step ID that does not exist |
| CircularReference | A step's template references its own output |
| MissingStepOutput | Workflow output `from` references nonexistent step ID |
| MissingTemplate | Prompt step has neither `template` nor `template_file` |
| BothTemplates | Prompt step has both `template` and `template_file` |
| EmptySteps | Workflow defines zero steps |
| MissingRequiredInput | Required input has no default and is not provided |
| InvalidEnumValue | Input value not in enum's allowed values |
| UnknownTransformOp | Transform step uses an unrecognized operation |

## Relationships

```text
Workflow 1──* WorkflowInput
Workflow 1──* Step
Workflow 1──* WorkflowOutput
WorkflowOutput.from ──> Step.id
Step (tool) 1──0..1 RetryConfig
Step templates reference → ExecutionContext (inputs + outputs)
```

## State Transitions

### Workflow Execution Lifecycle

```text
LOADED → VALIDATING → VALIDATED → EXECUTING → COMPLETED
                  ↘                    ↘
               INVALID              FAILED
```

- **LOADED**: YAML parsed into Workflow struct
- **VALIDATING**: Structural validation in progress
- **VALIDATED**: All validation checks passed
- **INVALID**: Validation errors found (reported to user)
- **EXECUTING**: Steps running sequentially
- **COMPLETED**: All steps finished, outputs available
- **FAILED**: A step failed and on_error is `fail`
