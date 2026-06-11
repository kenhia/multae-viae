# Tasks: TRT-LLM Streaming & Hardening

**Input**: Design documents from `/specs/007-trtllm-hardening/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/cli.md, quickstart.md

**Tests**: Tests ARE included — the spec mandates them (FR-010, SC-007) and
the project follows TDD per Principle III. Live-proxy tests are marked
`#[ignore]` per research R6 so `cargo test` stays green offline.

## Format

```text
- [ ] [TaskID] [P?] [Story?] Description with file path
```

- `[P]` — parallelizable (different files, no dependencies on in-flight work)
- `[US#]` — maps to user story from spec.md
- Setup / Foundational / Polish phases carry no story label

## Path Conventions

Rust workspace at repo root: `crates/mv-core/` (library) and `crates/mv-cli/`
(binary). Tests co-located: unit tests inside `src/**`, integration tests
under `crates/<crate>/tests/`. Design docs under `specs/007-trtllm-hardening/`.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add dependencies and shared utilities that all subsequent phases
build on.

- [X] T001 Verify `futures-util` is available to `mv-cli` (transitive through
  `rig-core`); if not, add `futures-util = "0.3"` to `crates/mv-cli/Cargo.toml`
  with only the `default` feature.
- [X] T002 Add a `just test-trtllm` recipe to `justfile` that runs
  `cargo test -- --ignored` scoped to `mv-cli` so live-proxy tests are
  reachable without changing default `just ci` behavior.

**Checkpoint**: Build deps are in place; `just ci` still green.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Data-model and provider primitives that every user story
consumes. **Must complete before any US phase starts.**

- [X] T003 [P] Add optional `stop_sequences: Option<Vec<String>>` field to
  `ModelEntry` in `crates/mv-core/src/lib.rs` and update the existing
  `ModelEntry` doc comment / serde derive (no `deny_unknown_fields` change
  needed since the struct already tolerates new optional fields).
- [X] T004 [P] Add `MvError::ModelNotLoaded { model: String, hint: String }`
  variant to the `MvError` enum in `crates/mv-core/src/lib.rs` with the
  `Display` format
  `"Model '{model}' is not loaded on the TRT-LLM proxy. {hint}"`
  per data-model.md.
- [X] T005 [P] Create `crates/mv-core/src/trtllm/usage.rs` with the `Usage`
  struct (`input_tokens: u64`, `output_tokens: u64`), a custom
  `Deserialize` that coerces integer / float / numeric-string inputs to
  `u64`, and a `Usage::from_rig<R: rig::completion::GetTokenUsage>(&R) ->
  Option<Self>` extractor (research R5).
- [X] T006 [P] Create `crates/mv-core/src/trtllm/stop.rs` exposing
  `pub fn default_stop_sequences() -> Vec<String>` returning
  `vec!["</s>".into(), "<|im_end|>".into(), "<|eot_id|>".into()]`
  and `pub fn request_stop_value(entry: &ModelEntry) -> Option<serde_json::Value>`
  that builds `{"stop": [...]}` from the merged (entry override or default)
  list (research R4).
- [X] T007 Register the new modules in `crates/mv-core/src/trtllm/mod.rs`
  (`pub mod usage; pub mod stop;`) so they are reachable from `mv-cli`.
- [X] T008 Add helper `impl ModelEntry { pub fn effective_stop_sequences(&self)
  -> Option<Vec<String>> { ... } }` in `crates/mv-core/src/lib.rs` returning
  the explicit list if set, else the TRT-LLM provider default when
  `provider == "trtllm"`, else `None` (data-model.md).
- [X] T009 [P] Unit tests in `crates/mv-core/src/trtllm/usage.rs` (in a
  `#[cfg(test)] mod tests` block) covering: integer counts, float counts,
  string counts, missing `usage` block returns `None`.
- [X] T010 [P] Unit tests in `crates/mv-core/src/trtllm/stop.rs` covering:
  default returned when entry has no `stop_sequences`, explicit list
  preserved verbatim, `request_stop_value` JSON shape exact match.
- [X] T011 [P] Unit tests in `crates/mv-core/src/lib.rs` (extend existing
  `tests` module): parse a `models.yaml` snippet containing
  `stop_sequences: ["<|eot_id|>"]` and assert
  `entry.effective_stop_sequences()` returns it; assert
  `MvError::ModelNotLoaded` `Display` matches the contract string exactly.

