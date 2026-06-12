# Feature Specification: RAG via klams Memory Service

**Feature Branch**: `010-klams-rag`  
**Created**: 2026-06-12  
**Status**: Draft  
**Input**: Phase 5.5 from docs/09-roadmap.md — retrieval-augmented context for
agent workflows. Backend decision (2026-06-12): **klams** (Ken's Local Agent
Memory System, Rust, already deployed on kubs0) replaces the originally
sketched krag integration — klams already provides the MCP server, auth,
hybrid retrieval, ingestion (scanner + push), and deployment that the krag
path would have had to build. The krag handoff
(`krag/specs/planning/handoff-mv-rag-mcp.md`) is superseded for the m-v role.
The klams tool surface m-v depends on is pinned in
[contracts/klams-tool-surface.md](contracts/klams-tool-surface.md).

Scope note: this sprint is **read-only** against klams (`memory_search` and
friends under a `Read`-scoped token). Memory *writes* (`register_author`,
`memory_add`, `memory_append_event`) belong to Phase 6 persistent memory.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Connect to an authenticated MCP server (Priority: P1)

klams requires a bearer token; m-v's HTTP MCP transport sends none today. A
user adds `auth_token_env: KLAMS_TOKEN` to a server entry in
`mcp-servers.yaml`; m-v reads the token from that environment variable and
sends `Authorization: Bearer <token>` on every request to that server. The
token value never appears in config files, logs, traces, or error messages.

**Acceptance Scenarios**:

1. **Given** a server entry with `auth_token_env` set and the variable
   present, **When** m-v connects, **Then** every HTTP request carries the
   bearer header (asserted by a fake server).
2. **Given** `auth_token_env` naming an unset variable, **When** m-v starts,
   **Then** connection to that server fails with an actionable error naming
   the variable and the server — and the failure is non-fatal (logged and
   skipped) exactly like any other MCP server failure.
3. **Given** a token in the environment, **When** running with `--verbose`,
   **Then** the token value appears nowhere in stderr output.
4. **Given** `auth_token_env` on a `stdio` transport entry, **When** the
   config loads, **Then** validation rejects it (the field is HTTP-only).

### User Story 2 - The agent retrieves context on its own (Priority: P1)

With klams configured, its tools (`memory_search`, …) merge into the agent
toolset like any MCP server's. A user asks a knowledge question; the agent
calls `memory_search` during its multi-turn loop and answers using retrieved
content. This is the Phase 5.5 deliverable in agentic form.

**Acceptance Scenarios**:

1. **Given** a fake klams server seeded with a known fact, **When** the user
   runs a prompt whose answer requires it, **Then** the agent calls
   `memory_search` and the final answer reflects the retrieved content
   (hermetic, via the fake proxy + fake klams).
2. **Given** a name collision between a klams tool and a built-in, **Then**
   existing precedence holds (built-ins win) — no new behavior, covered by
   existing tests.

### User Story 3 - Workflows retrieve context explicitly (Priority: P2)

A workflow author writes a `tool` step calling `memory_search` and feeds
`{{results}}` into a later prompt step — deterministic retrieval, no reliance
on the model choosing to search. A shipped example workflow demonstrates the
pattern.

**Acceptance Scenarios**:

1. **Given** the shipped RAG example workflow, **When** run against the fake
   klams + fake proxy, **Then** the search step's output is rendered into the
   prompt step and the workflow completes (e2e, hermetic).
