# CLI Contract: DSL Workflow Engine

**Feature**: 005-dsl-engine  
**Date**: 2026-04-30

## New CLI Subcommands

The existing flat CLI structure is refactored to support subcommands.
The current prompt behavior becomes the default (no subcommand) or
an explicit `prompt` subcommand.

### Top-Level Structure

```text
mv-cli [OPTIONS] <COMMAND>

Commands:
  prompt     Send a prompt to a model (default when no subcommand)
  workflow   Manage and execute workflows

Options:
  -v, --verbose    Increase log verbosity (repeat for more: -vv)
  --otlp [URL]     Enable OTLP trace export [default: http://localhost:4318]
  -j, --json       Output response as JSON object
  -h, --help       Print help
  -V, --version    Print version
```

### `mv-cli prompt` (default command)

Equivalent to the current flat CLI. When no subcommand is specified and
the first argument is not a known subcommand, the CLI treats it as a prompt.

```text
mv-cli prompt [OPTIONS] <PROMPT>
mv-cli [OPTIONS] <PROMPT>

Arguments:
  <PROMPT>  The prompt to send to the model

Options:
  -m, --model <MODEL>         Model name
  -e, --endpoint <URL>        Ollama endpoint override
  -c, --config <PATH>         Path to models.yaml
  --mcp-config <PATH>         Path to MCP servers YAML config
```

### `mv-cli workflow run`

Load, validate, and execute a workflow file.

```text
mv-cli workflow run [OPTIONS] <FILE>

Arguments:
  <FILE>  Path to the workflow YAML file

Options:
  -i, --input <KEY=VALUE>     Workflow input (repeatable)
  -c, --config <PATH>         Path to models.yaml
  --mcp-config <PATH>         Path to MCP servers YAML config
```

**Behavior**:
- Parse the workflow YAML file
- Validate structural correctness (duplicate IDs, unresolvable refs, etc.)
- Validate that all required inputs are provided
- Execute steps sequentially
- Print designated workflow outputs to stdout
- If `--json` is set, output as JSON object with output names as keys

### `mv-cli workflow validate`

Validate a workflow file without executing it.

```text
mv-cli workflow validate <FILE>

Arguments:
  <FILE>  Path to the workflow YAML file
```

**Behavior**:
- Parse the workflow YAML file
- Run all structural validations
- Print validation result to stdout
- Exit 0 if valid, exit 1 if invalid

## Workflow YAML File Format

```yaml
name: research-and-summarize
version: "1.0"
description: Research a topic and produce a structured summary

defaults:
  model: qwen3:8b
  temperature: 0.7
  max_tokens: 2048

inputs:
  - name: topic
    type: string
    required: true
    description: The topic to research

steps:
  - id: plan
    name: Create Research Plan
    type: prompt
    template: |
      Create a research plan for: {{topic}}
    output: research_plan

  - id: search
    name: Search for Information
    type: tool
    tool: web_search
    inputs:
      query: "{{topic}} {{research_plan}}"
    output: search_results
    on_error: skip

  - id: summarize
    name: Create Summary
    type: prompt
    model: qwen3:4b
    temperature: 0.3
    template: |
      Summarize the research plan and search results:
      Plan: {{research_plan}}
      Results: {{search_results}}
    output: summary

outputs:
  - name: summary
    from: summary
```

## Validation Errors

| Condition | Output (stderr) | Exit Code |
|-----------|-----------------|-----------|
| Workflow file not found | `error: workflow file not found: <path>` | 1 |
| Invalid YAML syntax | `error: failed to parse workflow '<path>': <details>` | 1 |
| Unknown field in YAML | `error: failed to parse workflow '<path>': unknown field '<field>'` | 1 |
| Unknown step type | `error: failed to parse workflow '<path>': unknown step type '<type>'` | 1 |
| Duplicate step ID | `error: validation failed: duplicate step id '<id>'` | 1 |
| Unresolvable reference | `error: validation failed: step '<id>' references unknown output '<ref>'` | 1 |
| Circular reference | `error: validation failed: step '<id>' references its own output` | 1 |
| Missing template | `error: validation failed: prompt step '<id>' has no template or template_file` | 1 |
| Both template and template_file | `error: validation failed: prompt step '<id>' has both template and template_file` | 1 |
| Missing required input | `error: required input '<name>' not provided` | 1 |
| Invalid enum value | `error: input '<name>' value '<value>' not in allowed values: [<values>]` | 1 |
| Zero steps defined | `error: validation failed: workflow has no steps` | 1 |
| Output references missing step | `error: validation failed: output '<name>' references unknown step '<id>'` | 1 |
| Template file not found | `error: template file not found: <path>` | 1 |

## Runtime Errors

| Condition | Output (stderr) | Exit Code |
|-----------|-----------------|-----------|
| Model not in registry | `error: step '<id>': model '<model>' not found in registry` | 1 |
| Model backend unreachable | `error: step '<id>': cannot reach model backend` | 1 |
| Tool not found | `error: step '<id>': tool '<tool>' not found` | 1 |
| Tool invocation failed (on_error: fail) | `error: step '<id>': tool '<tool>' failed: <details>` | 1 |
| Transform failed | `error: step '<id>': transform '<op>' failed: <details>` | 1 |
| Template render error | `error: step '<id>': template error: <details>` | 1 |

## Runtime Warnings

| Condition | Output (stderr, -v) |
|-----------|---------------------|
| Tool step skipped (on_error: skip) | `warn: step '<id>': tool '<tool>' failed, skipping: <details>` |
| Tool step retrying | `warn: step '<id>': tool '<tool>' failed, retrying (attempt <n>/<max>)` |
| Workflow input has default, not provided | `info: using default for input '<name>': '<default>'` |

## Standard Output

### Default format

Workflow outputs are printed one per line:

```text
## summary

<summary text from the model>
```

Multiple outputs are separated by blank lines:

```text
## research_plan

<research plan text>

## summary

<summary text>
```

### JSON format (`--json`)

```json
{
  "workflow": "research-and-summarize",
  "outputs": {
    "summary": "...",
    "research_plan": "..."
  }
}
```

### Validate command output

```text
# Valid workflow
✓ workflow 'research-and-summarize' is valid (3 steps, 1 input, 1 output)

# Invalid workflow
✗ workflow validation failed:
  - duplicate step id 'search'
  - step 'summarize' references unknown output 'search_rsults'
```

## Verbose Output (`-v`, `-vv`)

At `-v`:

```text
INFO loading workflow from workflows/research.yaml
INFO workflow 'research-and-summarize' validated (3 steps)
INFO executing step 1/3: plan (prompt, model: qwen3:8b)
INFO step 'plan' completed (1247 chars)
INFO executing step 2/3: search (tool: web_search)
WARN step 'search': tool 'web_search' failed, skipping: tool not found
INFO executing step 3/3: summarize (prompt, model: qwen3:4b)
INFO step 'summarize' completed (892 chars)
INFO workflow completed (3 steps, 2 succeeded, 1 skipped)
```

## Examples

### Run a workflow with inputs

```bash
mv-cli workflow run workflows/research.yaml --input topic="Rust async"
```

### Run with JSON output

```bash
mv-cli --json workflow run workflows/research.yaml --input topic="Rust async"
```

### Validate a workflow

```bash
mv-cli workflow validate workflows/research.yaml
```

### Backward-compatible prompt (no subcommand)

```bash
mv-cli "What is Rust?"
mv-cli prompt "What is Rust?" --model qwen3:8b
```