**Checkpoint**: `cargo test -p mv-core` green; provider primitives ready
for both buffered and streaming consumers.

---

## Phase 3: User Story 2 — Actionable 502 Error (Priority: P1) 🎯 MVP

**Goal**: When the proxy returns HTTP 502 for a prompt request, the CLI
emits `Error: Model '<id>' is not loaded on the TRT-LLM proxy. Run: just
load <id>` to stderr and exits 1.

**Independent Test**: With proxy running but no model loaded, run
`mv-cli -m llama-fp8 "hi"`; assert stderr contains
`Run: just load llama-fp8` and exit code is 1.

**Why first**: Smallest unit of value, no new code paths in the agent
loop, and unblocks tool-calling and workflow stories (US4, US6) which
inherit the same mapping. Sequenced ahead of US1 because it has zero
dependency on Rig streaming research uncertainties.

### Tests for User Story 2

- [X] T012 [P] [US2] Add offline unit test in
  `crates/mv-cli/src/main.rs` (inside `#[cfg(test)] mod tests`) for the
  502-classifier helper: feed a stringified rig error containing
  `"status code: 502"` and assert it maps to
  `MvError::ModelNotLoaded { model: "llama-fp8", hint: "Run: just load llama-fp8" }`.
- [X] T013 [P] [US2] Add offline unit test for a 500 error string: assert
  it does **not** map to `ModelNotLoaded` (falls through to existing
  `CompletionFailed` / `BackendUnreachable` classification).
- [X] T014 [US2] Add `#[ignore]` integration test
  `trtllm_502_emits_just_load_hint` in `crates/mv-cli/tests/cli_trtllm.rs`
  that runs `mv-cli -m llama-fp8 "ping"` against a live proxy (no model
  loaded), asserts stderr matches `Run: just load llama-fp8` (use
  `predicates::str::contains`) and exit code 1.

### Implementation for User Story 2

- [X] T015 [US2] In `crates/mv-cli/src/main.rs`, extend the existing
  TRT-LLM rig-error classifier (the helper that today produces
  `BackendUnreachable`/`CompletionFailed`) to add a `msg.contains("502")`
  branch returning `MvError::ModelNotLoaded { model: entry.id.clone(),
  hint: format!("Run: just load {}", entry.id) }`. Keep the connection-
  refused / unreachable path ahead of it so US2 acceptance scenario 3
  still triggers `BackendUnreachable` instead. No changes to the
  top-level error printer are expected; the offline tests from T012 +
  T013 exercise this path end-to-end.

**Checkpoint**: US2 fully shippable on its own. `just ci` green; the
ignored test passes when run against a live proxy without a model loaded.

---

## Phase 4: User Story 1 — Stream Tokens to the Terminal (Priority: P1) 🎯 MVP

**Goal**: `mv-cli -m <trtllm-model> --stream "<prompt>"` streams assistant
text to stdout as the proxy emits it; clean trailing newline; exit 0 on
success, 1 on stream error with partial output preserved.

**Independent Test**: Per quickstart.md step 4 — run the deliverable
command, observe incremental output, compare final concatenated text to a
buffered run of the same prompt.

**Depends on**: T003–T008 (foundational), T015 (the streaming path reuses
the 502 classifier).

### Tests for User Story 1

- [X] T017 [P] [US1] Add offline integration test
  `stream_flag_rejected_for_non_trtllm_provider` in
  `crates/mv-cli/tests/cli_trtllm.rs` that configures an Ollama model in a
  temp `models.yaml`, runs `mv-cli --stream "hi"` against it, and asserts
  stderr contains
  `streaming is only supported for TRT-LLM models in this release`
  with exit code 1 (research R7, contract `cli.md`).
- [X] T018 [P] [US1] Add offline integration test
  `stream_flag_known_to_clap` that runs `mv-cli --help` and asserts the
  `--stream` flag appears in the prompt subcommand help output.
- [X] T019 [US1] Add `#[ignore]` live-proxy integration test
  `trtllm_stream_emits_incremental_output` in
  `crates/mv-cli/tests/cli_trtllm.rs` that runs the deliverable command,
  captures stdout, and asserts the output contains a non-trivial token
  count and ends with a single trailing newline.
