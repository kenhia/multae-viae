# Feature Specification: TRT-LLM Streaming & Hardening

**Feature Branch**: `007-trtllm-hardening`
**Created**: 2026-05-19
**Status**: Draft
**Input**: User description: "Phase 4.5.1 from docs/09-roadmap.md — Wire up streaming responses and improve robustness for TRT-LLM as a first-class local inference backend."

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Stream Tokens to the Terminal (Priority: P1)

A user runs the CLI against a TRT-LLM-served model with a new `--stream` flag.
Instead of waiting for the full response, they see tokens print to the terminal
as they arrive from the proxy. This makes long generations feel responsive and
lets the user judge response quality early.

**Why this priority**: Streaming is the headline deliverable of this sprint
(per the roadmap example). It dramatically improves perceived latency for the
interactive single-prompt path that 006 established.

**Independent Test**: Start the TRT-LLM proxy with a loaded model, run
`mv-cli -m llama-fp8 --stream "Explain Rust ownership"`, and observe tokens
appearing incrementally on stdout rather than as one final block. Verify the
final concatenated text matches a non-streaming run of the same prompt.

**Acceptance Scenarios**:

1. **Given** a healthy TRT-LLM proxy with a loaded model, **When** the user
   runs `mv-cli -m <trtllm-model> --stream "<prompt>"`, **Then** partial text
   chunks are written to stdout as they arrive and the process exits cleanly
   once the stream terminates.
2. **Given** the user omits `--stream`, **When** they run the same prompt,
   **Then** behavior is unchanged from sprint 006 (single buffered response).
3. **Given** the user passes `--stream` against a non-trtllm provider (e.g.
   an Ollama model), **When** the prompt is sent, **Then** the CLI either
   streams via that provider's streaming path or reports a clear "streaming
   not supported for this provider" message rather than silently buffering.
4. **Given** the proxy closes the SSE stream mid-response (network error),
   **When** the failure occurs, **Then** the CLI reports the partial output
   already printed plus a clear error and exits with a non-zero status.

---

### User Story 2 - Actionable Error When Model Is Not Loaded (Priority: P1)

A user runs a prompt against a TRT-LLM model that the proxy does not currently
have loaded. The proxy responds with HTTP 502. Instead of surfacing the raw
status code, the CLI prints a message telling the user exactly which command
to run (`just load <model>`) to fix the problem.

**Why this priority**: The 502-on-missing-model failure mode is the most
common friction point for the TRT-LLM workflow. A clear, actionable error
turns a confusing wall into a one-line fix and is required before this
provider is comfortable for daily use.

**Independent Test**: Ensure the proxy is running but no model is loaded, then
run `mv-cli -m llama-fp8 "hi"`. The CLI output must mention that the model is
not loaded and suggest the exact `just load llama-fp8` command (using the
configured model ID).

**Acceptance Scenarios**:

1. **Given** the proxy is reachable but returns HTTP 502 for a prompt,
   **When** the CLI surfaces the failure, **Then** the error message names
   the model that was not loaded and includes the suggested
   `just load <model>` command.
2. **Given** the proxy returns a different 5xx status (e.g. 500), **When**
   the CLI surfaces the failure, **Then** the error reports the status and
   the proxy's response body without claiming the model is unloaded.
3. **Given** the proxy is unreachable (connection refused), **When** the CLI
   surfaces the failure, **Then** the existing health-check error from
   sprint 006 still fires and is not replaced by the new 502 hint.

---

### User Story 3 - Token Usage Recorded in Telemetry (Priority: P2)

When a user runs a prompt against a TRT-LLM model, the OpenTelemetry span for
that call records the input and output token counts reported by the proxy.
Operators viewing telemetry can attribute cost, throughput, and context-window
usage to specific calls just as they can for OpenAI and Ollama providers.

**Why this priority**: Token telemetry is required to compare providers and to
keep TRT-LLM at parity with the other backends already instrumented in
sprint 002. It is not on the user's critical path for a single prompt but is a
prerequisite for routing decisions and cost analysis.

**Independent Test**: Enable OTLP export, run a TRT-LLM prompt, and inspect
the exported span. It must contain `gen_ai.usage.input_tokens` and
`gen_ai.usage.output_tokens` attributes with non-zero integer values matching
the proxy's reported counts.

**Acceptance Scenarios**:

1. **Given** the proxy returns approximate token counts in its response,
   **When** the CLI completes the call, **Then** the active span carries
   `gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens` attributes.
2. **Given** the proxy omits token counts, **When** the CLI completes the
   call, **Then** the span is recorded without those attributes (no error,
   no zero placeholders).
3. **Given** the call is a streaming call, **When** the stream terminates,
   **Then** the same token-usage attributes are attached to the span on
   completion.

---

