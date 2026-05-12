# Feature Specification: DSL Workflow Engine

**Feature Branch**: `005-dsl-engine`  
**Created**: 2026-04-30  
**Status**: Draft  
**Input**: User description: "Phase 4 from docs/09-roadmap.md — Execute multi-step workflows defined in YAML."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Run a Sequential Multi-Step Workflow (Priority: P1)

A user authors a YAML workflow file that defines a series of steps — prompt calls and tool invocations — that execute in order. Each step can reference the output of previous steps via template variables. The user runs the workflow from the CLI and receives the final output.

**Why this priority**: Sequential step execution with output passing is the foundation of the entire DSL engine. Every other capability (branching, parallelism, loops) builds on top of sequentially executing steps and passing data between them.

**Independent Test**: Create a two-step workflow where step 1 asks a model to generate research questions about a topic, and step 2 asks a model to answer those questions. Run `mv-cli workflow run workflow.yaml --input topic="Rust async"` and verify both steps execute in order, with step 2 receiving step 1's output.

**Acceptance Scenarios**:

1. **Given** a valid YAML workflow file with two sequential `prompt` steps, **When** the user runs the workflow, **Then** both steps execute in order, each receiving the configured model's response.
2. **Given** a workflow step references `{{previous_step_output}}` in its template, **When** the step executes, **Then** the template variable is replaced with the actual output from the referenced step.
3. **Given** a workflow defines input parameters, **When** the user provides `--input key=value` arguments, **Then** the values are available as template variables in all steps.
4. **Given** a workflow step fails (model error, timeout), **When** no error handling is configured, **Then** the workflow stops with a clear error message identifying the failed step.

---

### User Story 2 - Author and Validate Workflow Files (Priority: P2)

A user writes a YAML workflow file and wants to verify it is correct before running it. The user runs a validate command that checks the workflow for structural errors — missing required fields, invalid step references, unresolvable template variables, and unknown step types — without executing any steps.

**Why this priority**: Fast feedback on workflow correctness prevents wasted time running broken workflows. Validation at parse time (not runtime) is a core design principle of the DSL.

**Independent Test**: Create a workflow with a template variable referencing a nonexistent step output, run `mv-cli workflow validate workflow.yaml`, and verify it reports the error without executing anything.

**Acceptance Scenarios**:

1. **Given** a syntactically valid workflow file, **When** the user runs the validate command, **Then** the system reports the workflow is valid.
2. **Given** a workflow with a step that references an output from a nonexistent step ID, **When** the user runs validate, **Then** the system reports the specific unresolvable reference.
3. **Given** a workflow with a missing required field (e.g., step without `id`), **When** the user runs validate, **Then** the system reports the missing field with its location.
4. **Given** a workflow with an unknown step type, **When** the user runs validate, **Then** the system reports the invalid step type.

---

### User Story 3 - Use Tool Steps in Workflows (Priority: P3)

A user includes tool invocation steps in their workflow — calling built-in tools or MCP server tools. The tool step specifies the tool name and input parameters (which can reference outputs from previous steps). The tool's result is captured and available to subsequent steps.

**Why this priority**: Tool steps connect the DSL engine to the existing tool infrastructure (built-in and MCP tools), making workflows capable of interacting with the outside world beyond just LLM calls.

**Independent Test**: Create a workflow with a `tool` step that calls the built-in `file_list` tool, followed by a `prompt` step that summarizes the results. Run the workflow and verify the tool is invoked and its output feeds into the prompt step.

**Acceptance Scenarios**:

1. **Given** a workflow with a `tool` step specifying a known built-in tool, **When** the step executes, **Then** the tool is invoked with the specified inputs and its result is stored under the step's output name.
2. **Given** a workflow with a `tool` step specifying an MCP tool, **When** the step executes, **Then** the MCP tool is invoked through the existing MCP integration and its result is available to subsequent steps.
3. **Given** a `tool` step with `on_error: skip`, **When** the tool invocation fails, **Then** the workflow continues to the next step with the failed step's output set to an empty value.
4. **Given** a `tool` step with `on_error: fail` (or no error handling), **When** the tool invocation fails, **Then** the workflow stops with an error identifying the failed tool and reason.

---

### User Story 4 - Transform Step Outputs (Priority: P4)

A user includes a `transform` step in their workflow to extract or restructure data from a previous step's output — for example, extracting JSON from a model response or selecting specific fields. This enables clean data flow between steps without requiring the model to reformat its output.

**Why this priority**: Transform steps are important for reliable data flow between steps, but workflows can function without them by relying on prompt engineering to format outputs. They add robustness rather than new capability.

**Independent Test**: Create a workflow where a `prompt` step returns a JSON-containing response, followed by a `transform` step with `operation: extract_json`, and verify the transform step outputs parsed structured data.

**Acceptance Scenarios**:

1. **Given** a `transform` step with `operation: extract_json` and a valid JSON input, **When** the step executes, **Then** the output is the parsed JSON structure.
2. **Given** a `transform` step with a JSON schema specified, **When** the extracted data does not match the schema, **Then** the step fails with a clear validation error.
3. **Given** a `transform` step with an unknown operation, **When** the workflow is validated, **Then** the validation reports the unknown transform operation.

---

### User Story 5 - Workflow Execution Appears in Telemetry (Priority: P5)

A developer inspects traces in Jaeger after running a workflow. The workflow execution appears as a parent span containing child spans for each step. Each step span includes attributes for step ID, step type, model used (for prompt steps), tool name (for tool steps), and duration.

**Why this priority**: Telemetry is essential for debugging and understanding workflow execution, but it layers on existing infrastructure and does not affect core functionality.

**Independent Test**: Run a multi-step workflow, then check Jaeger for a workflow span containing child spans for each step with the expected attributes.