- [X] T020 [US1] Add `#[ignore]` live-proxy integration test
  `trtllm_stream_inherits_just_load_hint_on_502` ensuring the streaming
  path surfaces the US2 error when the model is not loaded (proves the
  classifier is shared).

### Implementation for User Story 1

- [X] T021 [US1] In `crates/mv-cli/src/main.rs`, add `#[arg(long)] stream:
  bool` to `PromptArgs` (the clap struct for the prompt subcommand) per
  data-model.md "CLI types" section.
- [X] T022 [US1] In `run_prompt()` (or the equivalent dispatch site that
  currently branches to `call_trtllm`), branch on
  `args.stream && entry.provider == "trtllm"` to invoke a new
  `stream_trtllm()` function; if `args.stream` is set for any other
  provider, return the
  `streaming is only supported for TRT-LLM models in this release`
  error and exit 1 (research R7).
- [X] T023 [US1] Implement `stream_trtllm()` in
  `crates/mv-cli/src/main.rs` per the research R1 implementation outline:
  - Run the existing TRT-LLM health check first.
  - Build `openai::CompletionsClient` exactly like `call_trtllm()` does
    (same `api_key("tensorrt_llm")`, same `base_url(endpoint)`).
  - Build the agent with `SYSTEM_PREAMBLE`, the existing
    `tool_server_handle(handle)`, and
    `.additional_params(mv_core::trtllm::stop::request_stop_value(entry))`
    (skip the `additional_params` call if it returns `None`).
  - Call `agent.stream_prompt(prompt).multi_turn(10).await`.
  - Iterate with `futures_util::StreamExt::next`, writing each
    `StreamedAssistantContent::Text` delta to a locked stdout handle and
    flushing after each write.
  - On the terminal `MultiTurnStreamItem` carrying the aggregate response,
    capture `Usage::from_rig(&response)` for telemetry (recorded in T038).
  - Emit a single trailing `\n` after the stream terminates cleanly.
  - Preserve the existing MCP cleanup discipline: capture the stream result
    without `?`, call `shutdown_all`, then propagate any error.
- [X] T024 [US1] Map stream-iteration errors through the same TRT-LLM
  classifier used by `call_trtllm()` so 502 → `ModelNotLoaded` (T015) and
  network drop → `BackendUnreachable` produce identical messages; ensure
  partial output already written to stdout is left intact (no
  reset/clear) per contract "Stream interrupted mid-response".
- [X] T047 [US1] Implement the `--stream` + `--json` interaction (FR-011)
  in `crates/mv-cli/src/main.rs`: when both flags are set, write the
  warning `warning: --json overrides --stream; falling back to buffered
  JSON output` to stderr and route through the buffered JSON path.
  Cover with an offline integration test
  `json_overrides_stream_with_warning` in
  `crates/mv-cli/tests/cli_trtllm.rs` that asserts the warning appears
  on stderr and the stdout is valid JSON (use a non-trtllm model
  pointed at an unreachable endpoint — assertion is that the warning
  fires *before* the request, so the exit code / final body don't
  matter, only the warning + buffered code path do).

**Checkpoint**: US1 + US2 form the shippable MVP. The headline deliverable
command from the spec produces visible streaming output.

---

## Phase 5: User Story 3 — Token Usage in Telemetry (Priority: P2)

**Goal**: Every TRT-LLM call (buffered + streaming) attaches
`gen_ai.usage.input_tokens` and `gen_ai.usage.output_tokens` to the
`llm_completion` span when the proxy supplies them.

**Independent Test**: Run a TRT-LLM prompt with `--otlp`, export to a
local collector or stdout exporter, and assert the span carries both
attributes with non-zero `u64` values matching the proxy response.

**Depends on**: T005 (`Usage` type), T023 (streaming path exists so it
can be instrumented).

### Tests for User Story 3

- [X] T025 [P] [US3] Unit test in `crates/mv-core/src/trtllm/usage.rs`
  asserting `Usage::from_rig` correctly extracts counts from a stub
  implementing `GetTokenUsage` (use a small test double type defined in
  the same file under `#[cfg(test)]`).
