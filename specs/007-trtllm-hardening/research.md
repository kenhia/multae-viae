# Research: TRT-LLM Streaming & Hardening

## Overview

Sprint 006 landed TRT-LLM as a buffered, OpenAI-compatible provider keyed off
`openai::CompletionsClient` (deliberately not the default `openai::Client`,
which targets the Responses API that the TRT-LLM proxy does not implement).
Sprint 007 turns the integration into a daily-driver experience: streaming
output, actionable error messages, token telemetry, tool calling, and stop
sequences. This document resolves the open technical questions from
[spec.md](spec.md).

## R1: Rig 0.35 streaming on `CompletionsClient` (FR-001, FR-002, FR-009)

**Decision**: Use `agent.stream_prompt(text).multi_turn(10).await`, then
iterate with `StreamExt::next`, matching on `MultiTurnStreamItem`.

**Rationale**: Rig 0.35 exposes `StreamingPrompt<M, M::StreamingResponse>`
for any agent whose model's `StreamingResponse` implements `GetTokenUsage`.
The `openai::CompletionsClient` (`Provider = CompletionsApi`) does — its
`StreamingCompletionResponse` lives at
`rig::providers::openai::completion::streaming::StreamingCompletionResponse`
and the streaming `stream()` impl automatically merges
`{"stream": true, "stream_options": {"include_usage": true}}` into the
request body, so the proxy is asked to emit a final `usage` chunk.

The returned stream yields `MultiTurnStreamItem` values; the variants we care
about are:

- `MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(s))`
  — partial text deltas. Write each `s` to stdout and flush.
- `MultiTurnStreamItem::FinalResponse { response, .. }` (variant shape per
  Rig docs/cassette tests) — terminal item carrying the aggregate
  `StreamingCompletionResponse` from which `GetTokenUsage::token_usage()`
  yields `prompt_tokens` and `completion_tokens`.

Tool calls are handled inside `multi_turn`: the same `ToolServerHandle`
attached at agent build time (the existing `tool_server_handle(handle)` +
`default_max_turns(10)` pattern is replaced with `.multi_turn(10)` on the
stream) drives the tool execution between assistant turns. The stream
re-opens after each tool round-trip, so the user sees the final assistant
turn as deltas just like a single-turn stream.

**Alternatives considered**:

- `rig::agent::stream_to_stdout(&agent, &mut stream)` — convenience helper.
  Rejected because (a) it discards token-usage info needed for telemetry,
  and (b) it requires `&'static Agent<M>`, which doesn't fit the
  per-request agent we build from `CompletionsClient`.
- Hand-rolling SSE parsing via `reqwest` — would duplicate the entire
  `openai_chat_completions_compatible` machinery and lose the tool-calling
  loop. Explicitly rejected.
- Forcing the existing `prompt()` path and chunking server-side — defeats
  the purpose; SC-001 mandates incremental visible output within 1 s.

**Implementation outline** (`stream_trtllm` in `crates/mv-cli/src/main.rs`):

```rust
use futures_util::StreamExt;
use rig::client::CompletionClient;
use rig::streaming::{StreamingPrompt, MultiTurnStreamItem, StreamedAssistantContent};
use std::io::Write;

let client = rig::providers::openai::CompletionsClient::builder()
    .api_key("tensorrt_llm")
    .base_url(endpoint).build()?;
let agent = client.agent(entry.model_name())
    .preamble(SYSTEM_PREAMBLE)
    .tool_server_handle(handle)
    .additional_params(stop_sequences_json(entry)?)  // see R4
    .build();
let mut stream = agent.stream_prompt(prompt).multi_turn(10).await;

let mut stdout = std::io::stdout().lock();
let mut final_usage = None;
while let Some(item) = stream.next().await {
    match item.map_err(classify_stream_error)? {
        MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(t)) => {
            stdout.write_all(t.as_bytes())?;
            stdout.flush()?;
        }
        MultiTurnStreamItem::FinalResponse(r) => {
            final_usage = r.token_usage();
        }
        _ => {}
    }
}
writeln!(stdout)?;  // ensure trailing newline
record_usage_on_span(final_usage);
```

The exact `MultiTurnStreamItem` enum shape is finalized at implementation
time against the locked-in `rig-core 0.35` API; the contract is "one text
delta variant + one terminal variant carrying the streaming response", which
all examples and cassette tests in the rig repo confirm.

