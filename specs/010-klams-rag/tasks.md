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

- [ ] T001 [WS1] `auth_token_env: Option<String>` on `McpServerConfig`
  (`crates/mv-core/src/mcp/config.rs`); validation rejects it on `stdio`
  transport; parse + validation tests (FR-001)
- [ ] T002 [WS1] `connect_http` (`crates/mv-core/src/mcp/client.rs`) resolves
  the env var and builds the `reqwest::Client` with a default
  `Authorization: Bearer` header; missing/empty var → actionable `MvError`
  naming variable + server, surfaced via the existing log-and-skip path;
  unit tests incl. header presence via a wiremock-style assertion (FR-001)
- [ ] T003 [P] [WS1] Token secrecy: `--verbose` CLI run with a known token
  value asserts the value never reaches stderr; error text names the
  variable, not the value (FR-002)

**Checkpoint**: m-v can connect to a bearer-auth'd HTTP MCP server; failures
are actionable and non-fatal; `just ci` green.

---

## Phase 2: WS2 — Fake klams + agentic retrieval

- [ ] T004 [WS2] Fake klams MCP server (extend
  `crates/mv-cli/tests/support/` / the 008 fake-MCP binary): Streamable HTTP,
  rejects requests without the expected bearer token, implements
  `memory_search` returning `PublicMemory` knowledge items per
  contracts/klams-tool-surface.md with realistic ~800-char `text` payloads;
  seedable results (FR-003)
- [ ] T005 [WS2] Hermetic agentic e2e (`crates/mv-cli/tests/cli_klams.rs`):
  prompt run against fake proxy + fake klams where the agent's
  `memory_search` call retrieves seeded content that shapes the final
  answer; bearer header asserted server-side (FR-004, SC-001)

**Checkpoint**: the Phase 5.5 deliverable proven hermetically in agentic form.

---

## Phase 3: WS3 — Workflow retrieval

- [ ] T006 [WS3] `workflows/examples/rag-example.yaml`: `memory_search` tool
  step → prompt step consuming `{{results}}`; e2e hermetic test against fake
  klams + fake proxy; validation-pin test in `cli_workflow.rs` (FR-005,
  SC-002)
- [ ] T007 [WS3] FR-006 decision gate: measure default-shape `memory_search`
  output against the 10,000-char tool-output cap in the e2e test; either add
  per-server `tool_output_limit` (config + registry plumbing + tests) or
  document top_k guidance — record the outcome in plan.md §Design decisions 6
  (FR-006)

**Checkpoint**: deterministic workflow retrieval shipped; truncation question
answered with data.

---

## Phase 4: WS4 — Degradation + live tests

- [ ] T008 [P] [WS4] Degradation tests for the auth'd-HTTP case: dead klams
  endpoint → non-RAG prompt succeeds with skip warning; RAG workflow tool
  step fails loudly naming the missing tool (SC-003)
- [ ] T009 [P] [WS4] Live `#[ignore]`d tests against kubs0
  (`KLAMS_URL` default `http://kubs0:7777/mcp`, `KLAMS_TOKEN` required):
  agentic + workflow round-trips; `just test-klams` recipe in the justfile
  (FR-007, SC-006). *Prerequisite (user): mint Read token on kubs0.*

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