- [X] T026 [US3] Extend the existing `cli_trtllm.rs` live test (or add
  `#[ignore]` test `trtllm_buffered_records_token_usage_attrs`) that runs
  a prompt with the `tracing_subscriber::fmt` test exporter attached and
  asserts the captured span carries both attribute keys with non-zero
  values. (Use `tracing-subscriber` test helpers; see existing telemetry
  tests in sprint 002 for the pattern.)

### Implementation for User Story 3

- [X] T027 [US3] In `crates/mv-cli/src/main.rs`, change `call_trtllm()`
  from the current `agent.prompt(prompt)` shape to a path that retains
  the `CompletionResponse` (per research R5 — e.g.
  `agent.completion(prompt, vec![]).await?.send().await?`), extract the
  assistant text from the response, and additionally compute
  `mv_core::trtllm::usage::Usage::from_rig(&response)`.
- [X] T028 [US3] Extend the `#[tracing::instrument(name = "llm_completion",
  ...)]` attribute on `call_trtllm()` and `stream_trtllm()` to declare
  `gen_ai.usage.input_tokens = tracing::field::Empty` and
  `gen_ai.usage.output_tokens = tracing::field::Empty` (research R5 — keep
  them empty when absent rather than zero).
- [X] T029 [US3] After the call (and after the terminal stream item),
  conditionally `tracing::Span::current().record(...)` both fields when
  `Usage::from_rig` returned `Some`.
- [X] T030 [US3] Wire the streaming path token extraction captured in T023
  into the same `Span::current().record(...)` calls so the attributes
  appear on streaming spans too.

**Checkpoint**: Telemetry parity across buffered and streaming; offline
tests still green; live test confirms attributes on exported spans.

---

## Phase 6: User Story 4 — Tool Calling Through TRT-LLM (Priority: P2)

**Goal**: Agent prompts that require built-in tools (e.g. `file_list`)
complete end-to-end via the TRT-LLM provider — model emits tool call,
CLI runs the tool, result returns to the model, final answer references
tool output.

**Independent Test**: Run an `#[ignore]` integration test that points at
the proxy with a tool-capable model, asks "list the files in the current
directory", and asserts the final answer mentions at least one file name
from the test working directory.

**Depends on**: T023 (the streaming path already drives the tool loop via
`multi_turn`), T015 (shared 502 mapping).

### Tests for User Story 4

- [X] T031 [P] [US4] Add `#[ignore]` integration test
  `trtllm_buffered_tool_call_round_trip` in
  `crates/mv-cli/tests/cli_trtllm.rs` that runs the buffered path with a
  prompt requesting a directory listing and asserts the response includes
  a known file name from a `tempfile::TempDir` scratch directory.
- [X] T032 [P] [US4] Add `#[ignore]` integration test
  `trtllm_streaming_tool_call_round_trip` for the same scenario via
  `--stream`.

### Implementation for User Story 4

- [X] T033 [US4] Confirm the existing `tool_server_handle(handle)` and
  `default_max_turns(10)` (buffered) / `.multi_turn(10)` (streaming)
  attachments built in T023 + T027 are sufficient. No new agent-loop code
  expected; this story is primarily verification, but any wiring gaps
  found by T031 / T032 are fixed here in `crates/mv-cli/src/main.rs`.

**Checkpoint**: Tool-calling parity with sprint 003 / 004 confirmed for
TRT-LLM in both response modes.

---

## Phase 7: User Story 5 — Stop Sequences Prevent Runaway Generation (Priority: P2)

**Goal**: Every model in the TRT-LLM registry terminates cleanly on
configured stop sequences; per-model overrides are forwarded to the proxy.

**Independent Test**: For each model in `models.yaml` with
`provider: trtllm`, run a smoke prompt known to historically trigger
runaway output and assert the response does not contain the next-turn
role marker tokens (e.g. `<|eot_id|>`, `<|im_start|>user`).

**Depends on**: T003, T006, T008 (foundational stop-sequence plumbing) and
T023 / T027 (so the streaming and buffered paths both pass
`additional_params`).

### Tests for User Story 5

- [X] T034 [P] [US5] Add offline unit test in
  `crates/mv-core/src/trtllm/stop.rs` asserting `request_stop_value` for
  an entry with no `stop_sequences` yields a JSON object whose `"stop"`
  array equals `default_stop_sequences()`.
