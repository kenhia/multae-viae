# Tasks: Persistent Memory via klams

**Input**: Design documents from `/specs/011-klams-memory/`  
**Prerequisites**: plan.md, spec.md, contracts/klams-tool-surface.md (v1.1)

**Tests**: TDD per Principle III — failing hermetic test first. klams writes
are proven against the stateful FakeKlams; live kubs0 tests stay `#[ignore]`d
and self-clean.

## Format

```text
- [ ] [TaskID] [P?] [WS#] Description with file path (FR reference)
```

---

## Phase 1: WS1 — Memory seam

- [ ] T001 [WS1] `mv_core::memory`: `MemoryStore` trait
  (`register_session(SessionMeta) -> String`, `record_turn(&str, TurnRecord)`,
  `recall(&str, RecallOpts) -> Vec<MemoryItem>`,
  `session_events(&str, u32) -> Vec<MemoryItem>`) + `SessionMeta` /
  `TurnRecord` / `MemoryItem` types; payload-truncation helper with a
  documented cap; unit tests with a mock impl (FR-001)
- [ ] T002 [WS1] Stateful `FakeKlams` (`crates/mv-cli/tests/support/`):
  `Arc<Mutex<…>>` store behind the responder; `register_author` returns a
  fixed UUID and records the registration input; `memory_add` /
  `memory_append_event` append (rejecting a missing/unknown `author_id`);
  `memory_search` / `event_search` serve seeded + stored items
  (`payload_match` equality, desc order); accessors for test assertions
  (FR-006)
- [ ] T003 [WS1] `KlamsMemory` in mv-cli: implements `MemoryStore` over
  `ToolServerHandle::call_tool` per contract v1.1 (knowledge/fact/event wire
  shapes, error envelopes surfaced as warnings with their machine codes);
  integration-tested against the stateful fake (FR-002, FR-005)

**Checkpoint**: write→read round-trips provable hermetically; `just ci`
green.

---

## Phase 2: WS2 — Session continuity

- [ ] T004 [WS2] `--session <name>` on the prompt command
  (`crates/mv-cli/src/cli.rs`); a `SessionMemory` driver in mv-cli owning
  register → recall → record around `run_prompt`'s completion, kept at the
  edges (no entanglement with streaming/fallback/JSON paths); registration
  per contract §Attribution (`agent_name "mv-cli"`, model, session_title,
  client_app/version) (FR-003, FR-004)
- [ ] T005 [WS2] Hermetic session e2e (`crates/mv-cli/tests/cli_memory.rs`):
  two CLI invocations, same `--session`, one stateful fake — the second
  invocation's completion request contains the first's recorded content
  (SC-001); the fake asserts the `register_author` fields and the
  `author_id` on every write (SC-002); a run without `--session` records
  nothing (FR-003)

**Checkpoint**: m-v remembers across invocations, attributed.

---

## Phase 3: WS3 — Agent-writable memory

- [ ] T006 [WS3] Memory preamble addendum: on memory-active runs only,
  append the run's `author_id` + memory-tool usage note to the system
  preamble; absent otherwise; unit + CLI tests for both states (FR-005,
  SC-003)
- [ ] T007 [WS3] Hermetic agent-write e2e: scripted `memory_add` tool call
  via the fake proxy round-trips through real merged klams tools with the
  run's `author_id`; attribution asserted by the fake (SC-003)

**Checkpoint**: the agent can learn preferences, attributed and recoverable.

---

## Phase 4: WS4 — Degradation + live

- [ ] T008 [P] [WS4] Degradation tests: dead klams + `--session` → prompt
  succeeds with a warning (no hang); write rejected mid-run (fake returns a
  `MAINTENANCE_WINDOW_ACTIVE` envelope) → warn-and-continue, code surfaced
  (FR-005, SC-004)
- [ ] T009 [P] [WS4] Live `#[ignore]`d kubs0 round-trip in `cli_klams.rs`:
  register as `multae-viae` → append event → `event_search` it back →
  `memory_delete` cleanup; skips without `KLAMS_TOKEN`; runs under the
  existing `just test-klams` (FR-007, SC-006)

**Checkpoint**: failure modes proven; live path verified and self-cleaning.

---

## Phase 5: WS5 — Docs truth pass (polish)

- [ ] T010 [WS5] docs/08: "Memory (writes)" section — scope (`--session`),
  attribution model, the preamble addendum, degradation incl. maintenance
  window, pointer to contract v1.1 (FR-008)
- [ ] T011 [P] [WS5] README: `--session` usage + memory behavior; docs/01:
  memory seam (trait in core, impl in binary) + sprint-011 history entry
  (FR-008)
- [ ] T012 [P] [WS5] Roadmap: Phase 6.1 checkboxes + lessons learned at
  merge (the 6.1/6.2/6.3 restructure landed with this spec) (FR-008)

**Checkpoint**: SC-001–SC-006 met; sprint shippable.
