# Contract: klams tool surface consumed by multae-viae

**Pinned**: 2026-06-12, against klams main `f1ced5a` (sprint 009, "Stability &
Attribution"). klams source of truth: `crates/klams-mcp/src/tools/mod.rs`,
`crates/klams-types/src/memory.rs`.

This document records exactly what m-v depends on, so both projects know what
is load-bearing. **Additive** klams changes (new tools, new optional fields)
need no coordination. **Breaking** changes to anything below (renames,
removed/retyped fields, auth semantics) require a coordinated m-v change —
flag them in klams sprint planning.

## Transport & auth

- MCP **Streamable HTTP** at `http://kubs0:7777/mcp` (rmcp server ↔ rmcp
  client; m-v config via `mcp-servers.yaml` with `transport: http`).
- **Bearer token** required (`Authorization: Bearer <token>`), minted in klams
  `[[auth.tokens]]`. m-v needs **`Read` scope only** for sprint 010.
- m-v reads the token from the environment (`auth_token_env`), never from
  config files.

## Tools m-v calls (sprint 010)

### `memory_search` (scope: Read) — the load-bearing tool

Input (`MemorySearchArgs`):

| field | type | notes |
|---|---|---|
| `query` | string | required |
| `kinds` | [`fact`\|`knowledge`\|`event`] | optional filter |
| `tags` | [string] | optional filter |
| `top_k` | u32, 1..50 | default 10; m-v examples use 3–5 |

Output: array of `PublicMemory` (hybrid FTS + vector, fused, ranked; order is
authoritative, no score field is exposed).

### `PublicMemory` wire shape (fields m-v relies on)

Internally tagged on `kind` (flattened `content`):

```json
{
  "id": "uuid",
  "kind": "knowledge",
  "text": "…chunk content…",
  "source_path": "/abs/path/on/kubs0",   // optional
  "repo": "repo-name",                    // optional
  "tags": ["…"],
  "author": { … },
  "created_at": "RFC3339",
  "updated_at": "RFC3339"
}
```

- `kind: "fact"` carries `type` + `payload` (JSON) instead of `text`.
- `kind: "event"` carries `category` + `payload` (+ optional `task_id`).
- m-v templates/agents primarily consume `text`, `source_path`, `kind`,
  `tags`. `deleted_at`/`deleted_by_author_id` never appear on live rows.

## Tools available but NOT used in sprint 010

Listed so Phase 6 planning knows they exist; not load-bearing yet:

- `memory_related` (Read) — ANN neighbours of a knowledge id.
- `event_search` (Read) — pure-SQL event query (no embedder dependency).
- `register_author`, `memory_add`, `memory_append_event` (Write) — Phase 6
  persistent memory writes.
- Admin tools — never m-v's business.

## Operational expectations

- klams unreachable ⇒ m-v logs and skips the server (degraded mode, non-fatal).
- Content freshness is klams's concern (`klams-scanner` hourly over `~/src`,
  `~/obsidian` on kubs0); m-v does not trigger indexing in sprint 010.
- Known klams limitations m-v accepts for now: single knowledge collection,
  384-dim general embeddings, no code-aware chunking. Improvements land
  inside klams behind this contract without m-v changes.
