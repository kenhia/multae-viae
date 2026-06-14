# mv-server — REST Controller Daemon

`mv-server` runs multae-viae as a long-lived local service: an HTTP/REST API
over the same runtime the CLI uses, plus held-open conversation sessions and
cron-scheduled workflows. It is the Phase 6.3 deliverable — the controller
"runs as a system service; accepts requests via API, executes scheduled
workflows, consumes klams-monitor events."

It shares everything load-bearing with `mv-cli`: model routing and fallback,
the built-in + MCP tool set, the workflow engine, the error taxonomy, and
telemetry all live in `mv-core`. The server adds only the HTTP surface,
sessions, scheduling, and daemon lifecycle.

## Running

```bash
just serve --bind 127.0.0.1:7077 \
           --models models.yaml \
           --mcp-servers mcp-servers.yaml \
           --workflows-dir workflows/examples \
           --schedules workflows/examples/schedules.yaml \
           --otlp
```

All flags are optional:

| Flag | Default | Purpose |
|------|---------|---------|
| `--bind` | `127.0.0.1:7077` | Listen address. **Localhost only by default** — there is no API auth this release (Phase 7); binding to a public interface exposes an unauthenticated controller. |
| `--models` | `./models.yaml` → built-in | Model registry, as for the CLI. |
| `--mcp-servers` | discovery; omit for none | MCP server config; their tools merge into the agent tool set. |
| `--workflows-dir` | `.` | Directory `POST /v1/workflows/run` and the scheduler resolve workflow names against. Names cannot escape it. |
| `--schedules` | none | Cron→workflow schedule file (see [Schedules](#schedules)). |
| `--otlp [URL]` | — / `http://localhost:4318` | Export OpenTelemetry spans to an OTLP/HTTP collector. |

## Endpoints

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/health` | Liveness + a summary (model count, MCP servers configured/live). |
| `GET` | `/v1/models` | The registry: id, provider, locality, endpoint, default. |
| `POST` | `/v1/prompt` | One-shot completion through the fallback chain. |
| `POST` | `/v1/workflows/run` | Run a workflow by name; returns its outputs. |
| `POST` | `/v1/sessions` | Create a held-open session (`201`; `409` if the name exists). |
| `GET` | `/v1/sessions` | List active sessions. |
| `DELETE` | `/v1/sessions/{name}` | Drop a session (`204`). Persisted klams memory is retained. |
| `POST` | `/v1/sessions/{name}/turns` | Send a turn; the session carries context. |

### `POST /v1/prompt`

```bash
curl -s localhost:7077/v1/prompt \
  -H 'content-type: application/json' \
  -d '{"prompt": "What is multae-viae?", "model": "qwen3-coder:30b"}'
```

```json
{ "response": "...", "model_used": "qwen3-coder:30b" }
```

`model` is optional (defaults to the registry default). `temperature` and
`max_tokens` are optional sampling overrides. Routing, fallback, and the
TRT-LLM not-loaded preflight are identical to the CLI.

### `POST /v1/workflows/run`

```bash
curl -s localhost:7077/v1/workflows/run \
  -H 'content-type: application/json' \
  -d '{"workflow": "research.yaml", "inputs": {"topic": "rust async"}}'
```

```json
{ "workflow": "research", "outputs": { "summary": "..." } }
```

`workflow` is a file name resolved inside `--workflows-dir`; a name containing
`..`, an absolute path, or a drive prefix is rejected with `400`
`WORKFLOW_PATH_OUTSIDE_ROOT` **before any file is opened**. `inputs` is a JSON
object; non-string values are coerced to their compact JSON form (so `5`
becomes `"5"`), matching the CLI's string-valued workflow inputs.

### Sessions

A session holds an agent open across turns so the conversation accumulates
context, instead of rebuilding the agent per request.

```bash
curl -s localhost:7077/v1/sessions -d '{"name": "research"}' -H 'content-type: application/json'
curl -s localhost:7077/v1/sessions/research/turns \
  -H 'content-type: application/json' -d '{"prompt": "My name is Ken."}'
curl -s localhost:7077/v1/sessions/research/turns \
  -H 'content-type: application/json' -d '{"prompt": "What is my name?"}'   # → knows "Ken"
```

Turns within a session are **serialized** by a per-session lock — a second
concurrent turn to the same session waits its turn; distinct sessions run
concurrently. When a klams MCP server is connected, each turn is recorded and
the first turn of a (re)created session recalls prior context, so a session is
recoverable by name **after a server restart** (the same persistence the CLI's
`--session` flag uses). Deleting a session drops the in-memory agent but keeps
its klams history.

## Error envelope

Every error response is a single shape:

```json
{ "error": { "code": "MODEL_NOT_LOADED", "message": "...", "hint": "Run: just load <id>" } }
```

`code` is `MvError::code()` — the same stable, machine-readable discriminant the
CLI's `--json` errors carry, so a caller branches on the code rather than
parsing prose. `hint` is present for the variants that carry one. The HTTP
status is derived from the error:

| Status | When | Example codes |
|--------|------|---------------|
| `400` | bad caller input / unsupported request / path-boundary | `EMPTY_PROMPT`, `WORKFLOW_VALIDATION_ERROR`, `WORKFLOW_PATH_OUTSIDE_ROOT` |
| `404` | not found | `MODEL_NOT_FOUND`, `MODEL_NOT_IN_REGISTRY`, `SESSION_NOT_FOUND` |
| `409` | conflict | `SESSION_EXISTS` |
| `502` | backend reached but returned a server error | `BACKEND_ERROR_RESPONSE` |
| `503` | backend down / no usable model | `BACKEND_UNREACHABLE`, `MODEL_NOT_LOADED`, `ALL_MODELS_FAILED` |
| `500` | internal fault | `COMPLETION_FAILED`, `TOOL_CALL_FAILED`, `MEMORY_ERROR` |

## Schedules

`--schedules <file>` declares cron-driven workflow runs:

```yaml
schedules:
  - name: monitor-digest
    cron: "*/15 * * * *"          # 5-field, or 6-field with leading seconds
    workflow: monitor-summary.yaml # resolved inside --workflows-dir
    inputs:
      window: "1h"
```

Every entry is parsed and path-checked **at boot** — an invalid cron
expression, or a workflow that is missing or escapes the workflows directory,
is a startup error, not a silent no-op. Each schedule runs on its own timer
through the same engine path as `POST /v1/workflows/run`. **Overlap policy is
skip-and-log:** if a run is still in flight when the next tick arrives, the tick
is skipped — runs never stack. There is no catch-up: a tick missed while the
daemon was down is simply not run.

The shipped `workflows/examples/monitor-summary.yaml` consumes recent
klams-monitor events via the klams `event_search` tool and summarizes them —
the roadmap's "consumes klams-monitor events" deliverable, expressed as
configuration rather than new code.

## Daemon lifecycle

**MCP connections** are owned by a single manager
(`mv_core::mcp::manager::McpManager`) for the daemon's whole life: connections
open once and are health-checked; a server that drops can be reconnected with
exponential backoff while every other server keeps serving. A tool call into a
dead server returns a tool-level error the model can route around — it never
takes down the daemon. (The CLI uses the same manager in one-shot mode:
connect → use → shut down.)

**Graceful shutdown:** on `SIGINT` (Ctrl-C) or `SIGTERM` (what a service manager
sends), the server stops accepting connections, drains in-flight requests,
aborts the scheduler, shuts down MCP connections concurrently under a per-server
timeout (one hung server cannot wedge exit), flushes telemetry, and exits `0`.

## Run as a service

A sample systemd unit ships at
[`crates/mv-server/mv-server.service.example`](../crates/mv-server/mv-server.service.example).
`systemctl stop` sends `SIGTERM`, which triggers the graceful drain above;
`TimeoutStopSec` should be generous enough for in-flight requests to finish.

## Telemetry

Each request is wrapped in an HTTP server span; the `gen_ai.*` completion spans
the runtime emits nest under it. `--otlp [URL]` exports to an OTLP/HTTP
collector (default `http://localhost:4318`, view in Jaeger), identical to the
CLI, with `service.name = mv-server`.

## Non-goals (this release)

API auth/TLS, gRPC (Phase 7 adds MCP as the second protocol), SSE/streaming
responses, rate limiting, multi-tenancy, and a local state database (klams is
the persistence layer). See [docs/09-roadmap.md](09-roadmap.md).
