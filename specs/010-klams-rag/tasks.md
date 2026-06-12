# Tasks: RAG via klams Memory Service

**Input**: Design documents from `/specs/010-klams-rag/`  
**Prerequisites**: plan.md, spec.md, contracts/klams-tool-surface.md

**Tests**: TDD per Principle III — failing hermetic test first. klams behavior
is faked with klams wire shapes (contract doc); live kubs0 tests stay
`#[ignore]`d.

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (FR reference)
```

---

## Phase 1: WS1 — MCP bearer auth

- [X] T001 [WS1] `auth_token_env: Option<String>` on `McpServerConfig`
  (`crates/mv-core/src/mcp/config.rs`); validation rejects it on `stdio`
  transport; parse + validation tests (FR-001)
- [X] T002 [WS1] `connect_http` (`crates/mv-core/src/mcp/client.rs`) resolves
  the env var and builds the `reqwest::Client` with a default
  `Authorization: Bearer` header; missing/empty var → actionable `MvError`
  naming variable + server, surfaced via the existing log-and-skip path;
  unit tests (FR-001). *Extracted `build_http_client()`; header value marked
  `set_sensitive(true)` so reqwest redacts it in Debug/connection logs.
  Live header-on-the-wire assertion deferred to the WS2 fake klams (T004),
  which checks the bearer server-side — the real proof.*
- [X] T003 [P] [WS1] Token secrecy: `--verbose` CLI run with a known token
  value asserts the value never reaches stderr; error text names the
  variable, not the value (FR-002). *`crates/mv-cli/tests/cli_mcp_auth.rs`:
  present-token verbose run (value absent) + missing-var run (names var +
  server, non-fatal).*

**Checkpoint**: m-v can connect to a bearer-auth'd HTTP MCP server; failures
are actionable and non-fatal; `just ci` green.

---

## Phase 2: WS2 — Fake klams + agentic retrieval

- [X] T004 [WS2] Fake klams MCP server (`FakeKlams` in
  `crates/mv-cli/tests/support/`): wiremock-based Streamable HTTP, requires the
  exact bearer header on `/mcp`, implements the rmcp JSON-RPC subset
  (initialize + `Mcp-Session-Id`, GET→405 SSE decline, tools/list, tools/call)
  with a custom `Respond` echoing the request id; `memory_search` returns
  `PublicMemory` knowledge items per the contract with ~800-char `text`;
  seedable (FR-003). *In-process like `FakeProxy` (not a spawned bin), so it
  lives in dev-dep context. Resolves the rmcp HTTP-session risk hermetically.*
- [X] T005 [WS2] Hermetic agentic e2e (`crates/mv-cli/tests/cli_klams.rs`):
  prompt run against fake proxy + fake klams where the agent's
  `memory_search` call retrieves seeded content; proven by the marker reaching
  the model's follow-up request and the bearer reaching klams on the wire
  (FR-004, SC-001)

**Checkpoint**: the Phase 5.5 deliverable proven hermetically in agentic form.

---

## Phase 3: WS3 — Workflow retrieval

- [X] T006 [WS3] `workflows/examples/rag-example.yaml`: `memory_search` tool
  step → prompt step consuming `{{context}}`; e2e hermetic test
  (`workflow_retrieves_context_into_prompt_step` — retrieved marker reaches
  the prompt step's completion request); validation-pin test
  `shipped_rag_example_validates` in `cli_workflow.rs` (FR-005, SC-002)
- [X] T007 [WS3] FR-006 decision gate: measured in
  `realistic_search_payload_stays_under_tool_output_cap` — top_k 5 ≈ 6.5k,
  top_k 10 ≈ 13k. **Decision: keep the universal 10k cap, no per-server
  limit** (no real shape truncates at top_k 3–5); example caps top_k 5, docs
  carry the guidance. Outcome recorded in plan.md §Design decisions 6 (FR-006)

**Checkpoint**: deterministic workflow retrieval shipped; truncation question
answered with data.

---

## Phase 4: WS4 — Degradation + live tests

- [X] T008 [P] [WS4] Degradation tests for the auth'd-HTTP case
  (`cli_klams.rs`): dead klams endpoint → non-RAG prompt succeeds with a skip
  warning naming the server; RAG workflow tool step fails loudly naming
  `memory_search` (SC-003)
- [X] T009 [P] [WS4] Live `#[ignore]`d tests against kubs0 (`KLAMS_URL`
  default `http://kubs0:7777/mcp`, `KLAMS_TOKEN` required; `KLAMS_MODEL` gates
  the agentic one): model-free workflow retrieval + agentic round-trip; both
  early-return when `KLAMS_TOKEN` is unset so the `--ignored` sweep stays
  green. `just test-klams` recipe added (FR-007, SC-006). *Prerequisite
  (user): mint Read token on kubs0 — pending live run.*

**Checkpoint**: failure modes proven; live path runnable on demand.

---

## Phase 5: WS5 — Docs truth pass (polish)

- [ ] T010 [WS5] Rewrite docs/08-rag-integration.md to the shipped
  architecture: klams as the memory/RAG service, MCP boundary diagram, tool
  surface pointer to the contract doc, degraded-mode behavior; drop the
  krag/qdrant/ollama build plan (FR-008)
- [ ] T011 [P] [WS5] docs/04-mcp-integration.md: `auth_token_env`
  configuration + secrecy convention; README: klams server example in
  `mcp-servers.yaml`, RAG example workflow mention (FR-008)
- [ ] T012 [P] [WS5] docs/01-architecture-design.md refresh through sprint
  010 (MCP auth seam, klams integration); roadmap Phase 5.5 checkboxes +
  lessons learned at merge (FR-008)

**Checkpoint**: SC-001–SC-006 met; sprint shippable.