## R2: Stream + tool-calling coexistence (FR-005)

**Decision**: Reuse the single agent loop — `stream_prompt(...).multi_turn(N)`.

**Rationale**: Rig's streaming agent loop already handles the tool round-trip
internally. The stream emits assistant text deltas for the final turn only;
intermediate tool calls produce `MultiTurnStreamItem` variants we can ignore
for stdout purposes (or surface as `debug!` logs). The `ToolServerHandle`
attached via `tool_server_handle()` is identical to the buffered path.
Sprint 006's existing tool-call telemetry continues to fire because Rig
emits its own spans around tool dispatch.

**Verified via**: `rig` repo `tests/providers/openai/cassette/completions_api.rs`
demonstrates `agent.stream_prompt(P).multi_turn(8).await` driving two tool
calls before the final text — exactly our use case.

## R3: 502 → "model not loaded" classification (FR-003)

**Decision**: Add `MvError::ModelNotLoaded { model: String, hint: String }`.
In `call_trtllm()` / `stream_trtllm()`, when the Rig error string contains
the substring `502` (or, more robustly, `"status code: 502"` which is what
`reqwest`/`rig` produce), map to `ModelNotLoaded` with
`hint = format!("Run: just load {model}")`.

**Display format**: `"Model '{model}' is not loaded on the TRT-LLM proxy. {hint}"`.

**Rationale**: The proxy returns HTTP 502 specifically when the requested
model isn't loaded (per the trt-llm-explore handoff). The existing
`classify_rig_error` already inspects the stringified error; adding a
single `msg.contains("502")` branch keeps the boundary-only defensive
handling pattern. We do **not** also probe `/v1/models` — the spec requires
"clear, actionable error", not "guaranteed unique cause attribution"; the
edge case "502 from an unrelated cause" is explicitly accepted as a known
trade-off (spec edge cases) — the `just load <model>` hint is harmless if
the user runs it.

**Why a new variant rather than reusing `BackendUnreachable`**: the existing
variant carries `endpoint` semantics ("the server is down"). 502-on-missing-
model is a different operator action (load a model, not start the server),
and the spec mandates the literal `just load <model>` suggestion. A
distinct variant keeps the Display unambiguous and lets the workflow
engine surface it identically to `BackendUnreachable` without conflating
the two causes.

**500 / other 5xx**: fall through the existing `BackendUnreachable` /
`CompletionFailed` branches unchanged (acceptance scenario 2 of US2).

## R4: Stop sequences (FR-006, FR-007)

**Decision**: Add `stop_sequences: Option<Vec<String>>` to `ModelEntry`
(non-breaking, defaults to `None`). Provide a provider-level default
`default_stop_sequences()` returning `vec!["</s>".to_string(),
"<|im_end|>".to_string(), "<|eot_id|>".to_string()]` — the union of
chat-tuned-model role terminators known to leak from llama/qwen-family
checkpoints.

**Forwarding mechanism**: Rig's `CompletionRequest` supports
`additional_params: Option<serde_json::Value>` merged into the outgoing
body. Inject `{"stop": [...]}` via `AgentBuilder::additional_params(...)`
when building the agent for both buffered and streaming paths.

**Rationale**: The OpenAI Chat Completions schema accepts `stop` as
`string | array | null`. `trtllm-serve` honors it. Configuring via
`additional_params` avoids any Rig API extension and applies uniformly to
both `prompt()` and `stream_prompt()`. The default set covers the models
we currently ship; per-model overrides via the new YAML field handle the
rest.

**Alternatives considered**:

- Build a Rig provider patch to surface `stop` as a first-class builder
  field. Rejected — over-engineering for a 1-line additional param.
- Skip defaults and require per-model stop_sequences. Rejected — violates
  FR-007 (default must be sensible enough that no shipped model produces
  runaway output on smoke prompts).

## R5: Token usage extraction (FR-004, SC-003)

**Decision**: Use the `GetTokenUsage` trait, which both
`openai::CompletionResponse` and `openai::StreamingCompletionResponse`
implement.

- **Buffered path (`call_trtllm`)**: today the call goes through
  `agent.prompt(prompt)` which returns the assistant text string and
  discards the underlying `CompletionResponse`. To extract usage we drop
  one layer and call `agent.completion(prompt, []).await?.send().await?`
  (or equivalently `agent.chat(...)`), which yields
  `CompletionResponse<openai::CompletionResponse>`. Then
  `response.usage` gives `Usage { input_tokens, output_tokens, ... }`.
