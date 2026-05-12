# Quickstart: DSL Workflow Engine

**Feature**: 005-dsl-engine  
**Date**: 2026-04-30

## Prerequisites

- Rust toolchain (edition 2024)
- Ollama running locally with `qwen3:4b` model pulled
- Project built: `cargo build`

## 1. Create a Workflow File

Create `workflows/hello.yaml`:

```yaml
name: hello-workflow
version: "1.0"
description: A simple two-step workflow

defaults:
  model: qwen3:4b

inputs:
  - name: topic
    type: string
    required: true
    description: The topic to explore

steps:
  - id: questions
    name: Generate Questions
    type: prompt
    template: |
      Generate 3 interesting questions about: {{topic}}
      Output as a numbered list.
    output: questions_list

  - id: answers
    name: Answer Questions
    type: prompt
    template: |
      Answer each of these questions concisely:
      {{questions_list}}
    output: answers_text

outputs:
  - name: answers
    from: answers_text
```

## 2. Validate the Workflow

```bash
cargo run -p mv-cli -- workflow validate workflows/hello.yaml
```

Expected output:

```text
✓ workflow 'hello-workflow' is valid (2 steps, 1 input, 1 output)
```

## 3. Run the Workflow

```bash
cargo run -p mv-cli -- workflow run workflows/hello.yaml --input topic="Rust async"
```

The engine will:

1. Parse and validate the workflow
2. Execute step `questions` — send the prompt to `qwen3:4b`
3. Execute step `answers` — send the prompt with `{{questions_list}}`
   replaced by step 1's output
4. Print the `answers` output

## 4. Run with Verbose Logging

```bash
cargo run -p mv-cli -- -v workflow run workflows/hello.yaml --input topic="Rust async"
```

## 5. Run with JSON Output

```bash
cargo run -p mv-cli -- --json workflow run workflows/hello.yaml --input topic="Rust async"
```

## 6. Use a Tool Step

See `workflows/examples/tool-example.yaml`:

```yaml
name: tool-example
version: "1.0"
description: Workflow with a tool step

defaults:
  model: qwen3:4b

inputs:
  - name: directory
    type: string
    required: true

steps:
  - id: list
    name: List Files
    type: tool
    tool: file_list
    inputs:
      path: "{{directory}}"
    output: file_listing
    on_error: skip

  - id: describe
    name: Describe Files
    type: prompt
    template: |
      Describe what these files might be for:
      {{file_listing}}
    output: description

outputs:
  - name: description
    from: description
```

Run it:

```bash
cargo run -p mv-cli -- workflow run workflows/examples/tool-example.yaml --input directory="/tmp"
```

## Common Issues

| Problem | Solution |
|---------|----------|
| `error: workflow file not found` | Check the file path is correct |
| `error: required input 'topic' not provided` | Add `--input topic="value"` |
| `error: step 'x' references unknown output 'y'` | Check step IDs match template variables |
| `error: model 'x' not found in registry` | Ensure model is in `models.yaml` or pulled in Ollama |
