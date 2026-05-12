# Research: DSL Workflow Engine

**Feature**: 005-dsl-engine  
**Date**: 2026-04-30

## Template Engine Selection

### Decision: minijinja

### Rationale

minijinja provides `{{variable}}` interpolation with a clean Rust API via
the `context!` macro and `Environment` pattern. It has minimal dependencies,
fast compile times, and Jinja2-compatible syntax. The MVP needs only simple
variable substitution — no conditionals or loops in templates — making
minijinja's lighter footprint a better fit than the full Handlebars
implementation.

### Alternatives Considered

- **handlebars** (v6.4): Full Handlebars implementation. More features than
  needed for MVP (helpers, partials, inheritance). Slower compile times.
  Would be appropriate if templates needed conditionals/loops, but those
  are deferred to later phases.
- **Custom `{{var}}` replacement**: Simplest option but fragile. No error
  reporting for unresolvable variables, no escaping, no path to future
  features. Not worth the maintenance cost.
- **tera**: Jinja2-like, full-featured. Similar to handlebars in scope —
  more than needed for MVP.

### YAML Interaction

Template strings containing `{{` MUST be quoted in YAML to prevent parser
ambiguity. The DSL schema examples and documentation will use block scalar
(`|`) or quoted strings for all template values. Since `serde_yml` handles
YAML parsing before template rendering, this is a documentation concern
rather than a code concern.

## Strict YAML Parsing

### Decision: Use `#[serde(deny_unknown_fields)]` on all workflow types

### Rationale

The spec requires rejecting unknown fields (FR-002, Assumptions). serde's
`deny_unknown_fields` attribute is supported by `serde_yml` and provides
this at parse time with clear error messages. This catches typos in workflow
YAML files early, before any execution occurs.

### Alternatives Considered

- **Lenient parsing (ignore unknown fields)**: Default serde behavior.
  Rejected because silent typos (e.g., `tempalte:` instead of `template:`)
  would cause confusing runtime failures instead of clear parse errors.

## CLI Subcommand Architecture

### Decision: Refactor mv-cli to use clap subcommands

### Rationale

The current CLI is a flat `#[derive(Parser)]` struct with a positional
`prompt` argument. Adding `workflow run` and `workflow validate` requires
restructuring to subcommands. The existing prompt mode becomes the default
(or an explicit `prompt` subcommand) to maintain backward compatibility.

### Approach

Use clap's `#[derive(Subcommand)]` with a two-level hierarchy:
- Top-level: `Commands` enum with `Prompt` (default) and `Workflow` variants
- Workflow level: `WorkflowAction` enum with `Run` and `Validate` variants

Shared flags (verbose, otlp, json) move to the top-level `Cli` struct.
Prompt-specific flags (model, endpoint, config, mcp_config) stay with the
`Prompt` variant.

### Alternatives Considered

- **Separate binary**: A separate `mv-workflow` binary. Rejected because it
  adds build complexity and fragments the user experience.
- **Flag-based dispatch** (`--workflow` flag): Rejected because it does not
  scale cleanly as more commands are added.

## Output Passing Architecture

### Decision: HashMap-based execution context

### Rationale

During workflow execution, step outputs are stored in a
`HashMap<String, String>` keyed by step ID. Template rendering receives this
map as context, making all previous step outputs and workflow inputs available
for interpolation. String values cover all cases: LLM responses are text,
tool results are serialized text, and transform outputs are stringified JSON.

### Approach

```text
ExecutionContext {
    inputs: HashMap<String, String>,    // --input key=value from CLI
    outputs: HashMap<String, String>,   // step_id → step output text
}
```

Template variables resolve in order: step outputs first, then workflow inputs.
This allows step outputs to shadow workflow inputs of the same name (unlikely
but deterministic).

### Alternatives Considered

- **Typed values (serde_json::Value)**: More expressive but adds complexity.
  Deferred to when transform steps need to pass structured data. For MVP,
  stringified JSON in a string value is sufficient.
- **Ordered Vec of outputs**: Preserves insertion order but loses O(1)
  lookup. HashMap is simpler and sufficient.

## Step Execution Error Handling

### Decision: Per-step `on_error` field with `skip`, `fail`, `retry`

### Rationale

Consistent with the DSL design doc. Default behavior is `fail` (stop
workflow on error). `skip` sets output to empty string and continues.
`retry` uses configurable max attempts and exponential backoff.

Only `tool` steps support `on_error` and `retry`. Prompt steps always fail
on error (model errors are not retryable in a meaningful way at this level).
Transform steps always fail on error (data transformation errors indicate a
workflow logic problem).
