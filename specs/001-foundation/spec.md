# Feature Specification: Foundation — Workspace & Local Model Integration

**Feature Branch**: `001-foundation`
**Created**: 2026-04-28
**Status**: Draft
**Input**: Phase 0 from docs/09-roadmap.md — Working Rust workspace that can call a local model and print a response.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Send a Prompt to a Local Model (Priority: P1)

As a developer, I want to run a CLI command with a natural-language
prompt and receive a response from a locally-running LLM so that I
can verify the end-to-end model integration works.

**Why this priority**: This is the core value proposition of Phase 0
— proving that the Rust workspace can talk to a local model. Without
this, nothing else matters.

**Independent Test**: Run the CLI binary with a prompt string and
confirm a coherent response is printed to stdout. Requires Ollama
running locally with at least one model pulled.

**Acceptance Scenarios**:

1. **Given** Ollama is running locally with a model available,
   **When** the user runs the CLI with a text prompt,
   **Then** the model's response is printed to stdout.

2. **Given** Ollama is running but no model is available,
   **When** the user runs the CLI with a text prompt,
   **Then** a clear, actionable error message is printed to stderr
   explaining that no model was found and suggesting how to pull one.

3. **Given** Ollama is not running,
   **When** the user runs the CLI with a text prompt,
   **Then** a clear error message is printed to stderr indicating
   the inference backend is unreachable.

---

### User Story 2 - Structured Logging Output (Priority: P2)

As a developer, I want structured log output from the CLI so that I
can observe what the system is doing (connecting, sending, receiving)
and diagnose problems during development.

**Why this priority**: Observability is a first-class concern per the
constitution (Principle VI). Basic tracing output enables debugging
from day one, even before OpenTelemetry is wired up.

**Independent Test**: Run the CLI with an increased verbosity flag
and confirm structured log lines (with timestamps, levels, and
spans) appear on stderr without interfering with the model response
on stdout.

**Acceptance Scenarios**:

1. **Given** a default CLI invocation,
   **When** the user runs a prompt,
   **Then** only the model response appears on stdout; no logs are
   printed at the default verbosity level.

2. **Given** the user sets a verbose flag or environment variable,
   **When** the user runs a prompt,
   **Then** structured log lines (timestamp, level, span context)
   appear on stderr showing connection, request, and response events.

---

### User Story 3 - Workspace Builds and Passes Checks (Priority: P3)

As a developer, I want the Cargo workspace to compile cleanly and
pass all quality gates (format, lint, test) so that I have a solid
foundation for iterative development.

**Why this priority**: The constitution requires Code Standards Gate
compliance (Principle IV) on every commit. Establishing this baseline
now prevents tech debt from accumulating.

**Independent Test**: Run `cargo fmt --check`, `cargo clippy -- -D
warnings`, and `cargo test` from the workspace root. All three MUST
pass with zero warnings and zero failures.

**Acceptance Scenarios**:

1. **Given** a fresh clone of the repository on the feature branch,
   **When** `cargo build --workspace` is run,
   **Then** the build succeeds with no errors.

2. **Given** the workspace builds,
   **When** `cargo fmt --check` is run,
   **Then** no formatting issues are reported.

3. **Given** the workspace builds,
   **When** `cargo clippy --all-targets --all-features -- -D warnings`
   is run,
   **Then** no lint warnings or errors are reported.

4. **Given** the workspace builds,
   **When** `cargo test` is run,
   **Then** all tests pass.

### Edge Cases

- What happens when the user provides an empty prompt string?
  The CLI MUST reject it with a clear error, not send an empty
  request to the model.
- What happens when the model returns an empty response?
  The CLI MUST handle it gracefully (e.g., print nothing or a
  brief notice), not panic or crash.
- What happens when the response is very long (thousands of tokens)?
  The CLI MUST print it without truncation or memory issues.
- What happens when Ollama is reachable but returns an HTTP error
  (e.g., 500)? The CLI MUST surface the error clearly, not swallow
  it silently.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST provide a CLI binary (`mv-cli`) that
  accepts a text prompt as a positional argument.
- **FR-002**: System MUST connect to a locally-running Ollama
  instance and send the prompt to a configured model.
- **FR-003**: System MUST print the model's response text to stdout.
- **FR-004**: System MUST print errors to stderr with actionable
  messages (what went wrong + what to do).
- **FR-005**: System MUST support a verbosity flag (`-v` / `--verbose`
  or `RUST_LOG` environment variable) that enables structured log
  output on stderr.
- **FR-006**: System MUST be organized as a Cargo workspace with
  separate crates for core types (`mv-core`) and the CLI binary
  (`mv-cli`).
- **FR-007**: System MUST exit with code 0 on success and non-zero
  on failure.
- **FR-008**: System MUST reject empty or whitespace-only prompts
  with a user-friendly error before contacting the model.
- **FR-009**: System MUST support a `--json` flag that outputs the
  response as a JSON object (`{"response": "..."}`) to stdout,
  enabling programmatic consumption by agents and scripts.

### Key Entities

- **Prompt**: The user's natural-language input text. Passed as a
  CLI argument; forwarded to the model backend.
- **Response**: The model's generated text. Received from the
  inference backend; printed to stdout.
- **ModelBackend**: An abstraction representing the connection to an
  inference provider (Ollama in this phase). Holds endpoint
  configuration and model name.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A user can send a prompt and receive a response in
  under 30 seconds (excluding model generation time — measured from
  CLI start to first output token).
- **SC-002**: All three quality gates (format, lint, test) pass with
  zero warnings on every commit.
- **SC-003**: Error scenarios (Ollama down, no model, empty prompt)
  produce understandable error messages that suggest corrective
  action.
- **SC-004**: The CLI binary starts and connects to the model
  backend without requiring any configuration file — sensible
  defaults (localhost:11434, a default model name) work out of
  the box.

## Assumptions

- Ollama is installed and running on the developer's machine at
  `localhost:11434` (the default).
- At least one model (e.g., `qwen3:8b` or similar) is pulled and
  available in Ollama.
- The developer has a working Rust toolchain (stable, recent
  edition).
- This phase does NOT include streaming output, multi-turn
  conversation, tool calling, MCP, or any cloud fallback — those
  are subsequent phases.
- The workspace will contain additional crates in the future
  (`mv-engine`, `mv-router`, `mv-mcp`, etc.) but only `mv-core`
  and `mv-cli` are created in this phase.
