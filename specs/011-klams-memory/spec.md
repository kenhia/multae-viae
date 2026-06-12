# Feature Specification: Persistent Memory via klams

**Feature Branch**: `011-klams-memory`  
**Created**: 2026-06-12  
**Status**: Draft  
**Input**: Phase 6.1 from docs/09-roadmap.md (restructured 2026-06-12 into
6.1 memory / 6.2 DSL completion / 6.3 mv-server). Persistent memory is
streamlined onto klams: because klams holds the state, continuity does not
require an always-on server — the CLI remembers across invocations now, and
`mv-server` (sprint 013) later imports the same seam. Writes go through the
same authenticated MCP boundary as sprint 010; the write tools are pinned in
[contracts/klams-tool-surface.md](contracts/klams-tool-surface.md) (v1.1).

**Attribution** (decided with the user 2026-06-12): every memory-active run
registers a fresh author (`register_author` inserts a row per call); real
runs use `agent_name: "mv-cli"` with `session_title`/`model` metadata, live
tests use `agent_name: "multae-viae"`. The user's klams tooling audits
writes by author.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Sessions remember across invocations (Priority: P1)

A user runs `mv-cli --session research "What embedding model does klams
use?"` and later, in a *new process*, `mv-cli --session research "And what
dimension is that?"`. The second run recalls the first turn from klams and
answers in context. Memory is scoped: no `--session` ⇒ no turn recording.

**Acceptance Scenarios**:

1. **Given** a session with at least one recorded turn, **When** a new CLI
   invocation uses the same `--session` name, **Then** the recalled prior
   turn(s) are part of what the model sees (hermetic: the fake proxy's
   request body contains content recorded by the earlier invocation).
2. **Given** a memory-active run, **When** it starts, **Then** exactly one
   `register_author` call is made carrying `agent_name: "mv-cli"`, the
   resolved model id, and the session name as `session_title` — and every
   write from that run carries the returned `author_id`.
3. **Given** a run *without* `--session`, **When** it completes, **Then** no
   turn event is recorded (no `memory_append_event` reaches the server).
4. **Given** a completed memory-active run, **Then** one `conversation`
   event was appended whose payload carries `session`, `prompt`, `response`,
   and `model_used` (large fields truncated client-side).

### User Story 2 - The agent can learn preferences (Priority: P2)

With the Write-scoped token, klams's write tools are in the merged toolset.
When the user tells the agent something worth keeping ("remember that I
prefer fish shell"), the model calls `memory_add` — attributed to the run's
registered author, recallable in later sessions via `memory_search`, and
recoverable klams-side (soft-delete + admin restore) if an agent misbehaves.

**Acceptance Scenarios**:

1. **Given** a memory-active run, **When** the model emits a `memory_add`
   tool call (scripted via the fake proxy), **Then** the call reaches klams
   carrying the run's `author_id` and the tool result returns to the model.
2. **Given** a memory-active run, **Then** the model is told its
   `author_id` and that memory tools are available (a memory addendum to the
   system preamble) — without it the model cannot fill the required
   `author_id` argument.
3. **Given** a run with no memory (no klams or no `--session`), **Then** the
   preamble addendum is absent (no phantom capability).

### User Story 3 - Memory never blocks a prompt (Priority: P1)

klams being down, the token missing, or a write being rejected (maintenance
window, embedding down) must never fail the user's actual request. Memory
operations are strictly best-effort: warn on stderr, continue.

**Acceptance Scenarios**:

1. **Given** `--session` and a dead klams endpoint, **When** the user runs a
   prompt, **Then** it completes normally with a warning; no memory call
   blocks or fails the run.
2. **Given** a registration that fails mid-run (e.g. server up but write
   rejected), **Then** recording is skipped with a warning and the
   completion still returns.

### User Story 4 - Live round-trip on kubs0 (Priority: P3)

`just test-klams` grows a live write/recall round-trip: register (as
`multae-viae`), append a turn event, search it back, and clean up via
`memory_delete`. Gated on `KLAMS_TOKEN` exactly like the 010 live tests.

**Acceptance Scenarios**:

1. **Given** kubs0 reachable and the `Read|Write` token, **When**
   `just test-klams` runs, **Then** the write round-trip passes and the test
   soft-deletes what it wrote.

## Requirements

- **FR-001**: mv-core MUST define a `MemoryStore` trait (the
  `PromptExecutor` pattern: trait in core, impl in the binary) covering the
  sprint's needs: register a session author, record a turn event, recall by
  query, and fetch recent session events. The engine/CLI depend on the
  trait, never on klams types.
- **FR-002**: mv-cli MUST provide the klams-backed impl, calling the pinned
  tools **programmatically through the existing MCP handle** (the
  `HandleToolExecutor` path) — no new protocol client, no REST.
- **FR-003**: A memory-active run (klams configured + `--session <name>`)
  registers one author per contract §Attribution; all writes carry that
  `author_id`; runs without `--session` write nothing.
- **FR-004**: Recall: before the completion, the run fetches recent session
  events (`event_search`, `payload_match {session}`, desc, small limit) and
  relevant memories (`memory_search`, small `top_k`) and renders them into
  the model's context; after the completion it appends one `conversation`
  event (payload fields truncated client-side to a documented cap).
- **FR-005**: Memory is best-effort end to end: any failure (unreachable,
  auth, maintenance window, embedding down) degrades to a stderr warning
  carrying klams's machine-readable error code; the completion path is
  never blocked. The memory preamble addendum (and the model-facing
  `author_id`) appears only when memory is actually active.
- **FR-006**: `FakeKlams` becomes **stateful** (writes stored in-memory) and
  grows the write tools, asserting attribution (the registered
  `author_id` on every write) — so write→recall round-trips are provable
  hermetically across two CLI invocations.
- **FR-007**: Live tests: a kubs0 write/recall round-trip as agent
  `multae-viae`, self-cleaning via `memory_delete`, `#[ignore]`d behind
  `just test-klams`, skipping without `KLAMS_TOKEN`.
- **FR-008**: Docs are part of done: docs/08 gains the memory/write story
  (scope, attribution, degradation); docs/01 sprint entry; README
  `--session` usage; roadmap Phase 6.1 checkboxes + lessons learned at
  merge. The roadmap 6.1/6.2/6.3 restructure lands with this spec.

## Success Criteria

- **SC-001**: Hermetic: two CLI invocations with the same `--session` against
  stateful fake klams + fake proxy — the second invocation's completion
  request contains content recorded by the first.
- **SC-002**: Hermetic: the fake asserts `register_author` (agent_name
  "mv-cli", session_title, model) and that every write carries the returned
  `author_id`; a run without `--session` records nothing.
- **SC-003**: Hermetic: a scripted `memory_add` tool call from the model
  round-trips with correct attribution; the memory preamble addendum is
  present only on memory-active runs.
- **SC-004**: With klams dead and `--session` set, the prompt succeeds with
  a warning; no hang, no failure.
- **SC-005**: `just ci` stays green and hermetic; default suite wall time
  stays ≤ 10s.
- **SC-006**: (Live, opt-in) `just test-klams` write/recall round-trip on
  kubs0 passes and cleans up after itself.
