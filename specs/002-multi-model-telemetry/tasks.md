# Tasks: Multi-Model Routing & OpenTelemetry Observability

**Input**: Design documents from `/specs/002-multi-model-telemetry/`
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/cli.md, quickstart.md

**Tests**: Included — constitution mandates TDD (Principle III).

**Organization**: Tasks grouped by user story for independent implementation and testing.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Path Conventions

- **Workspace root**: `Cargo.toml`, `justfile`
- **Crates**: `crates/mv-core/`, `crates/mv-cli/`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add new dependencies to Cargo.toml files

- [x] T001 Add `serde_yml` dependency to `crates/mv-core/Cargo.toml`
- [x] T002 [P] Add OpenTelemetry dependencies to `crates/mv-cli/Cargo.toml`: opentelemetry 0.31, opentelemetry-sdk 0.31, opentelemetry-otlp 0.31, tracing-opentelemetry 0.32
- [x] T003 [P] Create example `models.yaml` in project root with qwen3:4b (default) and qwen3:8b entries per data-model.md

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core types and error variants that all user stories depend on

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

- [x] T004 Define `Locality` enum (Local, Cloud) with serde Deserialize and default inference from provider in `crates/mv-core/src/lib.rs`
- [x] T005 [P] Define `ModelEntry` struct with fields (id, provider, locality, api_key_env, endpoint, default) with serde Deserialize in `crates/mv-core/src/lib.rs`
- [x] T006 [P] Add new `MvError` variants: `ConfigParseError { path, details }`, `ModelNotInRegistry { model, available }`, `ApiKeyMissing { provider, env_var }` in `crates/mv-core/src/lib.rs`
- [x] T007 Define `ModelRegistry` struct with `models: Vec<ModelEntry>` and `default_model: Option<String>` in `crates/mv-core/src/lib.rs`
- [x] T008 [P] Write unit tests for new `MvError` variant display messages in `crates/mv-core/src/lib.rs`
- [x] T009 [P] Write unit test for `Locality` default inference (ollama→local, openai→cloud) in `crates/mv-core/src/lib.rs`

**Checkpoint**: Foundation ready — `cargo test -p mv-core` passes with all new type and error tests green

---

## Phase 3: User Story 1 — Configure and Select Models from a Registry (Priority: P1) 🎯 MVP

**Goal**: CLI loads models from a YAML config file, resolves `--model` against the registry, falls back to built-in defaults when no config exists

**Independent Test**: `cargo run -p mv-cli -- --model qwen3:8b "Hello"` with a `models.yaml` present uses the correct model; without `models.yaml` the CLI falls back to defaults

### Tests for User Story 1 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T010 [US1] Write unit test for `ModelRegistry::load()` — parse valid YAML, verify model count and fields in `crates/mv-core/src/lib.rs`
- [x] T011 [P] [US1] Write unit test for `ModelRegistry::load()` — malformed YAML returns `ConfigParseError` in `crates/mv-core/src/lib.rs`
- [x] T012 [P] [US1] Write unit test for `ModelRegistry::get()` — found and not-found cases in `crates/mv-core/src/lib.rs`
- [x] T013 [P] [US1] Write unit test for `ModelRegistry::default()` — explicit default vs first-entry fallback in `crates/mv-core/src/lib.rs`
- [x] T014 [P] [US1] Write unit test for `ModelRegistry::built_in()` — returns hardcoded qwen3:4b default in `crates/mv-core/src/lib.rs`
- [x] T015 [P] [US1] Write integration test for `--config` flag acceptance in `crates/mv-cli/tests/cli_args.rs`
- [x] T016 [P] [US1] Write integration test for unknown model error message in `crates/mv-cli/tests/cli_args.rs`

### Implementation for User Story 1

- [x] T017 [US1] Implement `ModelRegistry::load(path)` — read file, parse YAML via serde_yml, validate entries, return Result in `crates/mv-core/src/lib.rs`
- [x] T018 [US1] Implement `ModelRegistry::get(id)`, `ModelRegistry::default()`, and `ModelRegistry::built_in()` methods in `crates/mv-core/src/lib.rs`
- [x] T019 [US1] Implement config file resolution logic: --config flag → ./models.yaml → built_in() fallback in `crates/mv-core/src/lib.rs`
- [x] T020 [US1] Add `--config` CLI flag to clap args struct in `crates/mv-cli/src/main.rs`
- [x] T021 [US1] Refactor `run()` to load registry, resolve model from `--model` against registry, and create provider client from resolved `ModelEntry` in `crates/mv-cli/src/main.rs`

**Checkpoint**: `cargo run -p mv-cli -- "Hello"` works with and without `models.yaml`. `--model qwen3:8b` selects the correct model. Unknown models produce clear errors. `cargo test --workspace` passes.

---

## Phase 4: User Story 2 — View Traces of Model Calls in Jaeger (Priority: P2)

**Goal**: When `--otlp` is enabled, CLI exports OpenTelemetry trace spans for model calls to an OTLP-compatible collector with GenAI semantic convention attributes

**Independent Test**: Run `mv-cli --otlp "Hello"` with Jaeger on localhost:4317, verify trace visible in Jaeger UI with model name, provider, and timing attributes

### Tests for User Story 2 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T022 [US2] Write integration test verifying `--otlp` flag is accepted by CLI in `crates/mv-cli/tests/cli_args.rs`
- [x] T023 [P] [US2] Write integration test verifying CLI works normally when `--otlp` is set but no collector is running (graceful degradation) in `crates/mv-cli/tests/cli_args.rs`

### Implementation for User Story 2

