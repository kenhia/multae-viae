# Tasks: Consolidation & Hardening

**Input**: Design documents from `/specs/008-consolidation/` + `docs/fable/02-findings.md`
**Prerequisites**: plan.md, spec.md

**Tests**: TDD per Principle III — each fix lands with the hermetic test that was
missing. Live-backend tests stay `#[ignore]`d.

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (finding ID)
```

---

## Phase 1: WS1 — Correctness

- [X] T001 [WS1] UTF-8-safe `truncate_output` in `crates/mv-core/src/tools/mod.rs`
  — walk back to a char boundary before slicing; test with a multi-byte char
  straddling the limit (F1)
- [X] T002 [P] [WS1] `shell_exec`: kill timed-out children (`kill_on_drop` /
  explicit kill) in `crates/mv-core/src/tools/shell_exec.rs`; make timeout
  injectable for a fast test (F5)
- [X] T003 [P] [WS1] `shell_exec`: always report non-zero exit status; label
  `stdout:`/`stderr:` when both streams present; tests (F8)
- [X] T004 [P] [WS1] Truncate MCP tool output in `CleanedMcpTool::call`
  (`crates/mv-core/src/mcp/registry.rs`) to `MAX_TOOL_OUTPUT_CHARS`; test (F9)
- [X] T005 [P] [WS1] Validate `retry.max_attempts >= 1`
  (`crates/mv-core/src/workflow/validate.rs`, new `InvalidRetryConfig`); replace
  engine `unreachable!()` with an error; cap backoff delay at 30s; tests (F4)
- [X] T006 [P] [WS1] `MvError::MaxTurnsExceeded` variant in
  `crates/mv-core/src/lib.rs`; classify rig max-turn errors in
  `classify_rig_error` (`crates/mv-cli/src/main.rs`); tests (F7)
- [X] T007 [WS1] Real `HandleToolExecutor` over `ToolServerHandle::call_tool` in
  `crates/mv-cli/src/main.rs` replacing `NoopToolExecutor`; remove
  `on_error: skip` masking from `workflows/examples/tool-example.yaml`; test (F2)
- [X] T008 [WS1] Thread `temperature`/`max_tokens` from `RigPromptExecutor`
  through `call_ollama`/`call_openai`/`call_trtllm` via a `GenParams` struct
  (F3). *Note: the wire-level assertion that the provider request carries the
  params lands with the T025/T026 wiremock fixture — no hermetic observation
  point exists before it.*
- [X] T009 [WS1] `RigPromptExecutor`: unknown model → `ModelNotInRegistry`
  (no silent default fallback); hermetic CLI test (F6)
- [X] T010 [WS1] `--json` errors to stderr in `print_error`
  (`crates/mv-cli/src/main.rs`); update/extend CLI tests asserting the channel
  and shape (F10)

**Checkpoint**: every declared feature works or fails loudly; `just ci` green.

---

## Phase 2: WS2 — Phase-5 seam

- [ ] T011 [WS2] Create `crates/mv-core/src/providers.rs`: move `SYSTEM_PREAMBLE`,
  add `classify_backend_error` (typed-first, string fallback) and
  `MvError::is_fallback_eligible()`; re-point mv-cli; tests move/extend (F12, F20)
- [ ] T012 [WS2] `Provider` enum (serde) replacing stringly `ModelEntry.provider`;
  reject unknown providers at registry load with available list; collapse
  provider `match` arms into enum methods (F13)
- [ ] T013 [WS2] `ModelRegistry` validation: duplicate ids, multiple defaults;
  `ConfigNotFound` variant for missing file; `deny_unknown_fields` on
  `ModelEntry`; delete dead `BackendConfig` (F14, F24)
- [ ] T014 [WS2] Extract `async fn complete(entry, params, handle) ->
  Result<CompletionOutcome, MvError>` unifying both dispatch sites; generic
  `configure_agent`/`run_agent` helpers over `AgentBuilder<M>`; `stream_trtllm`
  shares preflight/builder/usage helpers with `call_trtllm` (F11)
- [ ] T015 [P] [WS2] `ModelEntry::effective_max_turns()` (default 10) replacing
  five hardcoded sites; `api_key_env()` resolver; hoist trtllm hint const (F21)
- [ ] T016 [P] [WS2] Use `Usage::from_rig()` + a `record_on(span)` helper for all
  usage recording (buffered + streaming) (F-cli-I9)
- [ ] T017 [WS2] Split `crates/mv-cli/src/main.rs` into `cli.rs`, `providers.rs`,
  `telemetry.rs`, `commands/{prompt,workflow}.rs`, `executors.rs` (F20)

**Checkpoint**: one dispatch seam; mv-server-bound logic lives in mv-core.

---

## Phase 3: WS3 — Engine prep

- [ ] T018 [WS3] Extract `execute_step`/`execute_steps` from the inline match;
  move transforms to `workflow/transform.rs` (single `KNOWN_TRANSFORMS` source);
  retry logic to `workflow/retry.rs` (F15, F-wf-I10)
- [ ] T019 [WS3] Encapsulate `ExecutionContext` (private fields; `get`/`insert`/
  `snapshot`); make engine default-model a required parameter (drop hardcoded
  `"qwen3:4b"`) (F15)
- [ ] T020 [WS3] Add `Send` bounds to `PromptExecutor`/`ToolExecutor` (F15)
- [ ] T021 [WS3] Replace naive `{{…}}` scanner with minijinja
  `undeclared_variables()`; validate `template_file` contents; recurse template
  rendering through nested tool-input values (F16, F-wf-I9)
- [ ] T022 [WS3] `DuplicateOutputName` error + `OutputShadowsInput` warning in
  validation (F17)
- [ ] T023 [WS3] `WorkflowStepFailed { step, source: Box<MvError> }`; retry only
  `is_retryable()` errors; configurable base delay; document side-effect
  re-execution (F18)
- [ ] T024 [WS3] `ToolPolicy` seam (default-allow) threaded through built-in tool
  construction (F19)

**Checkpoint**: branch/parallel/loop are additive changes, not rewrites.

---

## Phase 4: WS4 — Test infrastructure

- [ ] T025 [WS4] Add `wiremock` dev-dep to mv-cli; fixture module scripting
  `/health`, `/v1/models`, `/v1/chat/completions` (200, 502+Triton body,
  tool_calls round-trip, SSE stream, malformed usage) (F25)
- [ ] T026 [WS4] Hermetic happy-path tests: successful completion, multi-turn
  tool round-trip, streaming accumulation + trailing newline, 502→hint on both
  paths — against the fixture (F25)
- [ ] T027 [P] [WS4] Fake stdio MCP server dev-binary (2 tools: one colliding,
  one in `SEMANTIC_OVERLAPS`); tests for merge/precedence/skip/truncation/
  shutdown (F26)
- [ ] T028 [P] [WS4] Hermetically pin all CLI tests (explicit `--config` to
  unreachable endpoint or tempdir cwd); strengthen `code != 2`-only assertions;
  port-0-listener trick for unreachable tests; injectable HTTP/shell timeouts
  (suite ≤ 10s) (F27)
- [ ] T029 [WS4] End-to-end workflow test through the CLI against the fixture
  (prompt + tool + transform steps) (F28)

**Checkpoint**: SC-002 and SC-004 met.

---

## Phase 5: WS5 — Docs truth pass (polish)

- [ ] T030 [P] [WS5] README: quickstart model (`qwen3:8b`), flag matrix table
  (`--stream`×`--json`×`--no-tools`×provider), `--no-tools` in options, tool-step
  status, `just load` cross-repo note (F29, F34, G2/G3)
- [ ] T031 [P] [WS5] docs/01 refresh through sprint 008: real crate tree, tech
  stack corrections, retitle "As of Sprint 003" (F30)
- [ ] T032 [P] [WS5] docs/06: "(Phase 5/6 — not implemented)" tags on
  branch/parallel/loop/workflow/model-pref/`{{#if}}`/messages; fix top example;
  document the actual minijinja dialect (F31)
- [ ] T033 [P] [WS5] docs/05 span names + OTLP-HTTP/4318; docs/04 npm package
  name; docs/00 index rows for 10/11; docs/10 phase number (F32)
- [ ] T034 [P] [WS5] Spec hygiene: check 007 T042; refresh 007 quickstart
  streaming commands; create `specs/supplemental-spec.md` stub; models.yaml
  schema pointer (F33)
- [ ] T035 [WS5] Update `docs/09-roadmap.md` Phase 4.6 checkboxes + lessons
  learned (after merge)

**Checkpoint**: SC-005 met; sprint shippable.
