# Tasks: mv-server — REST Controller Daemon

**Input**: Design documents from `/specs/013-mv-server/`  
**Prerequisites**: plan.md, spec.md

**Tests**: TDD per Principle III — failing hermetic test first. Router
behavior via `tower::ServiceExt::oneshot` (in-process, no ports); backend
behavior via the wiremock fake proxy; MCP lifecycle via the
`fake_mcp_server` fixture; klams restart recovery live under
`just test-klams` (`#[ignore]`).

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (FR reference)
```

---

## Phase 1: WS1 — Runtime sink + error codes

- [X] T001 [WS1] Move `crates/mv-cli/src/providers.rs` (minus
  `stream_trtllm` and stdout printing) into `mv-core` as
  `crates/mv-core/src/runtime.rs` (or `runtime/` module): `complete`,
  `complete_with_fallback`, `build_chain`, `complete_chain`, agent
  construction. `mv-cli` imports from `mv_core::runtime`; `stream_trtllm`
  stays in the binary. No behavior change (FR-001)
- [X] T002 [WS1] Move `crates/mv-cli/src/executors.rs`
  (`RigPromptExecutor`, `HandleToolExecutor`) and
  `crates/mv-cli/src/memory.rs` (`KlamsMemory`, `SessionMemory`) into
  `mv-core`, colocated with the traits they implement; unit tests move with
  them (FR-001)
- [X] T003 [WS1] Gate: full existing CLI e2e suite passes **unmodified**
  (`just ci`); any required test edit is investigated before proceeding
  (SC-006)
- [X] T004 [P] [WS1] `MvError::code()` in `crates/mv-core/src/lib.rs`:
  exhaustive match (no `_` arm), SCREAMING_SNAKE codes for all 30 variants;
  pinned test asserts codes are unique, stable, and non-empty (FR-002)
- [X] T005 [P] [WS1] CLI `--json` error envelope gains additive `code`
  field; existing envelope assertions extended, not replaced (FR-002,
  SC-006)

**Checkpoint**: runtime importable from `mv-core`; CLI byte-for-byte
unchanged; every error has a code; `just ci` green.

---

## Phase 2: WS2 — Daemon pre-work in core

- [X] T006 [WS2] Shared `reqwest::Client` in `mv-core` (static `OnceLock`
  accessor with the standard timeout knobs); replace per-call builders at
  `trtllm/health.rs:30,93`, `preflight.rs:79`, `tools/http_get.rs:18`; MCP
  per-server clients kept only where per-server headers require them.
  Failing test first where injectable; grep-style review for the rest
  (FR-009, SC-007)
- [X] T007 [P] [WS2] `tools/file_read.rs` + `tools/file_list.rs` switch
  `std::fs` → `tokio::fs` (bodies only; they are already async Rig tools);
  existing tool tests keep passing (FR-009)
- [X] T008 [WS2] `McpManager` in `crates/mv-core/src/mcp/`: owns the
  connections, exposes the merged tool set, per-server state
  (healthy/reconnecting/dead). Failing test: tool dispatch through a dead
  server yields a tool-level error, not a panic (FR-008)
- [X] T009 [WS2] Reconnect with exponential backoff (cap + reset on
  success), republish tools on reconnect; test with `fake_mcp_server`
  killed and restarted, backoff constants injected short (FR-008)
- [X] T010 [WS2] Shutdown: concurrent `join_all` over per-server shutdowns,
  each under a timeout; test that a hanging server cannot extend shutdown
  past its budget (FR-008, SC-005)
- [X] T011 [WS2] CLI switches to `McpManager` one-shot mode
  (connect → use → shutdown); delete the superseded `shutdown_all` path;
  cross-server name collisions log a warning (namespacing deferred — note
  in docs) (FR-008)

**Checkpoint**: one MCP lifecycle implementation, daemon-safe; no fresh
clients in steady-state paths; `just ci` green.

---

## Phase 3: WS3 — Server skeleton + one-shot endpoints

- [X] T012 [WS3] New workspace member `crates/mv-server`: lib target with
  `build_router(AppState) -> Router` + thin `main.rs` (clap: `--bind`
  default `127.0.0.1:7077`, `--models`, `--mcp-servers`, `--workflows-dir`,
  `--schedules`, `--otlp`); add axum dep, tower/http-body-util dev-deps;
  resolve any http/hyper graph friction and record pins in plan.md
  (FR-003)
- [X] T013 [WS3] Failing oneshot test → `GET /health` (liveness + backend
  and MCP-manager summary) and `GET /v1/models` (registry: id, provider,
  locality, default) (FR-004)
- [X] T014 [WS3] `impl IntoResponse for MvError`: envelope
  `{"error": {code, message, hint?}}` + status mapping (404 not-found, 400
  input, 503 not-loaded/unreachable, 502 backend-error-response, 500
  rest); pinned matrix test (FR-005, SC-002)
- [X] T015 [WS3] Failing test → `POST /v1/prompt` {prompt, model?, …}
  through `mv_core::runtime` with fallback chain semantics; hermetic e2e
  via fake proxy: success body + not-loaded-502 → `503 MODEL_NOT_LOADED`
  with hint (FR-004, SC-001, SC-002)
- [X] T016 [WS3] Failing test → `POST /v1/workflows/run` {workflow, inputs}
  via the workflow engine with the real executors; outputs object returned;
  validation errors map through the envelope (FR-004, SC-001)
- [X] T017 [P] [WS3] Path boundary: workflow names resolve strictly inside
  `--workflows-dir`; failing test with `../` rejected `400` before any
  file I/O (FR-011)
- [X] T018 [P] [WS3] Telemetry: HTTP server spans wrapping the existing
  `gen_ai.*` spans; `--otlp` exporter wiring mirrors the CLI's (FR-012)

**Checkpoint**: one-shot API complete and hermetic; error taxonomy pinned
over HTTP; `just ci` green offline.

---

## Phase 4: WS4 — Sessions

- [X] T019 [WS4] `enum AnyAgent` in `mv_core::runtime` over the three
  concrete agent types; agent-building factored out of `complete()`
  (build once / prompt many); one-shot path re-expressed as build + single
  prompt — CLI e2e still unmodified (FR-006, SC-006)
- [X] T020 [WS4] Failing tests → session endpoints: `POST /v1/sessions`
  (201; duplicate name 409), `GET /v1/sessions`, `DELETE
  /v1/sessions/{name}` (drops agent, klams data retained), unknown-session
  turn 404 with create hint (FR-004, FR-006)
- [X] T021 [WS4] Failing test → `POST /v1/sessions/{name}/turns`: held
  agent carries context across turns (hermetic two-turn via fake proxy);
  per-session mutex serializes concurrent turns (FR-006, SC-003)
- [X] T022 [WS4] Wire `SessionMemory<KlamsMemory>` into the session
  lifecycle (record on turn, recall on create-by-existing-name) — same
  seam as CLI `--session` (FR-006)
- [X] T023 [WS4] Live `#[ignore]` test under `just test-klams`: record
  turns, drop state, recreate session by name, prior context recalled
  (SC-003)