**Acceptance Scenarios**:

1. **Given** a workflow executes successfully, **When** the user views the trace in Jaeger, **Then** a parent span for the workflow contains child spans for each executed step.
2. **Given** a workflow step fails, **When** the user views the trace, **Then** the failed step's span records the error status and error message.
3. **Given** a prompt step executes, **When** the user views its span, **Then** attributes include the model name, token counts, and duration.

---

### Edge Cases

- What happens when a workflow YAML file is empty or contains only comments?
- What happens when a workflow has zero steps defined?
- What happens when a step's template variable references its own output (circular reference)?
- What happens when two steps have the same ID?
- What happens when a workflow input parameter is required but not provided by the user?
- What happens when a model specified in a step is not available in the model registry?
- What happens when a step produces extremely large output that would exceed model context limits in a subsequent prompt step?
- What happens when the workflow YAML contains unknown fields (should they be ignored or rejected)?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST support defining workflows in YAML files conforming to a documented schema, including name, version, description, defaults, inputs, steps, and outputs.
- **FR-002**: System MUST parse and validate workflow YAML files at load time, reporting all structural errors before execution begins.
- **FR-003**: System MUST support `prompt` step type — sending a templated prompt to a specified model and capturing the response as the step's output.
- **FR-004**: System MUST support `tool` step type — invoking a built-in or MCP tool with specified inputs and capturing the result as the step's output.
- **FR-005**: System MUST support `transform` step type — applying a data transformation operation (at minimum `extract_json`) to an input and capturing the result.
- **FR-006**: System MUST execute steps sequentially in the order defined in the workflow file.
- **FR-007**: System MUST support template variable interpolation in prompt templates and tool inputs, resolving variables from step outputs and workflow inputs (in that precedence order).
- **FR-008**: System MUST pass step outputs forward — the output of step N is available to steps N+1, N+2, etc., referenced by step ID.
- **FR-009**: System MUST support workflow-level default settings (model, temperature, max_tokens) that apply to all steps unless overridden at the step level.
- **FR-010**: System MUST support workflow input parameters with name, type, required flag, description, and optional default value.
- **FR-011**: System MUST validate that all required workflow inputs are provided before execution begins.
- **FR-012**: System MUST support a `workflow validate` CLI subcommand that checks a workflow file for errors without executing it.
- **FR-013**: System MUST support a `workflow run` CLI subcommand that loads, validates, and executes a workflow file, accepting input parameters via `--input key=value` flags.
- **FR-014**: System MUST support configurable error handling per tool step via an `on_error` field with values `skip`, `fail`, or `retry`.
- **FR-015**: System MUST support retry configuration for tool steps, including max attempts and backoff strategy.
- **FR-016**: System MUST support both inline templates (`template:`) and external template files (`template_file:`) for prompt steps.
- **FR-017**: System MUST instrument workflow execution in telemetry — a parent span for the workflow with child spans for each step, including step-type-specific attributes.
- **FR-018**: System MUST define workflow outputs that map to specific step outputs, and display the designated outputs to the user upon workflow completion.
- **FR-019**: System MUST reject workflow files containing duplicate step IDs during validation.
- **FR-020**: System MUST reject workflow files containing circular output references during validation.

### Key Entities

- **Workflow**: A named, versioned definition of a multi-step process. Contains metadata (name, version, description), defaults, input parameters, an ordered list of steps, and output mappings.
- **Step**: A single unit of work within a workflow. Has a unique ID, a type (prompt, tool, transform), type-specific configuration, and an output name. Steps execute sequentially and can reference outputs of preceding steps.
- **Workflow Input**: A named parameter that the user provides when running a workflow. Has a name, type, required flag, description, and optional default value.
- **Workflow Output**: A mapping from a named output to a specific step's output, defining what the workflow returns to the user.
- **Template**: A text template with `{{variable}}` interpolation syntax used in prompt steps. Can be inline or loaded from an external file.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Users can author a multi-step workflow in YAML and execute it from the CLI in a single command, receiving the final output within the expected time for the configured models.
- **SC-002**: Workflow validation catches 100% of structural errors (missing fields, invalid references, duplicate IDs, circular references) before execution begins.
- **SC-003**: Users can chain 5 or more steps in a single workflow, with each step successfully referencing outputs from any preceding step.
- **SC-004**: Workflow execution traces are visible in Jaeger, with each step appearing as a child span of the workflow span, within 30 seconds of workflow completion.
- **SC-005**: A user unfamiliar with the system can author and run a simple two-step workflow within 10 minutes, using only the documented YAML schema and CLI help.
- **SC-006**: Tool steps (both built-in and MCP) execute with the same reliability as direct tool calls outside of workflows — no regressions in tool invocation success rate.
- **SC-007**: Workflow validation completes in under 1 second for workflows with up to 50 steps.

## Assumptions

- The existing model registry, tool registry, and MCP integration from Phases 1–3 are functional and stable. The DSL engine builds on these rather than reimplementing them.
- The template engine uses Handlebars-style `{{variable}}` interpolation syntax, consistent with the design document in `docs/06-dsl-flow-management.md`.
- Phase 4 scope covers sequential execution and the three core step types (`prompt`, `tool`, `transform`). Advanced step types (`branch`, `parallel`, `loop`, `workflow`) are deferred to Phase 5 or later, consistent with the DSL evolution path in the design document.
- Workflow YAML files are authored by the user and stored in the filesystem. There is no UI-based workflow editor in this phase.
- Unknown fields in workflow YAML files are rejected (strict parsing) to prevent silent typos from causing unexpected behavior.
- External template files (`template_file:`) are resolved relative to the workflow file's directory.
- The CLI subcommands (`workflow run`, `workflow validate`) are added under the existing `mv-cli` binary as subcommands.
