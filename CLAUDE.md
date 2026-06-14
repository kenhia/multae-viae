# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

**multae-viae** ("many paths") — a local-first agentic controller in Rust that orchestrates
multiple LLMs, tools, and services. Models run locally via Ollama / TensorRT-LLM / mistral.rs
with cloud fallback; tools come from built-ins plus MCP servers; multi-step work is described
in a YAML workflow DSL; everything emits OpenTelemetry traces.

Built on **Rig** (`rig-core`) for the agent framework, **RMCP** for MCP, and
`opentelemetry` + `tracing` for observability. Rust edition 2024, `resolver = "3"`.

## Commands

Use `just` (the canonical task runner) — do not hand-roll cargo invocations when a recipe exists.

```bash
just build            # cargo build --workspace
just test             # cargo test --workspace
just ci               # fmt --check + clippy -D warnings + test  (the gate — must pass before commit)
just lint             # fmt + clippy (mutating; ci is the non-interactive variant)
just run "prompt" --json -m qwen3:8b      # run the CLI (FLAGS go before the quoted prompt)
just test-trtllm      # cargo test -p mv-cli -- --ignored  (LIVE proxy tests, see below)
```

Run a single test: `cargo test -p mv-core effective_stop_sequences` or
`cargo test -p mv-cli --test cli_trtllm json_overrides_stream_with_warning`.

**Live-proxy tests are `#[ignore]`d**, not deleted — they require a running TRT-LLM OpenAI
proxy on `http://localhost:8003` with a model loaded. `just test` / `just ci` stay green
offline by skipping them; `just test-trtllm` opts them in. When adding a test that hits an
external backend, mark it `#[ignore]` so the default suite stays hermetic.

## Architecture

Three-crate workspace:

- **`mv-core`** — the library: all reusable logic — provider primitives, the **agent runtime**
  (`runtime.rs`: dispatch, fallback, the `AnyAgent` held-session enum, the workflow executors),
  the klams memory impl (`memory/klams.rs`), the MCP lifecycle manager, and the workflow engine.
- **`mv-cli`** — the binary: clap parsing, the stdout/stderr output contract, TRT-LLM terminal
  streaming (`stream.rs`), and telemetry wiring. A thin caller of `mv-core`.
- **`mv-server`** — the REST controller daemon (sprint 013): an axum API, held-open sessions, and
  a cron scheduler over the same `mv-core` runtime. Also a thin caller. See
  [docs/12-mv-server.md](docs/12-mv-server.md).

The standing rule: **anything a front end needs lives in `mv-core`**; the binaries hold only their
own concerns.

### Provider dispatch (the central seam)

