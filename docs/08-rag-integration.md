# RAG Integration

**Status (sprint 010, Phase 5.5):** retrieval-augmented context is provided by
**klams** (Ken's Local Agent Memory System), a separate Rust service deployed
on `kubs0`, consumed by multae-viae over MCP. This document describes the
shipped integration.

> Historical note: earlier drafts of this document sketched a build-it-here RAG
> service (a Qdrant store, an Ollama embedding pipeline, and a bespoke RAG MCP
> server). That plan was set aside — klams already provides all of it,
> deployed. A krag-backed alternative was also evaluated and declined (handoff
> in the krag repo, superseded). See `specs/010-klams-rag/` for the decision
> record.

## Architecture

klams owns the entire retrieval stack — vector store (Qdrant), embeddings
(Hugging Face TEI), chunking, ingestion (a filesystem scanner), and hybrid
search — behind an MCP server. multae-viae is a **client**: it sends text
queries and receives ranked results. It never embeds, chunks, or stores
anything itself. Because the boundary is MCP over the network, klams's
implementation language and internals are irrelevant to m-v, and the backend
could be swapped without m-v changes as long as the tool surface holds.

```
┌─────────────────────────────┐          ┌──────────────────────────────┐
│   multae-viae (dev box)     │          │   klams (kubs0)              │
│                             │          │                              │
│  ┌────────────────────┐     │   MCP    │  ┌────────────────────────┐  │
│  │  rmcp MCP client   │◄────┼──Streamable─│  rmcp MCP server :7777 │  │
│  │  (mcp-servers.yaml)│     │  HTTP +  │  │  /mcp  (bearer auth)   │  │
│  └─────────┬──────────┘     │  bearer  │  └───────────┬────────────┘  │
│            │ merge          │          │      ┌───────┴────────┐      │
│  ┌─────────▼──────────┐     │          │  Qdrant │ TEI embed │ PG    │
│  │  agent tool set    │     │          │  (vectors) (HF)  (facts) │   │
│  │  (built-ins + MCP) │     │          │      klams-scanner indexes │  │
│  └────────────────────┘     │          │      ~/src, ~/obsidian …   │  │
└─────────────────────────────┘          └──────────────────────────────┘
```

## What m-v consumes

The klams tool surface m-v depends on is pinned in
[`specs/010-klams-rag/contracts/klams-tool-surface.md`](../specs/010-klams-rag/contracts/klams-tool-surface.md)
— that contract is the source of truth; additive klams changes are safe,
breaking ones require coordination.

**Sprint 010 is read-only.** m-v calls `memory_search` (and could call
`memory_related` / `event_search`) under a **`Read`-scoped** token. Memory
*writes* (`register_author`, `memory_add`, `memory_append_event`) are deferred
to Phase 6 (persistent memory), where author lifecycle deserves its own design.

`memory_search` takes `{ query, top_k?, kinds?, tags? }` and returns ranked
`PublicMemory` items. Knowledge items carry `text`, `source_path`, `tags`, and
provenance; the ranked **order is authoritative** — klams fuses vector and
full-text results (RRF), so the numeric score is not a cross-result-comparable
similarity. m-v consumes the `text`.

## Configuring klams as an MCP server

klams requires a bearer token. m-v reads it from an environment variable named
by `auth_token_env` (the token value never appears in config, logs, or
traces — see [MCP integration](04-mcp-integration.md#authentication)):

```yaml
# mcp-servers.yaml
servers:
  - name: klams
    transport: http
    url: http://kubs0:7777/mcp
    auth_token_env: KLAMS_TOKEN     # the env var holding the bearer token
```

```bash
set -x KLAMS_TOKEN <read-scoped-token>   # fish; or export in bash
```

Once configured, `memory_search` merges into the agent's tool set like any MCP
tool (built-in tools still win on a name collision).

## Two retrieval styles

**Agentic** — the model decides to search. With klams configured, the agent
calls `memory_search` during its multi-turn loop when a prompt needs
background knowledge, then answers from what came back. Nothing else is
required; it is just another tool the model can reach.

**Workflow (deterministic)** — a `tool` step retrieves and a later prompt step
consumes the result, so retrieval does not depend on the model choosing to
search. See [`workflows/examples/rag-example.yaml`](../workflows/examples/rag-example.yaml):

```yaml
steps:
  - id: retrieve
    type: tool
    tool: memory_search
    inputs:
      query: "{{question}}"
      top_k: 5                 # keep small — see "Result size" below
    output: context
  - id: answer
    type: prompt
    template: |
      Answer using only this context:
      {{context}}
      Question: {{question}}
    output: answer
```

### Result size and the tool-output cap

Tool output is capped at 10,000 characters (the universal MCP-tool guard). A
`memory_search` result is a JSON array of chunks; at klams's ~800-char chunk
size, **top_k 3–5 stays comfortably under the cap** (~4–6.5k), while top_k ≈ 8+
can exceed it and truncate the context silently. Keep `top_k` in the 3–5 range
for prompt-bound retrieval. The cap is deliberately a single universal value,
not per-server (measured in `cli_klams.rs`; see
`specs/010-klams-rag/plan.md` §6).

## Degraded mode

If klams is unreachable, the MCP connection failure is **logged and skipped** —
non-RAG work proceeds normally (this is the standard MCP behavior: a failed
server never aborts the run). A workflow that *requires* `memory_search` fails
loudly with the usual unknown-tool error, naming the missing tool. A
missing/empty `auth_token_env` variable is an actionable, non-fatal error
naming the variable and server.

## Ingestion (klams-side)

m-v does not ingest. klams keeps its corpus fresh with `klams-scanner` (an
hourly systemd timer) walking configured roots (`~/src`, `~/obsidian`, …),
chunking on Markdown headings (~800 chars, content-hashed for dedupe), and
pruning vanished files. Adding content to klams's knowledge base is a klams
operation, out of m-v's scope this sprint.

## Known limitations (accepted for now)

Behind the contract, klams today uses a single knowledge collection, a 384-dim
general-purpose embedding model, and heading-based (not code-aware) chunking —
so code retrieval is weaker than prose retrieval. These are improvable inside
klams without m-v changes. Adaptive/code-aware retrieval is not on m-v's
roadmap; it would be a klams enhancement.

## Live verification

`just test-klams` runs `#[ignore]`d round-trips against the real klams on
kubs0 (gated on `KLAMS_TOKEN`; `KLAMS_URL` overrides the endpoint). The
hermetic suite (`just ci`) proves the integration against a fake klams MCP
server and never touches the network.
