# RAG Integration

## Architecture

RAG (Retrieval-Augmented Generation) provides the controller with long-term
memory and domain-specific knowledge. The RAG service runs on a separate machine
on the local network, exposed as an MCP server.

```
┌─────────────────────────────┐          ┌──────────────────────────┐
│     Multae Viae Controller  │          │    RAG Service           │
│     (this machine)          │          │    (network machine)     │
│                             │          │                          │
│  ┌────────────────────┐     │   HTTP   │  ┌────────────────────┐  │
│  │    MCP Client      │◄────┼──────────┼──│   MCP Server       │  │
│  │  (rmcp, HTTP)      │     │          │  │   (rmcp)           │  │
│  └────────────────────┘     │          │  └────────────────────┘  │
│                             │          │           │              │
│  ┌────────────────────┐     │          │  ┌────────▼───────────┐  │
│  │  Embedding Model   │     │          │  │  Vector Store      │  │
│  │  (local, Ollama)   │     │          │  │  (Qdrant/LanceDB)  │  │
│  └────────────────────┘     │          │  └────────────────────┘  │
│                             │          │                          │
│  ┌────────────────────┐     │          │  ┌────────────────────┐  │
│  │  Document Loader   │     │          │  │  Embedding Model   │  │
│  │  (ingestion)       │     │          │  │  (local or shared) │  │
│  └────────────────────┘     │          │  └────────────────────┘  │
└─────────────────────────────┘          └──────────────────────────┘
```

## RAG as MCP Server

The RAG service exposes its capabilities via MCP primitives:

### Tools

```yaml
# Tools exposed by the RAG MCP server
tools:
  - name: search_documents
    description: Search the knowledge base for relevant documents
    parameters:
      query: string            # Natural language search query
      collection: string       # Which collection to search
      limit: integer           # Max results (default: 5)
      min_score: float         # Minimum similarity score (0-1)
    
  - name: ingest_document
    description: Add a document to the knowledge base
    parameters:
      content: string          # Document text
      metadata: object         # Title, source, tags, etc.
      collection: string       # Target collection

  - name: list_collections
    description: List available document collections
    
  - name: delete_document
    description: Remove a document by ID
    parameters:
      id: string
      collection: string
```

### Resources

```yaml
# Resources exposed by the RAG MCP server
resources:
  - uri: "rag://collections"
    description: List of available collections and their stats
  - uri: "rag://collections/{name}/stats"
    description: Statistics for a specific collection
```

## Vector Store Options

### Qdrant — ⭐ Recommended

**Why**: Purpose-built vector database, excellent performance, REST + gRPC API,
Rig integration (`rig-qdrant`), easy Docker deployment.

```bash
# Run Qdrant
docker run -p 6333:6333 -p 6334:6334 \
  -v $(pwd)/qdrant_data:/qdrant/storage \
  qdrant/qdrant
```

```rust
// Rig + Qdrant integration
use rig_qdrant::QdrantVectorStore;

let qdrant = QdrantVectorStore::new("http://rag-machine:6333", "knowledge_base");
let agent = client.agent("qwen3:8b")
    .preamble("You are a helpful assistant.")
    .dynamic_context(2, qdrant.index())  // RAG with top-2 results
    .build();
```

### LanceDB — Alternative

**Why**: Embedded vector database (no separate server), good for simpler
setups, Rig integration (`rig-lancedb`).

```rust
use rig_lancedb::LanceDbVectorStore;

let db = lancedb::connect("data/lancedb").await?;
let store = LanceDbVectorStore::new(db, "documents");
```

### SQLite with Vector Extension

**Why**: Simplest possible option, Rig integration (`rig-sqlite`), no
additional infrastructure.

## Embedding Models

### Local Embeddings (Recommended)

Run embedding models locally via Ollama:

```bash
ollama pull nomic-embed-text     # 137M params, good quality
ollama pull mxbai-embed-large    # 335M params, higher quality
```

```rust
// Generate embeddings via Ollama
let embeddings = client.embeddings("nomic-embed-text")
    .embed("text to embed")
    .await?;
```

### Via Candle (Custom)

For custom embedding models or when you need more control:

```rust
use candle_core::{Device, Tensor};
use candle_transformers::models::bert;

// Load and run a BERT model for embeddings
let model = bert::BertModel::load(weights, &config, device)?;
let embeddings = model.forward(&input_ids, &token_type_ids)?;
```

## RAG Pipeline

### Ingestion Pipeline

```
Document Source
    │
    ▼
┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│  Load    │────▶│  Chunk   │────▶│  Embed   │────▶│  Store   │
│  (parse) │     │  (split) │     │  (model) │     │  (vector │
│          │     │          │     │          │     │   DB)    │
└──────────┘     └──────────┘     └──────────┘     └──────────┘
```

**Loading**: Extract text from various formats:
- PDF, DOCX, TXT, MD, HTML
- Code files (with language-aware chunking)
- Web pages (via scraping)

**Chunking strategies**:
- Fixed size with overlap (simple, works for most text)
- Semantic chunking (split on topic changes)
- Code-aware chunking (split on function/class boundaries)
- Recursive splitting (split large chunks further)

**Kalosm utilities**: The Kalosm crate provides built-in document extraction
and chunking utilities that could be useful here:
```rust
use kalosm::language::*;

// Extract context from various formats
let document = Document::from_path("research.pdf")?;
let chunks = document.chunked(ChunkStrategy::Sentence { overlap: 2 });
```

### Retrieval Pipeline

```
User Query
    │
    ▼
┌──────────┐     ┌──────────┐     ┌──────────┐     ┌──────────┐
│  Embed   │────▶│  Search  │────▶│  Rerank  │────▶│  Format  │
│  Query   │     │  Vector  │     │(optional)│     │  Context │
│          │     │   DB     │     │          │     │          │
└──────────┘     └──────────┘     └──────────┘     └──────────┘
                                                        │
                                                        ▼
                                                   ┌───────────┐
                                                   │  LLM      │
                                                   │  Prompt   │
                                                   │  + Context│
                                                   └───────────┘
```

### Rig's RAG Integration

Rig makes RAG straightforward with its `dynamic_context` builder:

```rust
let index = qdrant_store.index(embedding_model);

let agent = client.agent("qwen3:8b")
    .preamble("You are a helpful assistant. Use the provided context to answer.")
    .dynamic_context(3, index)  // Retrieve top-3 chunks per query
    .build();

// Queries automatically retrieve relevant context
let response = agent.prompt("What were the key findings?").await?;
```

## Data to Index

For an "always-on second brain" agent, consider indexing:

| Source | Type | Update Frequency |
|--------|------|-----------------|
| Personal notes | Markdown files | On file change (watch) |
| Code repositories | Code + docs | On commit |
| Bookmarks/articles | Web content | On add |
| Meeting notes | Text/audio transcription | After each meeting |
| Terminal history | Command history | Periodic |
| Email summaries | Text | Periodic |
| Calendar events | Structured data | Periodic |
| Project documentation | Various formats | On change |

## Network Considerations

Since the RAG service is on a different machine on the local network:

1. **Transport**: Use MCP over Streamable HTTP (not stdio)
2. **Latency**: Expect 1-10ms network latency (LAN)
3. **Authentication**: Use API keys or mTLS for the MCP connection
4. **Availability**: Handle RAG service being temporarily unavailable
   (graceful degradation — answer without context)
5. **Bandwidth**: Embedding vectors are small (~1.5KB for 384-dim float32),
   document chunks are typically 500-2000 tokens (~2-8KB)