- [X] T035 [US5] Add `#[ignore]` live-proxy parameterized integration
  test `trtllm_registry_models_terminate_cleanly` that loops every
  `models.yaml` entry with `provider: trtllm`, sends the fixed smoke
  prompt `"Say hi and then stop."`, and asserts the response does not
  contain any of the provider-default stop tokens (`</s>`, `<|im_end|>`,
  `<|eot_id|>`) as literal substrings.

### Implementation for User Story 5

- [X] T036 [US5] In `call_trtllm()` (T027) ensure the agent builder calls
  `.additional_params(mv_core::trtllm::stop::request_stop_value(entry))`
  before `.build()` (skipping the call only if the helper returns `None`).
- [X] T037 [US5] Update `models.yaml` to add a `stop_sequences` entry on
  the existing `llama-fp8` model (per data-model.md "With per-model
  overrides" example) so the registry exercises both the override and
  default code paths.

**Checkpoint**: Registry smoke test green; no runaway generations on the
shipped TRT-LLM models.

---

## Phase 8: User Story 6 — Workflow Step Uses TRT-LLM End-to-End (Priority: P3)

**Goal**: A workflow YAML with a prompt step bound to a TRT-LLM model
runs to completion; its output flows into downstream steps; the
`ModelNotLoaded` error surfaces cleanly from the workflow path on 502.

**Independent Test**: Per quickstart.md step 7 — author a 2-step workflow
that prompts a TRT-LLM model and references the result in a second step,
run it via `mv-cli workflow run`, and assert the downstream step sees the
captured value.

**Depends on**: T015 (502 mapping), T027 (buffered path with telemetry
extraction — workflow uses the buffered path; `--stream` is rejected on
`workflow run` per R8).

### Tests for User Story 6

- [X] T038 [P] [US6] Add `#[ignore]` integration test
  `workflow_with_trtllm_prompt_step_runs_end_to_end` in
  `crates/mv-cli/tests/cli_trtllm.rs` that writes a minimal 2-step
  workflow YAML to a `TempDir`, runs `mv-cli workflow run` against it,
  and asserts the process exits 0 with the second step's output non-empty.
- [X] T039 [P] [US6] Add `#[ignore]` integration test
  `workflow_with_unloaded_trtllm_model_emits_just_load_hint` that runs
  the same workflow while no model is loaded and asserts stderr contains
  the `Run: just load <model>` hint with exit code 1.

### Implementation for User Story 6

- [X] T040 [US6] Confirm `RigPromptExecutor::execute_prompt` (the workflow
  engine's prompt path in `crates/mv-core/src/workflow/engine.rs`) routes
  TRT-LLM provider entries through the same path that received T015's 502
  classifier and T027's usage extraction. If not (e.g. the workflow engine
  builds its own client), refactor the shared code into a small reusable
  function in `crates/mv-cli/src/main.rs` or move it to `mv-core` so both
  paths use it. This is the only task likely to require code in
  `mv-core/src/workflow/`.

**Checkpoint**: All six user stories independently testable; workflow
parity with sprint 004 behavior preserved.

---

## Phase 9: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, roadmap, and final regression validation.

- [X] T041 [P] Update `docs/11-trt-llm-integration.md` with a "Sprint 007:
  Streaming & Hardening" subsection summarising the `--stream` flag, the
  `just load` hint, telemetry attributes, and stop-sequence configuration.
- [ ] T042 [P] Update `docs/09-roadmap.md` to mark Phase 4.5.1 tasks
  complete (checkbox flip on the 7 task bullets) **only after** merge —
  this is the post-merge cleanup task tracked alongside the sprint-ship
  workflow.
- [X] T043 [P] Update `README.md` to mention `--stream` under the
  "Prompt (default command)" examples and the `stop_sequences` field
  under "Model Configuration".
- [X] T044 Run `just ci` (fmt + clippy `-D warnings` + test) and confirm
  the full suite passes with at least the pre-sprint 123 tests plus the
  new offline tests added in Phases 2–8.
- [X] T048 Run `just ci` three consecutive times back-to-back and confirm
  zero flakes — required by SC-007.
- [ ] T045 Run `just test-trtllm` (T002) against a healthy proxy with
  `llama-fp8` loaded, then again with no model loaded, to confirm both
  the success path (US1, US3, US4, US5, US6) and the 502 path (US2,
  US6) report correctly.
- [ ] T046 Run the deliverable command exactly as quoted in the spec
  (`cargo run -p mv-cli -- -m llama-fp8 --stream "Explain Rust ownership"`)
  and visually confirm SC-001 (first token visible within 1 s, subsequent
  chunks streaming).

---

## Dependencies

```text
Phase 1 (Setup: T001–T002)
   │
   ▼
Phase 2 (Foundational: T003–T011)
   │
   ├─► Phase 3 (US2 — 502 hint: T012–T015)
   │      │
   │      ▼
   ├─► Phase 4 (US1 — streaming: T017–T024, T047) ── depends on US2's classifier
   │      │
   │      ▼
   ├─► Phase 5 (US3 — token telemetry: T025–T030) ── needs streaming path
   │      │
   │      ▼
   ├─► Phase 6 (US4 — tool calling: T031–T033) ── exercises agent loop built in US1/US3
   │      │
   │      ▼
   ├─► Phase 7 (US5 — stop sequences: T034–T037) ── needs agent builder paths from US1/US3
   │      │
   │      ▼
   └─► Phase 8 (US6 — workflow: T038–T040) ── needs US2 + US3 classifier and extractor
          │
          ▼
       Phase 9 (Polish: T041–T046, T048)
```

User Story 2 is sequenced first because the classifier it introduces is a
dependency for every other story. Stories US3–US6 then build on the
agent paths created by US1.

## Parallel Execution Examples

Within each phase, tasks marked `[P]` operate on different files and can
be implemented in parallel:

- **Phase 2**: T003, T004, T005, T006 all touch different files (`lib.rs`
  enum vs struct vs two new modules) and can be parallelized; T009, T010,
  T011 (unit tests in their respective modules) likewise.
- **Phase 3**: T012 and T013 (two offline unit tests in the same file) can
  be authored in parallel as long as they go into the same `mod tests`
  block; merge before T014.
- **Phase 4**: T017 and T018 (two offline integration tests in
  `cli_trtllm.rs`) are independent of T019 / T020 (live tests).
- **Phase 6**: T031 and T032 (buffered vs streaming tool-call tests) are
  independent.
- **Phase 8**: T038 and T039 (success vs failure workflow tests) are
  independent.
- **Phase 9**: T041, T042, T043 touch three different docs and are fully
  parallel.

## Implementation Strategy

**MVP scope (minimum shippable increment)**: Phases 1, 2, 3, 4 → User
Stories 1 + 2. This delivers the headline `--stream` deliverable plus
the actionable 502 error — the two P1 stories. The remaining P2/P3
stories (telemetry, tool calling, stop sequences, workflows) are
additive and can each ship as follow-on increments without breaking the
MVP.

**Incremental delivery order**:

1. Phases 1–4: ship MVP (US1 + US2) → `cargo test` green, live demo of
   streaming + load hint.
2. Phase 5: telemetry → operators get cost/throughput visibility.
3. Phase 6: tool calling integration confirmed.
4. Phase 7: stop sequences applied across registry.
5. Phase 8: workflow end-to-end.
6. Phase 9: docs / regression / spec-deliverable command verification →
   sprint-ship.

## Independent Test Criteria (per story, restated)

- **US1**: Streaming command shows incremental output and matches buffered
  text. (T019)
- **US2**: 502 produces `Run: just load llama-fp8` on stderr, exit 1. (T014)
- **US3**: Span carries `gen_ai.usage.{input,output}_tokens` with non-zero
  `u64`. (T026)
- **US4**: Tool-call prompt completes and references real file names from
  test scratch directory. (T031, T032)
- **US5**: Smoke prompt against every registry trtllm model terminates
  cleanly (no stop-token leakage). (T035)
- **US6**: 2-step workflow with a trtllm prompt step completes and
  downstream step sees the value. (T038); 502 path surfaces hint. (T039)

## Format Validation

All 47 tasks above (T001–T015, T017–T048; T016 was removed as redundant)
follow the strict checklist format:

- Begin with `- [ ]`
- Carry a `T###` ID (T016 intentionally absent)
- Include `[P]` only when parallelizable
- Include `[US#]` only inside US phases (Phases 3–8)
- Include exact file paths in descriptions