- [x] T024 [US2] Add `--otlp` CLI flag (optional endpoint value, default http://localhost:4317) to clap args struct in `crates/mv-cli/src/main.rs`
- [x] T025 [US2] Implement `init_telemetry()` function that conditionally adds OpenTelemetry layer: build OTLP SpanExporter, TracerProvider, and tracing-opentelemetry layer in `crates/mv-cli/src/main.rs`
- [x] T026 [US2] Refactor tracing subscriber setup to compose fmt layer + optional OTel layer using `tracing_subscriber::registry()` in `crates/mv-cli/src/main.rs`
- [x] T027 [US2] Add `#[tracing::instrument]` with GenAI semantic convention fields (gen_ai.system, gen_ai.request.model, prompt length, response length, mv.model.locality) to model call in `crates/mv-cli/src/main.rs`
- [x] T028 [US2] Implement OTel graceful degradation — catch exporter init errors, warn and proceed without OTel layer in `crates/mv-cli/src/main.rs`
- [x] T029 [US2] Add `opentelemetry::global::shutdown_tracer_provider()` call before process exit in `crates/mv-cli/src/main.rs`

**Checkpoint**: `cargo run -p mv-cli -- --otlp "Hello"` exports traces to Jaeger. Without `--otlp`, behavior unchanged. Unreachable collector does not block prompt. `cargo test --workspace` passes.

---

## Phase 5: User Story 3 — Route a Prompt to a Cloud Fallback Provider (Priority: P3)

**Goal**: CLI can route prompts to OpenAI-compatible cloud providers based on model registry configuration

**Independent Test**: With OPENAI_API_KEY set and gpt-4o-mini in `models.yaml`, `cargo run -p mv-cli -- --model gpt-4o-mini "Hello"` returns a response from the cloud provider

### Tests for User Story 3 ⚠️

> **NOTE: Write these tests FIRST, ensure they FAIL before implementation**

- [x] T030 [US3] Write unit test for `ApiKeyMissing` error when cloud model selected without API key set in `crates/mv-core/src/lib.rs`
- [x] T031 [P] [US3] Write integration test verifying missing API key produces clear error message in `crates/mv-cli/tests/cli_args.rs`

### Implementation for User Story 3

- [x] T032 [US3] Implement provider client factory — match on `ModelEntry.provider` to create either Ollama or OpenAI Rig client in `crates/mv-cli/src/main.rs`
- [x] T033 [US3] Implement API key resolution — read env var from `ModelEntry.api_key_env`, return `ApiKeyMissing` error if not set in `crates/mv-cli/src/main.rs`
- [x] T034 [US3] Add cloud model entry (gpt-4o-mini) to example `models.yaml` (commented out with instructions)

**Checkpoint**: Cloud model routing works when API key is set. Missing API key produces clear error. Trace spans include provider type. `cargo test --workspace` passes.

---

## Phase 6: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, quality gates, and quickstart validation

- [x] T035 [P] Update `README.md` with new CLI flags (--config, --otlp), models.yaml setup, and Jaeger instructions
- [x] T036 [P] Run `cargo fmt --all` and fix any formatting issues across all crates
- [x] T037 Run `cargo clippy --all-targets --all-features -- -D warnings` and fix all warnings
- [x] T038 Run `just ci` and confirm zero warnings, zero failures
- [x] T039 Run quickstart.md validation — follow quickstart.md steps and verify all commands work
- [x] T040 Verify all acceptance scenarios from spec.md: registry load, model selection, unknown model error, no-config fallback, OTLP traces in Jaeger, graceful degradation, cloud routing

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **User Stories (Phase 3+)**: All depend on Foundational phase completion
  - US1 (P1) should complete first — US2 and US3 build on registry code
  - US2 (P2) can start after US1 (adds OTel to the model call flow from US1)
  - US3 (P3) can start after US1 (adds cloud provider to the client factory from US1)
  - US2 and US3 are independent of each other
- **Polish (Phase 6)**: Depends on all user stories being complete

### User Story Dependencies

- **User Story 1 (P1)**: Can start after Foundational (Phase 2) — MVP, no dependencies on other stories
- **User Story 2 (P2)**: Depends on US1 (instruments the model call flow implemented in US1)
- **User Story 3 (P3)**: Depends on US1 (extends the provider factory implemented in US1); independent of US2

### Within Each User Story

- Tests MUST be written and FAIL before implementation
- Core types/registry before CLI integration
- Error handling after happy path
- Story complete before moving to next priority

### Parallel Opportunities

- T001, T002, T003 can all run in parallel (Phase 1 — different files)
- T005, T006, T008, T009 can run in parallel within Phase 2 (independent types/tests)
- T010–T016 can run in parallel within US1 tests (independent test functions)
- US2 and US3 can run in parallel after US1 is complete (independent features)
- T035, T036 can run in parallel in Phase 6

---

## Parallel Example: Phase 1 Setup

```text
T001 ─┐
T002 ─┤──► Phase 2
T003 ─┘
```

## Parallel Example: User Stories after US1

```text
US1 complete ──► US2 (OTel) ─┐
               └► US3 (Cloud) ─┤──► Phase 6 (Polish)
```

---

## Implementation Strategy

1. **MVP first**: Phase 1 → Phase 2 → Phase 3 (US1) delivers a working multi-model CLI with config-driven model selection
2. **Incremental delivery**: Each phase is a testable increment
3. **TDD throughout**: Red → Green → Refactor per constitution Principle III
4. **Quality gates**: `just ci` after every phase to catch regressions
5. **Backward compatible**: No `models.yaml` → same behavior as sprint 001