### User Story 4 - Tool Calling Through TRT-LLM (Priority: P2)

A user runs an agent prompt that requires invoking a built-in tool (for
example, `file_list`) against a TRT-LLM model. The proxy's tool-calling
support is exercised end-to-end: the model emits a tool call, the CLI runs
the tool, the result is fed back to the model, and a final answer is
returned.

**Why this priority**: Tool calling is the bridge between the TRT-LLM
provider and the existing agent functionality from sprints 003 and 004.
Without an integration test, regressions in either the proxy or the
client could go unnoticed.

**Independent Test**: Run an integration test that points at the proxy with
a tool-capable model loaded, asks "list the files in the current directory",
and asserts both that `file_list` was invoked and that the final answer
mentions actual file names from the test fixture.

**Acceptance Scenarios**:

1. **Given** a tool-capable TRT-LLM model and the built-in `file_list` tool
   enabled, **When** the user asks a question that requires listing files,
   **Then** the model issues a tool call, the CLI executes the tool, and the
   final response references the tool output.
2. **Given** the same setup, **When** the model declines to use a tool,
   **Then** the CLI returns the model's direct answer without error.

---

### User Story 5 - Stop Sequences Prevent Runaway Generation (Priority: P2)

A user runs prompts against any model in the TRT-LLM registry without seeing
the model generate past natural stopping points (e.g. repeating role tokens,
emitting the next "user:" turn). Each registered TRT-LLM model has stop
sequences configured so that generations end cleanly.

**Why this priority**: Without correct stop sequences, some chat-tuned models
produce output that goes well past the intended reply, wasting time and
tokens and degrading user experience. Configuring this once per provider
fixes the entire registry.

**Independent Test**: For each model in the TRT-LLM registry, run a short
prompt that historically triggered runaway generation and verify the
response terminates at the expected stop point (no echo of the next turn's
role marker).

**Acceptance Scenarios**:

1. **Given** the TRT-LLM provider has per-model stop sequences configured,
   **When** a prompt is sent, **Then** the request includes those stop
   sequences and the response ends at or before them.
2. **Given** a model with no model-specific stop sequences configured,
   **When** a prompt is sent, **Then** a sensible provider-level default is
   applied so generation still terminates cleanly.

---

### User Story 6 - Workflow Step Uses a TRT-LLM Model End-to-End (Priority: P3)

A user runs a workflow that includes a prompt step bound to a TRT-LLM model.
The workflow engine drives the TRT-LLM provider just like Ollama or OpenAI,
including the new hardening features (token telemetry, stop sequences,
error messages). The step completes and its output flows into subsequent
steps.

**Why this priority**: Workflows were exercised against TRT-LLM in sprint 006
at a basic level. This sprint adds the end-to-end test that locks in the
hardened behavior so future changes don't quietly regress workflow execution.

**Independent Test**: Author a small workflow YAML with at least one prompt
step bound to a TRT-LLM model, run it via `mv-cli workflow run`, and assert
the step's output is captured and any downstream step that depends on it
sees the value.

**Acceptance Scenarios**:

1. **Given** a workflow with a TRT-LLM-backed prompt step, **When** the
   workflow runs against a healthy proxy with the model loaded, **Then**
   the step completes successfully and its output is stored in the run
   context.
2. **Given** the same workflow runs while the model is not loaded, **When**
   the step executes, **Then** the workflow surfaces the actionable 502
   error from User Story 2 and fails the step cleanly rather than hanging.

---

### Edge Cases

- The user passes `--stream` together with a workflow run (where streaming
  to stdout may not make sense): the CLI must define and document a single
  predictable behavior (either ignore the flag for workflows or stream the
  active step).