[crates/mv-core/src/runtime.rs](crates/mv-core/src/runtime.rs) `complete()` resolves a `ModelEntry`
and branches on `entry.provider` to one of `call_ollama`, `call_openai`, or `call_trtllm` (plus
`mv-cli`'s `stream_trtllm`). Each builds a Rig agent — via the shared `ollama_agent`/`openai_agent`/
`trtllm_agent` builders — with `SYSTEM_PREAMBLE`, attaches the unified tool set, and runs a
multi-turn agentic loop. `complete_chain()` walks a fallback chain over `complete()`; both `mv-cli`
and `mv-server` call these.

`ModelEntry` ([crates/mv-core/src/lib.rs](crates/mv-core/src/lib.rs)) is the config-to-runtime
bridge: methods like `endpoint()`, `model_name()` (`served_name` ?? `id`), `locality()`, and
`effective_stop_sequences()` resolve explicit-or-defaulted values so call sites never special-case
providers inline. **Add provider behavior here as a resolved method, not as scattered match arms.**

`--stream` is **TRT-LLM only**; any other provider returns `StreamingNotSupported`. `--json`
overrides `--stream` (buffered JSON wins, with a stderr warning).

### TRT-LLM specifics ([crates/mv-core/src/trtllm/](crates/mv-core/src/trtllm/))

- `health.rs` — proxy reachability preflight, run before a TRT-LLM call.
- `stop.rs` — provider-default stop sequences (`</s>`, `<|im_end|>`, `<|eot_id|>`) and
  `request_stop_value()`, forwarded to the proxy via Rig `additional_params`.
- `usage.rs` — `Usage` (token counts) with a lenient `Deserialize` (int/float/string → `u64`)
  and `Usage::from_rig()` to populate `gen_ai.usage.{input,output}_tokens` span attributes.
- The 502 classifier in `classify_rig_error()` maps a not-loaded model to
  `MvError::ModelNotLoaded { hint: "Run: just load <id>" }`. This classifier is **shared** by
  buffered, streaming, and workflow paths — keep it the single source of that mapping.

### Tools & MCP

Built-in tools live in [crates/mv-core/src/tools/](crates/mv-core/src/tools/) (`file_list`,
`file_read`, `shell_exec`, `http_get` — each a Rig `Tool`). MCP servers
([crates/mv-core/src/mcp/](crates/mv-core/src/mcp/)) are configured via `mcp-servers.yaml` and
**merge into the same tool set** the model sees. Built-in tools take precedence on name collision;
MCP server failures are logged and skipped, not fatal; all MCP connections shut down gracefully on
exit. Tool output truncates at 10,000 chars.

### Workflow engine ([crates/mv-core/src/workflow/](crates/mv-core/src/workflow/))

`execute_workflow()` is generic over the `PromptExecutor` and `ToolExecutor` traits, so the engine
in `mv-core` has **no dependency on Rig or any provider** — it is unit-tested with mock executors.
`mv-cli` provides the real `RigPromptExecutor` / tool executor. Steps are `prompt` | `tool` |
`transform`; `{{var}}` templating pulls from an `ExecutionContext` where step outputs shadow
workflow inputs. Tool steps support skip/fail/retry error handling. When wiring TRT-LLM behavior
into workflows, route through the same shared classifier/usage code rather than duplicating it.

### Errors & telemetry

`MvError` ([crates/mv-core/src/lib.rs](crates/mv-core/src/lib.rs)) is the single error enum; every
variant's `Display` is an **actionable, user-facing** message (often with a `hint`). Errors go to
stderr, results to stdout, both honor `--json`. Model calls, tool invocations, and workflow steps
are `tracing`-instrumented; `--otlp [URL]` exports spans (default `http://localhost:4318`, view in
Jaeger). Span/attribute names follow OpenTelemetry GenAI conventions (`gen_ai.*`).

## Working conventions (from the project constitution)

This repo is **spec-driven** ([.specify/memory/constitution.md](.specify/memory/constitution.md)).
The user calls each spec block a **"sprint"** — they map to `specs/NNN-name/` directories. The
active sprint is the highest-numbered one (currently `specs/013-mv-server/`); its
`tasks.md` is the live checklist and `plan.md` the technical context.

- **No code change without a spec entry.** Ad-hoc changes go in the active spec or
  `specs/supplemental-spec.md`.
- **TDD is mandatory** — write the failing test first (or alongside). Cross-crate boundaries and
  external-service interactions require integration tests.
- **Docs are part of "done", not a follow-up** — update `docs/` (especially
  `docs/01-architecture-design.md`) and `README.md` in the polish phase of each sprint.
- **The Code Standards Gate (`just ci`) must pass clean before any commit** — fmt, clippy
  `-D warnings`, and tests, on new *and* existing code.
- **YAGNI / simplicity** — defensive coding at system boundaries only (user input, external APIs,
  MCP messages, file I/O); trust internal code.

`.scratch-agent/` is your gitignored scratch space; `.scratch/` is the user's.

## Markdown formatting

Mirrors the GitHub Copilot instruction at
[.github/instructions/markdown.instructions.md](.github/instructions/markdown.instructions.md)
(applies to all `*.md`):

- When consecutive lines should render as separate lines (not a merged paragraph), end each
  line except the last with **two trailing spaces** to produce a soft line break.
- This commonly applies to key-value metadata lines like `**Key**: value` that appear on
  consecutive lines with no blank line between them.
