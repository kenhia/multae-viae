# Contract v1.1: klams tool surface consumed by multae-viae

**Pinned**: 2026-06-12, against klams main (sprint 009, "Stability &
Attribution"). Supersedes v1.0
(`specs/010-klams-rag/contracts/klams-tool-surface.md`): the read surface is
unchanged; the **write tools become load-bearing** for sprint 011.
klams source of truth: `crates/klams-mcp/src/tools/*.rs`,
`crates/klams-types/src/memory.rs`.

Versioning rule (unchanged): **additive** klams changes need no coordination;
**breaking** changes to anything below require a coordinated m-v change.

## Transport & auth

- MCP **Streamable HTTP** at `http://kubs0:7777/mcp`, bearer token via
  m-v's `auth_token_env` (shipped sprint 010).
- Sprint 011 requires a **`Read|Write`-scoped** token (minted 2026-06-12).
  Admin tools remain out of scope and fail server-side on this token.

## Attribution model (decided 2026-06-12)

Every `register_author` call **inserts a new author row** (UUIDv7) — it is
not an upsert. m-v therefore registers **one author per memory-active run**;
all of that run's writes carry the returned `author_id`, and Ken's klams
tooling lists writes by author.

- Real m-v runs: `agent_name: "mv-cli"`, `model: <resolved model id>`,
  `session_title: <--session name>`, `client_app: "mv-cli"`,
  `client_version: <CARGO_PKG_VERSION>`.
- Live tests: `agent_name: "multae-viae"` (distinguishes test writes).
- Validation (server-side): non-empty `agent_name` (length-capped); `repo`,
  if sent, must be an absolute path; `extra` is size-capped.

## Read tools (load-bearing since v1.0 — unchanged)

### `memory_search` (Read)

`{ query, kinds?, tags?, top_k? (1..50, default 10) }` → ranked
`PublicMemory[]`. Order is authoritative; no score exposed.

### `PublicMemory` wire shape

Internally tagged on `kind` (flattened content):

- `knowledge`: `{ id, kind, text, source_path?, repo?, tags, author,
  created_at, updated_at }`
- `fact`: `{ id, kind, type, payload (JSON), tags, author, … }`
- `event`: `{ id, kind, category, payload (JSON), task_id?, tags, author, … }`

## Write tools (load-bearing since v1.1)

### `register_author` (Read scope)

```yaml
input:
  agent_name: string          # required, non-empty, length-capped
  model: string?              # the LLM identity behind the agent
  session_title: string?
  repo: string?               # must be absolute if present
  client_app: string?
  client_version: string?
  extra: JSON                 # optional, size-capped
output:
  author_id: uuid             # NEW row per call (UUIDv7) — not an upsert
  agent_name: string
  created_at: RFC3339
errors: INVALID_AGENT_NAME | INVALID_REPO_PATH | EXTRA_TOO_LARGE
```

### `memory_add` (Write)

Internally tagged on `kind` (snake_case), flattened beside `author_id`:

```yaml
input (knowledge):
  author_id: uuid             # required, must reference a registered author
  kind: "knowledge"
  text: string
  tags: [string]              # optional
  source_path: string?
  repo: string?
input (fact):
  author_id: uuid
  kind: "fact"
  fact_type: UserFact | TaskFact | EnvFact   # PascalCase
  payload: JSON
output: PublicMemory
errors: MAINTENANCE_WINDOW_ACTIVE | MISSING_AUTHOR_ID | UNKNOWN_AUTHOR_ID |
        EMBEDDING_UNAVAILABLE | SCHEMA_VALIDATION_FAILED
```

### `memory_append_event` (Write)

```yaml
input:
  author_id: uuid
  category: string            # free-form, trimmed, 1..=cap chars
  payload: JSON object        # MUST be an object (not array/scalar)
  task_id: uuid?
output: PublicMemory (event)
errors: MAINTENANCE_WINDOW_ACTIVE | MISSING_AUTHOR_ID | UNKNOWN_AUTHOR_ID |
        INVALID_CATEGORY | SCHEMA_VALIDATION_FAILED
```

m-v's turn-recording uses `category: "conversation"` with payload
`{ session, prompt, response, model_used }` (large fields truncated
client-side — events are records, not archives).

### `event_search` (Read)

```yaml
input:
  author_id: uuid | [uuid]    # optional
  category: string | [string] # optional
  since: RFC3339?             # optional
  until: RFC3339?
  payload_match: JSON object? # field-equality match into the payload
  limit: u32?
  order: asc | desc?
  cursor: string?
output:
  events: PublicMemory[]      # kind == "event"
  next_cursor: string?
```

m-v's session recall filters with `payload_match: { "session": <name> }`,
`order: desc`.

### `memory_delete` (Write)

`{ id }` → `{ id, deleted_at }`. Soft-delete, idempotent; admin tools can
restore. Used by live tests to clean up after themselves, and available to
the agent (klams's soft-delete + admin recovery is the designed safety net
for agent writes).

## Operational expectations

- `MAINTENANCE_WINDOW_ACTIVE`: klams's daily backup window rejects writes
  with a retryable error envelope — m-v treats it like any memory failure
  (warn and continue; never block a prompt).
- Error envelopes carry machine-readable codes (the strings above); m-v
  surfaces them in warnings verbatim.
- Embedding may be down while the rest of klams is up
  (`EMBEDDING_UNAVAILABLE` on knowledge adds) — same warn-and-continue.
- klams unreachable ⇒ logged and skipped (degraded mode, unchanged from v1.0).
