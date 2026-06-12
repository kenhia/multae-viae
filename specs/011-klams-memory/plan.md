# Implementation Plan: Persistent Memory via klams

**Branch**: `011-klams-memory` | **Spec**: [spec.md](spec.md)  
**Research**: klams write-tool schemas read from source 2026-06-12 and pinned
in [contracts/klams-tool-surface.md](contracts/klams-tool-surface.md) (v1.1);
fable Phase 6 pre-work notes
([docs/fable/03](../../docs/fable/03-roadmap-recommendations.md) §Phase 6 —
"persistent memory should repeat the `PromptExecutor` pattern"); sprint 010's
klams integration (auth, FakeKlams, degradation contract).

## Technical Context

**Language**: Rust edition 2024, workspace (`mv-core` lib, `mv-cli` bin)  
**Key deps**: unchanged — rig-core 0.35, rmcp (the existing MCP handle is the
write path too), serde_json, tokio  
**New deps**: none expected; `uuid` NOT added to mv-core (author ids cross
the trait as `String` — only klams needs them to be UUIDs)  
**Testing**: `just ci` hermetic; `FakeKlams` becomes stateful; live kubs0
write round-trip behind `just test-klams`

## Constitution Check

- Spec entry: this plan + spec.md cover all changes (Principle I). ✔
- TDD: every behavior lands with a hermetic test first (Principle III);
  cross-service interaction (klams writes) gets integration tests against
  the stateful fake + a live `#[ignore]`d round-trip. ✔
- YAGNI: no summarization, no token-budgeted context assembly (klams's REST
  `/memory/context` bundler is noted as a 6.3 option, not built), no
  workflow-step memory, no `mv-cli remember` subcommand — the agent's own
  tool call covers explicit remembering. The trait has exactly the four
  methods this sprint uses. ✔
- Docs are part of done: WS6 in-sprint. ✔

## Design decisions

1. **Same boundary, programmatic calls.** `KlamsMemory` calls the pinned
   tools through the existing `ToolServerHandle::call_tool` path — exactly
   how `HandleToolExecutor` executes workflow tool steps today. No REST
   client, no second protocol, no new auth. (Consequence: the 10k tool-output
   truncation applies to recall reads; with small `top_k`/`limit` that is
   comfortably within bounds per the 010 measurements.)
2. **Trait in core, impl in binary** (the fable-named pattern):
   `mv_core::memory::MemoryStore` with four methods —
   `register_session(meta) -> author_id`, `record_turn(author_id, turn)`,
   `recall(query, opts) -> Vec<MemoryItem>`,
   `session_events(session, limit) -> Vec<MemoryItem>`. mv-core stays free
   of klams/rmcp specifics; `author_id` crosses as `String`.
3. **One author per memory-active run** (user-decided attribution).
   `register_author` inserts a new row per call (verified in klams source —
   UUIDv7, not an upsert), so per-run registration is the natural grain:
   the user's tooling sees each run's writes under its own author, with
   `session_title` linking runs of the same session. Real runs:
   `agent_name "mv-cli"`; live tests: `"multae-viae"`.
4. **Recording is opt-in via `--session`; recall and recording travel
   together.** No silent ambient capture: without `--session`, m-v writes
   nothing and injects nothing. With it: recall = recent session events
   (`event_search`, `payload_match {session}`, desc) + relevant memories
   (`memory_search`, small top_k) rendered as a context block; record = one
   `conversation` event per turn (payload `{session, prompt, response,
   model_used}`, fields truncated client-side — events are records, not
   archives).
5. **Agent-writable memory = the tools that already merge + a preamble
   addendum.** With the `Read|Write` token, klams's write tools are in the
   toolset with zero new code. The missing piece is knowledge: the model
   cannot invent the required `author_id`. Memory-active runs append a short
   addendum to the system preamble stating the `author_id` and when to use
   `memory_add`. No wrapper tool unless live use shows models fumbling the
   args (recorded as a possible follow-up, not built).
6. **Best-effort end to end.** Every memory call is wrapped: failure ⇒ one
   stderr warning (carrying klams's machine-readable error code — e.g.
   `MAINTENANCE_WINDOW_ACTIVE` during klams's backup window,
   `EMBEDDING_UNAVAILABLE`) ⇒ the run proceeds. Registration failure
   disables memory for the run rather than erroring per call. The
   completion path never awaits memory on its critical error path.
7. **Stateful FakeKlams.** The 010 fixture is static-seeded; write→recall
   round-trips need real state. The `McpResponder` gains an
   `Arc<Mutex<Vec<…>>>` store: `register_author` returns a fixed known
   UUID, writes append, `memory_search`/`event_search` serve seeded + stored
   items. Statefulness is what lets SC-001 span two CLI subprocesses against
   one fixture.
8. **Scope guard: prompt path only.** Sessions/memory apply to the CLI
   prompt command. Workflow runs are untouched this sprint (a workflow-level
   memory affordance is a 6.2/6.3 question once `Value` context exists).

## Workstream structure (maps to tasks.md phases)

1. **WS1 Memory seam** — `mv_core::memory` (trait + `MemoryItem`/`TurnRecord`
   types); `KlamsMemory` impl in mv-cli over `call_tool`; unit tests with a
   mock store; stateful FakeKlams + write tools.
2. **WS2 Session continuity** — `--session <name>` on the prompt command:
   register → recall → complete → record; hermetic e2e incl. the two-process
   SC-001 round-trip and the no-session-no-writes assertion.
3. **WS3 Agent-writable memory** — preamble addendum (author_id + usage
   note) gated on memory-active; scripted `memory_add` e2e; attribution
   asserted by the fake.
4. **WS4 Degradation + live** — dead-klams and write-rejected paths;
   `cli_klams.rs` live write/recall round-trip with `memory_delete` cleanup
   (agent `multae-viae`).
5. **WS5 Docs truth pass** — docs/08 memory section, docs/01, README
   (`--session`), roadmap 6.1 checkboxes + lessons at merge.

## Ordering & risk

- WS1 first (everything sits on the seam + stateful fake); WS2 before WS3
  (the preamble addendum needs the registered author from WS2's flow);
  WS4/WS5 parallel after WS3.
- Biggest risk: **prompt-path plumbing**. `run_prompt` is already the
  busiest function in mv-cli; threading register/recall/record through it
  without tangling streaming, fallback, and JSON modes needs the memory
  steps kept strictly at the edges (recall before dispatch, record after).
  Mitigation: a small `SessionMemory` driver type in mv-cli owning the
  sequence, unit-tested apart from the CLI.
- Second risk: **recall quality** (stuffing stale turns into every prompt
  can hurt answers). Mitigation: small limits, recency-ordered events, and
  the recall block clearly delimited in the context; quality tuning is
  explicitly out of scope beyond "the model sees the prior turn."
- Out-of-repo prerequisite: ✅ done — the token now carries `Write` scope
  (user, 2026-06-12).