2. **Given** the shipped example, **When** `workflow validate` runs in the
   test suite, **Then** it validates clean (the example can't rot).
3. **Given** realistic klams payloads (top_k × ~800-char chunks), **When** the
   tool step runs, **Then** results are not silently mangled by the 10,000-char
   tool-output cap — see FR-006 for the decision gate.

### User Story 4 - Graceful degradation when klams is down (Priority: P2)

The RAG service being unreachable must not break non-RAG work (existing MCP
behavior: log, skip, continue). A workflow that *requires* the tool fails
loudly with the existing unknown-tool error.

**Acceptance Scenarios**:

1. **Given** a klams entry pointing at a dead endpoint, **When** a non-RAG
   prompt runs, **Then** it succeeds; a warning notes the skipped server.
2. **Given** the same dead endpoint, **When** a workflow with a
   `memory_search` tool step runs, **Then** it fails loudly naming the missing
   tool (existing semantics, verified for the auth'd-HTTP case).

### User Story 5 - Live verification against kubs0 (Priority: P3)

`#[ignore]`d live tests (pattern: `just test-trtllm`) run the agentic and
workflow paths against the real klams on kubs0, gated on `KLAMS_TOKEN` and
reachability. This is the roadmap deliverable check, not part of `just ci`.

**Acceptance Scenarios**:

1. **Given** kubs0 reachable and a valid `Read` token, **When**
   `just test-klams` runs, **Then** a real `memory_search` round-trips and
   returns plausibly-shaped results through both paths.

## Requirements

- **FR-001**: `McpServerConfig` MUST support `auth_token_env: Option<String>`
  (HTTP transport only; config validation rejects it on `stdio`). Connection
  MUST read the token from the named variable at connect time and send
  `Authorization: Bearer <token>` on every request. A missing/empty variable
  MUST produce an actionable error naming the variable and server, surfaced
  through the existing log-and-skip path.
- **FR-002**: The token value MUST NOT appear in logs, traces, span
  attributes, or error text (mirroring the `api_key_env` convention).
- **FR-003**: The test suite MUST include a hermetic fake klams MCP server
  (extending the 008 fake-MCP infrastructure) that (a) requires the bearer
  header, (b) implements `memory_search` with klams's wire shapes
  (`PublicMemory` knowledge results per the contract doc), and (c) serves
  payloads of realistic size.
- **FR-004**: Agentic retrieval MUST be proven hermetically: a CLI prompt run
  against fake proxy + fake klams where the agent's tool call retrieves
  seeded content that shapes the final answer.
- **FR-005**: A shipped example workflow
  (`workflows/examples/rag-example.yaml`) MUST demonstrate tool-step
  retrieval feeding a prompt step; e2e hermetic test plus a validation test
  pinning the example.
- **FR-006**: The 10,000-char tool-output cap MUST be evaluated against
  realistic `memory_search` payloads in-sprint. If default-shaped results
  (top_k ≤ 10) truncate meaningfully, the cap becomes per-server configurable
  (`tool_output_limit`, default unchanged) in this sprint; otherwise the
  finding and top_k guidance (3–5 for prompt-bound retrieval) are documented
  and the cap stands. Either way the decision is recorded in plan.md.
- **FR-007**: Live tests against kubs0 klams MUST exist, `#[ignore]`d, opt-in
  via a `just test-klams` recipe, configured by environment
  (`KLAMS_URL` override, `KLAMS_TOKEN`). The default suite stays hermetic.
- **FR-008**: Docs are part of done: docs/08 rewritten to the shipped klams
  architecture (the krag/qdrant/ollama sketch replaced, MCP-boundary diagram
  updated); docs/04 documents MCP auth config; README gains the
  `auth_token_env` example and the RAG example; docs/01 and roadmap Phase 5.5
  updated (roadmap amendment lands with this spec).

## Success Criteria

- **SC-001**: Hermetic: a prompt run retrieves seeded content from the fake
  klams (bearer header asserted) and the answer reflects it.
- **SC-002**: Hermetic: the shipped RAG example workflow runs e2e; its
  validation is pinned by a test.
- **SC-003**: With the klams endpoint dead: non-RAG prompt succeeds with a
  skip warning; a RAG workflow fails loudly naming the missing tool.
- **SC-004**: Missing token variable yields an actionable error naming it;
  a `--verbose` run never prints the token value.
- **SC-005**: `just ci` stays green and hermetic; default suite wall time
  stays ≤ 10s.
- **SC-006**: (Live, opt-in) `just test-klams` round-trips real retrieval
  from kubs0 through both the agentic and workflow paths — the Phase 5.5
  deliverable.
