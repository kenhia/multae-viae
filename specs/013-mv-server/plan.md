# Implementation Plan: mv-server — REST Controller Daemon

**Branch**: `013-mv-server` | **Spec**: [spec.md](spec.md)  
**Research**: docs/09-roadmap.md Phase 6.3 (restructured 2026-06-12: REST
only, monitoring delegated to klams), fable notes
([02-findings.md](../../docs/fable/02-findings.md) F22 — MCP lifecycle "a
throwaway for a daemon"; F23 — fresh `reqwest::Client` + blocking `std::fs`;
[03-roadmap-recommendations.md](../../docs/fable/03-roadmap-recommendations.md)
§Phase 6 pre-work — sink-to-mv-core, `MvError::code()`, connection manager,
closed `enum AnyAgent` "not `Box<dyn>`"), and the 011 lesson that `mv-cli`
has no lib target so nothing in the binary crate is importable by tests or
other crates.

## Technical Context

**Language**: Rust edition 2024, workspace grows to three crates
(`mv-core` lib, `mv-cli` bin, `mv-server` lib+bin)  
**Key deps (existing)**: rig-core 0.35 (already in `mv-core` — the sink
moves code, not dependencies), rmcp, tokio, serde_json, minijinja,
opentelemetry/tracing  
**New deps**: `axum` (the API server; Tokio-native, tower ecosystem, the
de-facto standard — justified as Phase 6.3's core deliverable), `tower` +
`http-body-util` (dev-deps for in-process `oneshot` router tests), `croner`
(cron-expression parsing for the scheduler; small, no runtime of its own —
ticking stays hand-rolled Tokio). Tokio gains the `signal` feature.  
**Testing**: `just ci` hermetic — router tests in-process via
`tower::ServiceExt::oneshot` (no port binding), backend behavior via the
existing wiremock fake proxy, MCP lifecycle via the `fake_mcp_server`
fixture; live klams restart-recovery under `just test-klams` (`#[ignore]`)

## Constitution Check

- Spec entry: this plan + spec.md cover all changes, including the F22/F23
  remediations (Principle I). ✔
- TDD: failing hermetic test first per behavior; the runtime sink is pinned
  by the *existing* CLI e2e suite passing unmodified (Principle III). ✔
- YAGNI: no auth/TLS (localhost bind; Phase 7), no SSE streaming, no gRPC,
  no rate limiting, no local state DB (klams persists sessions), no
  scheduler catch-up/misfire semantics (skip-and-log only), no hot config
  reload. Each noted in the spec's Non-Goals, none built. ✔
- Defensive coding at boundaries only: request deserialization, the
  workflows-dir path check, schedules-file validation. Internal seams stay
  trusting. ✔
- Docs are part of done: WS5 in-sprint (`docs/12-mv-server.md`, docs/01,
  README, architecture SVG panel). ✔

## The sink surface (measured)

What moves from `mv-cli` to `mv-core`, and what stays:

- **Moves** — `providers.rs` (459 lines: `complete`,
  `complete_with_fallback`, `build_chain`, `complete_chain`, agent
  construction, `SYSTEM_PREAMBLE` attachment), `executors.rs` (77 lines:
  `RigPromptExecutor`, `HandleToolExecutor`), `memory.rs` (379 lines:
  `KlamsMemory`, `SessionMemory` — the trait they implement is already in
  `mv-core/src/memory.rs`). All of it already speaks only `mv_core` types
  plus rig, and `mv-core` already depends on `rig-core` (its tools are Rig
  `Tool`s) — this is a code move, not a dependency change.
- **Stays in `mv-cli`** — clap (`cli.rs`, `commands/`), stdout/stderr
  formatting, `--json` envelopes, `stream_trtllm` (writes to a terminal;
  the server has no streaming this sprint), telemetry *initialization*
  (each binary wires its own subscriber/exporter; span emission lives with
  the moved code).
- **Risk check**: the only rig-free zone in `mv-core` is the workflow
  engine module, which keeps its trait seam — the sink does not touch
  `workflow/`, so the engine remains mock-tested and provider-free.

## Design decisions

1. **`mv-server` is a library with a thin binary** (FR-003). The 011 lesson
   was direct: binary-only crates can't be integration-tested. The lib
   exposes `build_router(state) -> axum::Router` and the state constructor;
   `main.rs` parses flags, wires telemetry, binds, and serves. Router tests
   run in-process with `oneshot` — no ports, no processes, hermetic.
2. **One error story** (FR-002, FR-005). `MvError::code()` lives next to
   `Display` in `mv-core` with an exhaustive match (no `_` arm — adding a
   variant without a code is a compile error). `mv-server` owns a single
   `impl IntoResponse for MvError` doing code → HTTP status; handlers
   return `Result<Json<T>, MvError>` and never hand-build error bodies. The
   CLI's `--json` error envelope gains `code` additively.
3. **`AnyAgent` is a closed enum** (FR-006) over the three concrete agent
   types the three `call_*` paths build today. Constructing one factors the
   agent-building half out of `complete()` (build once, prompt many); the
   one-shot path becomes build + single prompt, so CLI behavior cannot
   drift. If a concrete agent type proves unnameable, the fallback is one
   thin newtype per provider inside the enum — still closed, still no
   `Box<dyn>`.
4. **Session state is RAM + klams, nothing else.** Server state holds
   `HashMap<String, Arc<Mutex<Session>>>` (name → held agent + meta); the
   mutex serializes turns (scenario US3-3 — a second concurrent turn waits;
   `409` only on create-collision). Persistence is the existing
   `SessionMemory<KlamsMemory>` recording turns as they happen; restart
   recovery is exactly the CLI's `--session NAME` recall path. No snapshot
   file, no sqlite.
5. **Scheduler = croner + a Tokio loop** (FR-007). Parse all expressions at
   boot (fail fast on bad config), then one task per schedule:
   sleep-until-next-fire, run the workflow through the same engine call the
   API handler uses, holding a per-schedule "running" flag — if the next
   fire arrives while set, log and skip. No queue, no catch-up, no jitter.
   Schedules file: `schedules: [{cron, workflow, inputs?, name?}]`.
6. **MCP manager** (FR-008) wraps today's `Vec<McpConnection>` in an owner:
   per-server task monitors health, reconnects with exponential backoff
   (cap + reset on success), and republishes the server's tools on
   reconnect. Tool dispatch through a dead server returns a tool-level
   error (the model can route around it), never a panic.
   `shutdown_all` becomes `join_all` over per-server shutdowns, each under
   a timeout. The CLI uses the manager in one-shot mode
   (connect → use → shutdown) so there is one lifecycle implementation; the
   stale "Drop shuts down" doc claim is corrected in WS5. Cross-server tool
   namespacing (the rest of F22) is **deferred** — collision detection logs
   a warning this sprint; renaming tools changes what models see and
   deserves its own decision.
7. **Shared `reqwest::Client`** (FR-009): a `OnceLock`-style shared default
   client in `mv-core` for the no-special-config callers (health, preflight,
   http_get); MCP keeps per-server clients only where per-server
   auth headers force them. `file_read`/`file_list` go `tokio::fs` (they're
   already `async fn` Rig tools — only the bodies change).
8. **Server config is flags, reusing existing files.** `--bind` (default
   `127.0.0.1:7077`), `--models`, `--mcp-servers`, `--workflows-dir`,
   `--schedules`, `--otlp` — the same `models.yaml`/`mcp-servers.yaml`
   discovery the CLI uses. No new server-config YAML until something needs
   structure flags can't give (YAGNI).
9. **Shutdown order** (FR-010): listener stops accepting → drain in-flight
   (bounded grace) → scheduler tasks aborted at a safe point → MCP manager
   shutdown (concurrent, per-server timeout) → telemetry flush → exit 0.
   Driven by `tokio::signal` + axum's `with_graceful_shutdown`.

## Workstream structure (maps to tasks.md phases)

- **WS1 — Runtime sink + error codes** (FR-001, FR-002, SC-006): move
  providers/executors/memory into `mv-core`, re-export or update `mv-cli`
  imports, `MvError::code()` + exhaustive test, `--json` gains `code`.
  Gate: existing CLI e2e suite passes unmodified.
- **WS2 — Daemon pre-work in core** (FR-008, FR-009, SC-007): shared
  client, `tokio::fs`, MCP connection manager with reconnect/backoff and
  timed concurrent shutdown; CLI switched to the manager's one-shot mode.
- **WS3 — Server skeleton + one-shot endpoints** (FR-003, FR-004, FR-005,
  FR-011, FR-012, SC-001, SC-002): crate, router, state, `/health`,
  `/v1/models`, `/v1/prompt`, `/v1/workflows/run`, error envelope +
  status mapping, path boundary, telemetry wiring.
- **WS4 — Sessions** (FR-006, SC-003): `AnyAgent`, session map + per-session
  mutex, the four session endpoints, klams persistence + live restart test.
- **WS5 — Scheduler, shutdown, operations, docs** (FR-007, FR-010, FR-013,
  SC-004, SC-005): croner loop + skip-overlap, graceful shutdown, example
  klams-monitor-events schedule, `just serve`, systemd unit sample,
  `docs/12-mv-server.md`, docs/01 + README + SVG panel.

## Ordering & risk

- WS1 first and alone: it is pure churn (file moves + import rewrites)
  best done while nothing else is in flight, and everything later imports
  what it relocates. WS2 next — WS3's `/health` and tool dispatch want the
  manager and shared client to exist. WS3 → WS4 → WS5 build upward.
- **Biggest risk: the sink silently changing CLI behavior.** Mitigation is
  SC-006 — the CLI e2e suite is the regression net and must pass
  *unmodified*; any test edit during WS1 is a red flag to stop and look.
- **Axum/rig version friction** (shared `http`/`hyper` graph): resolve at
  WS3 start; if the workspace needs a pin, record it in this plan.
- **Reconnect testing flakiness**: backoff tests use the `fake_mcp_server`
  fixture with injected short backoff constants — never wall-clock sleeps
  at real durations.
- **Scope valve**: if the sprint runs long, WS5's scheduler (FR-007) is
  the detachable piece — sessions and lifecycle are the daemon's core;
  schedules can ship as 013b without destabilizing anything beneath them.
