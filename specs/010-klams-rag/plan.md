# Implementation Plan: RAG via klams Memory Service

**Branch**: `010-klams-rag` | **Spec**: [spec.md](spec.md)  
**Research**: [docs/08-rag-integration.md](../../docs/08-rag-integration.md)
(original design sketch — this sprint replaces its krag/qdrant/ollama plan
with klams and rewrites the doc), klams assessment (2026-06-12, this
session), contract pin in
[contracts/klams-tool-surface.md](contracts/klams-tool-surface.md).

## Technical Context

**Language**: Rust edition 2024, workspace (`mv-core` lib, `mv-cli` bin)  
**Key deps**: rig-core 0.35, rmcp (client; klams's server is rmcp 1.7 — same
SDK both sides), reqwest (auth header injection), serde_yml, tokio  
**New deps**: none expected — bearer auth is a reqwest default-header on the
existing `StreamableHttpClientTransport::with_client` seam  
**Testing**: `just ci` hermetic; fake klams MCP server extends the 008
fake-MCP infrastructure; live kubs0 tests `#[ignore]`d behind `just test-klams`  

## Constitution Check

- Spec entry: this plan + spec.md cover all changes (Principle I). ✔
- TDD: every behavior lands with a hermetic test first (Principle III). ✔
- YAGNI: no new DSL surface, no rig `dynamic_context`, no write-path tools,
  no retry/caching layer over klams — retrieval is just an MCP tool reaching
  the existing merged toolset. The only new mechanism is bearer auth on HTTP
  transport, which klams hard-requires. ✔
- Docs are part of done: WS5 in-sprint, and the docs/08 rewrite is a major
  deliverable (the doc currently describes an architecture we decided not to
  build). ✔

## Decision record: klams over krag

Phase 5.5 originally planned a krag-backed RAG service (handoff drafted at
`krag/specs/planning/handoff-mv-rag-mcp.md`, now superseded for this role).
klams replaced it because every krag work item already exists in klams,
deployed: rmcp Streamable HTTP MCP server with 9 tools, scoped bearer auth,
hybrid (vector + FTS) retrieval with RRF/weighted fusion, push *and* scanner
ingestion, systemd-hardened deployment on kubs0 with backups and metrics, 77
integration tests. Strategically, klams's facts/events/knowledge model is
also the Phase 6 persistent-memory backend, so one boundary serves both
phases. Accepted trade-off: klams retrieves code less well than krag would
have (384-dim general embeddings, no tree-sitter chunking, single
collection) — fixable inside klams later without m-v changes. krag continues
as a standalone tool, decoupled from m-v.

## Design decisions

1. **Retrieval is a tool, not a step type.** klams tools merge into the agent
   toolset via the existing MCP registry; workflows reach them through
   ordinary `tool` steps. No `rag:` DSL affordance, no rig `dynamic_context`
   — the agentic loop and explicit tool steps cover both retrieval styles.
   Revisit only if prompt-engineering around `{{results}}` proves painful.
2. **Read-only this sprint.** No `register_author`/`memory_add` — writes are
   Phase 6 (persistent memory), where attribution and author lifecycle
   deserve their own design. The token m-v ships with is `Read`-scoped, so
   the boundary is enforced server-side too.
3. **Auth via env-var indirection** (`auth_token_env`), mirroring the
   `api_key_env` convention models.yaml already uses: config names the
   variable, never the secret. Injection point: build the `reqwest::Client`
   with a default `Authorization` header in `connect_http` — the rmcp
   transport already accepts a caller-supplied client, so no transport fork.
   Validation rejects the field on `stdio` (env-var secrets for stdio servers
   already flow through the existing `env:` map).
4. **Token secrecy is tested, not assumed**: a `--verbose` run with a known
   token value asserts the value is absent from stderr. Errors name the
   *variable*, never its content.
5. **Fake klams = klams wire shapes, realistic sizes.** The fake server
   implements `memory_search` returning `PublicMemory` knowledge items per
   the contract doc, with ~800-char `text` payloads, and rejects requests
   missing the bearer header. It extends the 008 fake-MCP binary rather than
   starting a new fixture.
6. **Tool-output cap is a measured decision, not a guess** (FR-006). Default
   `memory_search` (top_k 10 × ~800 chars + JSON overhead) likely exceeds the
   10,000-char cap. The fake-server e2e measures it; the gate: if meaningful
   truncation occurs at default shapes, add per-server `tool_output_limit`
   (default 10,000 — boundary defense stays); if top_k 3–5 keeps results
   comfortably under, document the guidance and keep the cap universal.
   Outcome recorded here at implementation time.
7. **Live tests mirror the TRT-LLM pattern**: `#[ignore]`d, `just
   test-klams`, env-configured (`KLAMS_URL` default `http://kubs0:7777/mcp`,
   `KLAMS_TOKEN` required). Hermetic remains the default truth.

## Workstream structure (maps to tasks.md phases)

1. **WS1 MCP bearer auth** — `auth_token_env` field + validation (HTTP-only);
   `connect_http` default-header injection; actionable missing-var error
   through the log-and-skip path; secrecy test.
2. **WS2 Fake klams + agentic retrieval** — fake klams MCP server (auth
   asserting, klams shapes, realistic payloads); hermetic e2e: prompt →
   agent calls `memory_search` → answer reflects seeded content.
3. **WS3 Workflow retrieval** — `workflows/examples/rag-example.yaml`
   (search tool step → prompt step); e2e + validation-pin tests; FR-006
   cap measurement and decision.
4. **WS4 Degradation + live tests** — dead-endpoint scenarios (skip warning /
   loud workflow failure) for the auth'd-HTTP case; `#[ignore]`d kubs0 tests;
   `just test-klams` recipe.
5. **WS5 Docs truth pass** — docs/08 rewritten to shipped architecture;
   docs/04 MCP auth section; README (`auth_token_env` + RAG example);
   docs/01; roadmap checkboxes at merge.

## Ordering & risk

- WS1 strictly first — everything else talks to an auth'd server.
- WS2 before WS3: the fake klams is WS3's fixture too.
- WS4/WS5 parallel after WS3.
- Biggest risk: **rmcp client/server session compatibility** (stateful
  Streamable HTTP sessions, klams's `LocalSessionManager`). Mitigation: the
  fake klams uses the same rmcp server crate semantics as klams, so the
  hermetic suite exercises the real protocol path; the live test is the
  final word. If rmcp versions skew (m-v's rig-pinned rmcp vs klams's 1.7),
  surface early in WS2.
- Second risk: tool-output truncation degrading retrieval quality silently —
  addressed head-on by the FR-006 gate rather than discovered in production.
- Out-of-repo prerequisite (user): mint a `Read`-scoped token for m-v in
  klams `[[auth.tokens]]` on kubs0 and confirm `:7777/mcp` is reachable from
  the dev box. Needed for SC-006 only; all other work is hermetic.
