# Feature Specification: mv-server — REST Controller Daemon

**Feature Branch**: `013-mv-server`  
**Created**: 2026-06-13  
**Status**: Draft  
**Input**: Phase 6.3 from docs/09-roadmap.md — the controller becomes a
long-running service: an Axum REST API over the same runtime the CLI uses,
session/conversation management, scheduled workflow execution, and the
daemon pre-work the fable review flagged
([02-findings.md](../../docs/fable/02-findings.md) F22 — MCP
connect-per-invocation is "a throwaway for a daemon"; F23 — fresh
`reqwest::Client` per call, blocking `std::fs` in tool paths). The 2026-06-12
roadmap restructure already scoped it: REST only (no gRPC — "controller as
MCP server" is Phase 7's second protocol), monitoring consumed from the klams
ecosystem rather than built here.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - One-shot prompts over HTTP (Priority: P1)

A script (or another service) POSTs a prompt to a locally running `mv-server`
and gets the same answer, the same model routing, and the same tool access
the CLI would give — without spawning a process per request. Errors come back
machine-readable: a stable code plus the existing actionable message, so the
caller can branch on `MODEL_NOT_LOADED` instead of parsing prose.

**Acceptance Scenarios**:

1. **Given** a running server and a reachable backend (hermetic via the fake
   proxy), **When** a client POSTs `{"prompt": "...", "model": "..."}` to
   `/v1/prompt`, **Then** it receives `200` with the completion text and the
   model id that served it.
2. **Given** a TRT-LLM backend answering the not-loaded 502 (fake proxy),
   **When** a prompt runs, **Then** the response is a `503` JSON envelope
   with `code: "MODEL_NOT_LOADED"` and the existing `just load` hint —
   the same classifier the CLI uses, not a re-implementation.
3. **Given** an unknown model id, **When** a prompt is POSTed, **Then** the
   server answers `404` with `code: "MODEL_NOT_FOUND"` and the message
   listing valid ids.
4. **Given** the server was started with `--json`-equivalent defaults (it is
   an API — everything is JSON), **When** any error occurs, **Then** stdout
   logging and the HTTP body never disagree about what happened.

---

### User Story 2 - Workflow execution over HTTP (Priority: P1)

A caller runs a YAML workflow by name through the API, passing inputs as
JSON, and receives the workflow's outputs object — typed, per the 012 `Value`
context. Workflow files are resolved only inside the server's configured
workflows directory; the API cannot be used to execute arbitrary paths.

**Acceptance Scenarios**:

1. **Given** `--workflows-dir` containing `report.yaml`, **When** a client
   POSTs `{"workflow": "report.yaml", "inputs": {...}}` to
   `/v1/workflows/run`, **Then** it receives the outputs object that
   `mv-cli workflow run` would print.
2. **Given** a workflow name containing `../`, **When** it is POSTed,
   **Then** the server refuses with `400` and a path-boundary error code —
   the file is never opened.
3. **Given** a workflow that fails validation, **When** it is POSTed,
   **Then** the response carries the validation error's code and message
   (same taxonomy as the CLI).

---

### User Story 3 - Multi-turn sessions with persistent memory (Priority: P2)

A client creates a named session, sends turns, and the conversation context
carries forward — the agent is held open between requests instead of rebuilt
per call. Session transcripts persist through the sprint-011 memory seam
(klams), so a session survives a server restart: recreate it by name and
recall does what `mv-cli --session NAME` already does.

**Acceptance Scenarios**:

1. **Given** a created session, **When** two turns are sent ("My name is
   Ken" / "What is my name?"), **Then** the second reply demonstrates the
   first turn is in context.
2. **Given** a session with recorded turns and a restarted server (live
   klams, `#[ignore]`), **When** the session is recreated by the same name,
   **Then** prior context is recalled through the `MemoryStore` seam.
3. **Given** two concurrent turn requests to the same session, **When** one
   is in flight, **Then** the second waits or is refused with `409` — turns
   within a session are serialized, never interleaved.
4. **Given** a `DELETE` on a session, **Then** the held agent is dropped and
   its name is free to recreate; persisted memory in klams is retained.

---

### User Story 4 - Scheduled workflows (Priority: P2)

An operator declares schedules (cron expression → workflow + inputs) in a
config file. The daemon runs them unattended, with the same telemetry as
interactive runs. The shipped example consumes klams-monitor events via the
klams `event_search` tool — the roadmap's "consumes klams-monitor events"
deliverable, done as configuration, not new code.

**Acceptance Scenarios**:

1. **Given** a schedule with a near-term cron expression and a trivial
   workflow (hermetic), **When** the tick arrives, **Then** the workflow
   executes and the run is visible in logs/spans.
2. **Given** a scheduled workflow still running when its next tick arrives,
   **Then** the new tick is skipped (logged), not stacked.
3. **Given** a schedules file with an invalid cron expression, **When** the
   server starts, **Then** startup fails with an actionable error naming the
   bad entry — bad config is a boot error, not a silent no-op.

---

### User Story 5 - Daemon-grade lifecycle (Priority: P2)

`mv-server` runs as a system service. MCP connections are opened once and
kept healthy — a server that drops is reconnected with backoff instead of
poisoning every later tool call. On SIGTERM the daemon stops accepting work,
drains in-flight requests, shuts MCP connections down concurrently with a
timeout, and flushes telemetry.

**Acceptance Scenarios**:

1. **Given** a running daemon and an MCP server that dies (fake stdio server
   killed), **When** the next tool call happens after reconnect backoff,
   **Then** it succeeds — and the failure window produced tool-level errors,
   not a daemon crash.
2. **Given** an in-flight prompt request, **When** SIGTERM arrives, **Then**
   the request completes (within the drain timeout) before the process
   exits 0.
3. **Given** an MCP server that hangs on shutdown, **When** the daemon
   stops, **Then** shutdown still completes within the per-server timeout —
   one bad server cannot wedge exit.

---

### Edge Cases

- Empty prompt over the API → `400` `EMPTY_PROMPT` (same `MvError`, new
  transport).
- Session turn for a name that was never created (or was evicted) → `404`
  with a hint to create it.
- `/v1/models` with a missing/unparseable `models.yaml` → boot error, same
  as the CLI today.
- Two sessions created with the same name → `409`; names are the identity
  (they map to klams session names).
- Scheduler tick during shutdown drain → skipped; the scheduler stops before
  the listener.

## Requirements

### Functional Requirements

- **FR-001 (runtime sink)**: The agent runtime moves from `mv-cli` to
  `mv-core`: `complete`, `complete_with_fallback`, `build_chain`,
  `complete_chain`, the `RigPromptExecutor`/`HandleToolExecutor` executors,
  and the `KlamsMemory`/`SessionMemory` memory impls. `mv-cli` keeps clap,
  output formatting, `--stream` (terminal printing), and telemetry wiring —
  it becomes a caller of `mv-core` like `mv-server`. No behavior change;
  existing tests keep passing (moved where they must).
- **FR-002 (error codes)**: `MvError::code(&self) -> &'static str` gives
  every variant a stable SCREAMING_SNAKE code (e.g. `MODEL_NOT_LOADED`,
  `BACKEND_UNREACHABLE`). A pinned exhaustive-match test forces new variants
  to declare a code. The CLI's `--json` error envelope gains the additive
  `code` field.
- **FR-003 (server crate)**: New workspace member `crates/mv-server` —
  a library (router/state construction, fully testable in-process via
  `tower::ServiceExt::oneshot`, per the 011 no-lib-target lesson) plus a
  thin `main.rs`. Axum, Tokio, binding `127.0.0.1:7077` by default
  (`--bind` to override). No auth in this sprint — localhost bind is the
  boundary; API auth is Phase 7 security hardening.
- **FR-004 (endpoints)**: `GET /health` (liveness + backend/MCP summary),
  `GET /v1/models` (registry, locality, default), `POST /v1/prompt`,
  `POST /v1/workflows/run`, `POST /v1/sessions`, `GET /v1/sessions`,
  `POST /v1/sessions/{name}/turns`, `DELETE /v1/sessions/{name}`.
- **FR-005 (error envelope)**: Every error response is
  `{"error": {"code", "message", "hint"?}}` derived from `MvError` — one
  `IntoResponse` impl owns the `MvError` → HTTP status mapping (404 for
  not-found shapes, 400 for input errors, 503 for not-loaded/unreachable,
  502 for backend error responses, 500 otherwise).
- **FR-006 (sessions)**: Held-open agents via a closed
  `enum AnyAgent { Ollama(..), OpenAi(..), TrtLlm(..) }` over the three
  concrete Rig agent types — not `Box<dyn>` (per the fable WS2 note).
  Sessions live in server state behind a per-session async mutex; turns
  serialize. Transcript persistence and recall reuse
  `SessionMemory<KlamsMemory>` unchanged — restart recovery is recall by
  session name, no new persistence layer.
- **FR-007 (scheduler)**: A `--schedules <file>` YAML maps cron expressions
  to `{workflow, inputs}`. In-process Tokio scheduler; overlap policy is
  skip-and-log; invalid entries fail startup. Shipped example schedule runs
  a workflow that summarizes recent klams-monitor events via `event_search`.
- **FR-008 (MCP connection manager, F22)**: A manager owns the long-lived
  connections: health-checked keep-alive, reconnect with exponential
  backoff, per-server restart isolation (one dead server degrades its tools,
  nothing else), and shutdown that is concurrent (`join_all`) with a
  per-server timeout. The CLI's connect→use→shutdown flow becomes the
  manager's one-shot mode; the stale "Drop shuts down" doc claim is fixed.
- **FR-009 (shared HTTP client + async fs, F23)**: One shared
  `reqwest::Client` (connection pooling) replaces the per-call
  `Client::builder()` at `trtllm/health.rs:30,93`, `tools/http_get.rs:18`,
  `preflight.rs:79`, and `mcp/client.rs` (per-server clients remain only
  where per-server headers require them). `tools/file_read.rs` /
  `tools/file_list.rs` switch `std::fs` → `tokio::fs` — no blocking syscalls
  on the daemon's runtime threads.
- **FR-010 (graceful shutdown)**: SIGTERM/SIGINT → stop accepting, drain
  in-flight requests (bounded), stop the scheduler, shut down MCP via the
  manager, flush telemetry, exit 0.
- **FR-011 (path boundary)**: Workflow names resolve strictly inside
  `--workflows-dir`; traversal attempts are rejected before any file I/O
  (defensive coding at the system boundary, per the constitution).
- **FR-012 (telemetry parity)**: Server requests produce HTTP server spans
  wrapping the same `gen_ai.*` spans the CLI emits; `--otlp [URL]` works
  identically.
- **FR-013 (operations & docs)**: `just serve` recipe; a sample systemd unit
  in docs; new `docs/12-mv-server.md` (API reference, config, service
  setup); `docs/01-architecture-design.md` gains the third crate (and the
  architecture SVG gets an mv-server panel); README updated.

### Non-Goals (this sprint)

API auth/TLS (Phase 7), gRPC (dropped — Phase 7 adds MCP as the second
protocol), SSE/streaming responses (streaming is TRT-LLM-only and
terminal-oriented today; revisit when a consumer exists), rate limiting,
multi-tenancy, a local state database (klams is the persistence layer), and
building any monitoring/file-watching (klams ecosystem owns it).

## Success Criteria

- **SC-001**: A hermetic e2e (server in-process + fake proxy) proves
  `/v1/prompt` returns the completion and `/v1/workflows/run` returns a
  multi-step workflow's outputs — `just ci` stays green offline.
- **SC-002**: The error matrix is pinned: not-loaded 502 → `503
  MODEL_NOT_LOADED` (with hint), unknown model → `404 MODEL_NOT_FOUND`,
  empty prompt → `400 EMPTY_PROMPT`, traversal → `400`. Every `MvError`
  variant has a code (exhaustive-match test).
- **SC-003**: Session continuity is proven hermetically (two turns, context
  carries) and restart recovery live against klams (`just test-klams`,
  `#[ignore]`).
- **SC-004**: A near-term schedule fires its workflow within one tick
  (hermetic, short-interval) and an overlapping tick is skipped and logged.
- **SC-005**: SIGTERM with one in-flight request exits 0 with the request
  served; a hanging MCP server cannot extend shutdown past its timeout.
- **SC-006**: `mv-cli` behavior is byte-for-byte unchanged after the runtime
  sink (existing CLI e2e suite passes unmodified, `--json` errors gain only
  the `code` field).
- **SC-007**: No per-request `reqwest::Client` construction remains in
  steady-state paths (the F23 sites are gone; verified by test where
  injectable, by review elsewhere).
