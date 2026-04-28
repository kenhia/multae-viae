# Research: Foundation

**Feature**: 001-foundation
**Date**: 2026-04-28
**Status**: Complete

## Research Tasks

### R1: Rig + Ollama Integration Pattern

**Decision**: Use `rig::providers::ollama::Client` as the primary
model client. Connect to Ollama's default endpoint at
`http://localhost:11434`.

**Rationale**: Rig has a native Ollama provider — no need for a
custom HTTP client or OpenAI-compatible shim. The builder pattern
(`client.agent("model").preamble(...).build()`) maps directly to
our spec requirements.

**Alternatives considered**:
- Direct HTTP calls to Ollama's REST API — rejected because Rig
  already wraps this and adds structured error handling.
- `ollama-rs` crate — rejected because Rig's built-in provider is
  sufficient and avoids adding another dependency.

**Code pattern**:
```rust
use rig::providers::ollama;
use rig::completion::Prompt;

let client = ollama::Client::new("http://localhost:11434");
let agent = client.agent("qwen3:4b")
    .preamble("You are a helpful assistant.")
    .build();
let response = agent.prompt("user input here").await?;
```

### R2: Error Handling Strategy

**Decision**: Use `thiserror` for library error types in `mv-core`,
`anyhow` for application-level error handling in `mv-cli`.

**Rationale**: This is the standard Rust pattern — libraries define
typed errors for programmatic handling; binaries use `anyhow` for
ergonomic error propagation and user-facing messages.

**Alternatives considered**:
- `anyhow` everywhere — rejected because `mv-core` will be consumed
  by other crates that need typed errors.
- `miette` — rejected as over-engineered for this phase (fancy
  error reports not needed yet). Can revisit later.

### R3: CLI Framework

**Decision**: Use `clap` with derive macros for argument parsing.

**Rationale**: Clap is the de facto Rust CLI framework. Derive-based
API is ergonomic and generates help/version output automatically.
Positional arg for prompt, optional flags for verbosity and model.

**Alternatives considered**:
- `argh` — smaller but less capable; clap is the ecosystem standard.
- Manual `std::env::args` — too low-level for proper help text.

### R4: Tracing Setup

**Decision**: Use `tracing` + `tracing-subscriber` with
`EnvFilter` for structured logging to stderr.

**Rationale**: `tracing` is already the foundation for our
OpenTelemetry strategy (Phase 1). Starting with it now means we
only add the OTel exporter layer later — no instrumentation
rewrite needed.

**Configuration**:
- Default level: `warn` (quiet — only model response to stdout)
- Verbose mode: `info` or `debug` via `-v`/`-vv` flags or
  `RUST_LOG` environment variable
- Output: stderr only (stdout reserved for model responses)

**Alternatives considered**:
- `log` + `env_logger` — rejected because `tracing` is needed for
  OpenTelemetry in Phase 1 anyway. Starting with `log` would mean
  a migration later.
- `slog` — rejected; `tracing` has won the Rust ecosystem.

### R5: Workspace Layout

**Decision**: `crates/` directory for all workspace members.

**Rationale**: Keeps the project root clean as crate count grows.
`crates/mv-core/` and `crates/mv-cli/` now; `crates/mv-engine/`,
`crates/mv-router/`, etc. later.

**Alternatives considered**:
- Top-level directories per crate (`mv-core/`, `mv-cli/`) — works
  for 2 crates but gets messy at 8-10. `crates/` is the convention
  used by ripgrep, Rig itself, and most large Rust projects.

### R6: Task Runner

**Decision**: Use `just` (justfile) for development commands.

**Rationale**: The user explicitly requested a justfile. `just` is
simpler than `make`, supports argument passing, and works well for
Rust projects. Recipes for: build, check, test, run, fmt, lint.

**Alternatives considered**:
- `cargo-make` — more powerful but heavier; `just` is sufficient.
- `make` — works but Makefile syntax is clunky for this use case.
- `cargo xtask` — good for complex build logic but overkill here.

### R7: Default Model

**Decision**: Default to `qwen3:4b` with a `--model` flag for
override.

**Rationale**: Qwen 3 4B is small enough to run on most hardware,
fast, and capable enough for general chat. Users with better
hardware can override to `qwen3:8b` or any other Ollama model.

**Alternatives considered**:
- No default (require `--model` always) — rejected; spec requires
  zero-config defaults (SC-004).
- `llama3:8b` — viable but Qwen 3 is newer and performs well at
  the 4B size.

### R8: Rig Version & Features

**Decision**: Use `rig-core = "0.35"` with default features.

**Rationale**: 0.35.0 is the latest stable release (2026-04-13).
Uses edition 2024. Default features include `reqwest` and `rustls`
which are needed for HTTP connectivity to Ollama.

**Key features available** (for later phases):
- `rmcp` — MCP integration (Phase 3)
- `derive` — derive macros for tools (Phase 2)