**Checkpoint**: multi-turn sessions over HTTP with persistent memory;
restart recovery proven live; `just ci` green.

---

## Phase 5: WS5 — Scheduler, shutdown, operations, docs

- [X] T024 [WS5] Schedules file parsing + validation (croner): boot fails
  on invalid cron/missing workflow with an actionable error naming the
  entry; failing test first (FR-007)
- [X] T025 [WS5] Scheduler loop: per-schedule Tokio task,
  sleep-until-next-fire, run via the same engine call as the API handler;
  skip-and-log on overlap. Hermetic test: near-term schedule fires a
  trivial workflow; overlapping tick skipped (FR-007, SC-004)
- [X] T026 [P] [WS5] Example `examples/schedules.yaml` + workflow
  summarizing recent klams-monitor events via `event_search` (the roadmap
  "consumes klams-monitor events" deliverable, as configuration) (FR-007)
- [X] T027 [WS5] Graceful shutdown: `tokio::signal` +
  `with_graceful_shutdown`; order = stop accepting → drain (bounded) →
  scheduler stop → MCP manager shutdown → telemetry flush → exit 0. Test:
  SIGTERM with in-flight request completes it and exits 0 (FR-010, SC-005)
- [X] T028 [P] [WS5] Operations: `just serve` recipe; sample systemd unit
  in docs (FR-013)
- [ ] T029 [WS5] `docs/12-mv-server.md`: API reference (endpoints, error
  envelope + codes, status mapping), config flags, schedules format,
  service setup (FR-013)
- [ ] T030 [WS5] Truth pass: `docs/01-architecture-design.md` (third
  crate, runtime now in `mv-core`, MCP manager — fix the stale "Drop shuts
  down" claim), `docs/04-mcp-integration.md` lifecycle section, README,
  architecture SVG gains the mv-server panel; CLAUDE.md architecture notes
  updated (FR-013)
- [ ] T031 [WS5] Roadmap: flip Phase 6.3 checkboxes in
  `docs/09-roadmap.md`, record lessons learned (FR-013)

**Checkpoint**: daemon runs as a service, schedules fire, shuts down
clean, docs match the code; `just ci` green — sprint ready to ship.