- The proxy emits an SSE `[DONE]` sentinel with no preceding content (empty
  (empty response): the CLI must exit cleanly with empty output, not
  hang. Accepted without a dedicated test in this sprint — adding a
  mock SSE harness is out of scope; the live `--stream` test covers
  the non-empty path.
- The proxy returns token counts as floats or strings: the CLI must coerce
  to integers and record them, not crash.
- A model has stop sequences that overlap with legitimate user content
  (rare): the configuration is still applied — this is accepted as a known
  trade-off and documented in the model registry.
- The proxy returns 502 for a cause other than "model not loaded": the
  CLI still surfaces the `Run: just load <model>` hint because the
  classifier does not probe the response body. This is accepted as a
  known trade-off — 502 from any other cause is rare against this proxy,
  and the actionable hint is harmless if the user runs it.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The CLI MUST accept a `--stream` flag on the prompt
  subcommand and, when set against a TRT-LLM model, request a server-sent
  event stream from the proxy and write chunks to stdout as they arrive.
- **FR-002**: The CLI MUST be able to drive a streaming TRT-LLM
  completion without going through the buffered path. The streaming
  entry point lives in the CLI today; refactor into `mv-core::trtllm`
  is deferred until the workflow engine needs it.
- **FR-003**: When the proxy returns HTTP 502 for a prompt request, the
  CLI MUST surface an error message that names the requested model and
  includes the literal suggestion `just load <model>` (substituting the
  configured model identifier).
- **FR-004**: The TRT-LLM provider MUST extract input and output token
  counts from proxy responses (both streaming and non-streaming) and
  attach them to the active telemetry span as `gen_ai.usage.input_tokens`
  and `gen_ai.usage.output_tokens`.
- **FR-005**: The CLI MUST support tool-calling round-trips through the
  TRT-LLM provider for built-in tools (verified against `file_list`),
  using the same tool-execution path established in sprint 003.
- **FR-006**: The TRT-LLM provider MUST allow stop sequences to be
  configured per model in the model registry and MUST include them in
  every prompt and stream request to the proxy.
- **FR-007**: The TRT-LLM provider MUST apply a sensible default stop
  sequence set when a registered model does not declare its own, so that
  no model in the shipped registry produces runaway generations on the
  smoke prompts used in tests.
- **FR-008**: The workflow engine MUST be able to execute a prompt step
  bound to a TRT-LLM model end-to-end, propagating both successful
  outputs and the actionable error messages defined in FR-003.
- **FR-009**: When streaming fails partway through (network error, proxy
  disconnect), the CLI MUST report a clear error in addition to whatever
  text has already been printed and exit with a non-zero status.
- **FR-010**: All existing tests (123 as of merge of sprint 006) MUST
  continue to pass; new behavior MUST be covered by additional integration
  tests in `crates/mv-cli/tests/` and unit tests in `crates/mv-core/`.
- **FR-011**: When the user passes both `--stream` and `--json`, the
  CLI MUST emit a warning to stderr ("--json overrides --stream; falling
  back to buffered JSON output") and fall back to the buffered JSON
  output path. Streaming is suppressed because incremental token chunks
  are not valid JSON.

### Key Entities *(include if feature involves data)*

- **Streaming session**: A live request/response interaction with the
  TRT-LLM proxy that emits incremental text chunks and finishes with a
  terminator plus optional token-usage summary.
- **Token usage record**: A pair of integer counters (input, output)
  attached to a single completion or streaming session and surfaced as
  span attributes.
- **Stop sequence set**: The collection of strings configured per model
  (with a provider-level default) that the proxy uses to terminate
  generation.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: Running the deliverable command
  `mv-cli -m <trtllm-model> --stream "Explain Rust ownership"` against a
  healthy proxy prints visible token output within 1 second of the request
  being accepted, with subsequent chunks arriving as the proxy produces
  them.
- **SC-002**: When the requested TRT-LLM model is not loaded, the user
  sees an error message that includes the exact `just load <model>`
  command in 100% of cases (verified by integration test).
- **SC-003**: For every TRT-LLM call (streaming and non-streaming), the
  exported telemetry span carries integer `gen_ai.usage.input_tokens` and
  `gen_ai.usage.output_tokens` attributes whenever the proxy supplies
  them.
- **SC-004**: An agent prompt requiring a built-in tool completes
  successfully through the TRT-LLM provider in the integration test
  suite, with the tool invocation captured in the test assertions.
- **SC-005**: Every model in the shipped TRT-LLM registry passes a
  runaway-generation smoke test (response terminates at a configured stop
  sequence or natural end-of-turn, not at the max-token limit) for the
  test prompts.
- **SC-006**: A workflow whose prompt step is bound to a TRT-LLM model
  runs to completion in the end-to-end test and produces the expected
  downstream value in the run context.
- **SC-007**: The full test suite continues to pass with at least the
  pre-sprint 123 tests plus the new tests added in this sprint, verified
  by three consecutive `just ci` runs with zero flakes.

## Assumptions

- The TRT-LLM proxy (`trt-llm-explore`) is already updated to support SSE
  streaming, tool calling, and approximate token counts; this sprint is
  purely the client-side work.
- The proxy continues to expose its OpenAI-compatible API at
  `http://localhost:8003` by default, accessible via
  `rig::providers::openai::CompletionsClient`.
- Rig 0.35's SSE streaming for `CompletionsClient` is usable as-is and
  does not require forking or patching for our use case.
- Stop sequences are configured per model in the existing model registry
  format; adding optional stop-sequence fields is a non-breaking change.
- The 502-on-missing-model contract is stable enough to key user-facing
  guidance on; if the proxy later changes this code or body, the CLI
  message will be updated in a follow-up.
- Streaming output goes to stdout by design; capturing or piping the
  stream is the user's responsibility.
- Workflow streaming UX is out of scope for this sprint — workflows
  continue to use the buffered path unless trivially extended.