- **Streaming path**: the terminal `MultiTurnStreamItem` carries the
  aggregate streaming response; call `.token_usage()` (provided by
  `GetTokenUsage`) on it.

**Span attribute recording**: use `tracing::Span::current().record(...)`
inside the existing `#[tracing::instrument(name = "llm_completion", ...)]`
function. Declare the fields up-front as `tracing::field::Empty` so they
appear in the span schema even when the proxy omits usage (Principle VI:
"never zero placeholders"):

```rust
#[tracing::instrument(name = "llm_completion", skip(handle), fields(
    gen_ai.system = "trtllm",
    gen_ai.request.model = %entry.model_name(),
    gen_ai.usage.input_tokens = tracing::field::Empty,
    gen_ai.usage.output_tokens = tracing::field::Empty,
    // ... existing trtllm.* fields
))]
```

Then after the call:

```rust
if let Some(usage) = extracted {
    tracing::Span::current().record("gen_ai.usage.input_tokens", usage.input);
    tracing::Span::current().record("gen_ai.usage.output_tokens", usage.output);
}
```

If the proxy returns the counts as floats or strings (spec edge case),
serde's `u64` deserialization will fail. We deal with that exactly once,
at the boundary, in `mv-core::trtllm::usage::Usage` (custom
`Deserialize` that coerces via `serde_json::Number::as_u64()` /
`String::parse()`).

**Rationale**: Centralizing extraction in `mv-core::trtllm::usage` means the
workflow path (`RigPromptExecutor::execute_prompt`) and the streaming CLI
path use the same code; the tests cover the coercion edge cases once.

## R6: `#[ignore]` discipline for live-proxy tests (FR-010, SC-007)

**Decision**: Tests that require the live proxy (`http://localhost:8003/v1`
healthy + a loaded model) are gated with `#[ignore = "requires TRT-LLM proxy"]`
exactly like the existing `cli_trtllm.rs` style sprint 006 used. Default
`cargo test` runs the 123 existing tests plus the unit tests added in
`mv-core` (which mock at the parsing/HTTP-classification level) and the
new CLI integration tests that only exercise config parsing and the
unreachable-endpoint path. The live tests run via `cargo test --
--ignored` on demand or in a dedicated `just test-trtllm` recipe.

**Rationale**: Established sprint 006 pattern (`cli_trtllm.rs` already
mixes online and offline tests by using `127.0.0.1:19999` as a guaranteed-
closed port for the offline ones). Keeps `just ci` deterministic.

## R7: `--stream` against non-TRT-LLM providers (FR-001 edge case)

**Decision**: For sprint 007, `--stream` against `provider: ollama` or
`provider: openai` returns
`MvError::CompletionFailed { details: "streaming is only supported for TRT-LLM models in this release" }`.

**Rationale**: Spec acceptance scenario 1.3 explicitly allows either path —
"stream via that provider's streaming path or reports a clear … not
supported message". Adding streaming for Ollama and OpenAI is out of scope
for this sprint and risks regressing the existing buffered paths. The
clear error message satisfies the spec; a follow-up sprint can lift the
restriction.

## R8: `--stream` inside `workflow run` (edge case in spec)

**Decision**: `--stream` is **not accepted** on `workflow run`; the flag
only exists on the `prompt` subcommand. Workflow steps continue to use the
buffered path, which now also benefits from the token telemetry and 502
mapping (FR-008). No CLI flag means no ambiguous user expectation.

**Rationale**: The spec edge case warns about flag-vs-workflow interaction
and explicitly allows "ignore the flag for workflows or stream the active
step". Restricting `--stream` to `prompt` is the simplest interpretation
that satisfies the edge case unambiguously and matches the existing
clap layout (`PromptArgs` is the natural carrier).

## R9: Stream failure → non-zero exit (FR-009)

**Decision**: Any `Err` from `stream.next()` is mapped through the existing
`classify_rig_error` (with the new 502 branch) and propagated up. The
already-printed text remains on stdout; the classified error goes to
stderr; the process exits with code 1 via the existing top-level error
handling in `main()`.

**Rationale**: Matches Principle VI (stdout for results, stderr for
errors) and avoids any new error-aggregation machinery. The acceptance
scenario only requires "report partial output already printed plus a clear
error and exits non-zero" — which is the default behavior once we propagate
the error.
