// Shared test-helper module: each integration-test binary that pulls it in via
// `mod support;` uses a different subset, so unused-in-one-binary helpers are
// expected.
#![allow(dead_code)]

//! Test support: a scripted fake OpenAI/TRT-LLM proxy (T025).
//!
//! Backs the hermetic integration tests in `cli_fake_proxy.rs`. The fixture
//! speaks just enough of the OpenAI Chat Completions surface for rig's
//! `CompletionsClient` plus the mv-cli TRT-LLM preflights:
//!
//! - `GET  /health`               → 200 (the `check_health` preflight)
//! - `GET  /v1/models`            → `{"data":[{"id": …}]}` (streaming preflight)
//! - `POST /v1/chat/completions`  → scripted: plain text, 502 + Triton body,
//!   a tool_calls → final-message round trip, or an SSE stream.
//!
//! The CLI under test runs as a subprocess (assert_cmd), so the wiremock
//! server runs on a private tokio runtime owned by the fixture — tests stay
//! plain `#[test]` functions and drive the binary synchronously.

use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A running fake proxy. Dropping it shuts down both the mock server and its
/// runtime.
pub struct FakeProxy {
    rt: tokio::runtime::Runtime,
    server: MockServer,
}

impl FakeProxy {
    pub fn start() -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build fixture runtime");
        let server = rt.block_on(MockServer::start());
        Self { rt, server }
    }

    /// OpenAI-style base URL (`…/v1`) for a models.yaml `endpoint:` entry.
    pub fn endpoint(&self) -> String {
        format!("{}/v1", self.server.uri())
    }

    fn mount(&self, mock: Mock) {
        self.rt.block_on(self.server.register(mock));
    }

    /// `GET /health` → 200, the buffered/streaming preflight.
    pub fn mount_health_ok(&self) {
        self.mount(
            Mock::given(method("GET"))
                .and(path("/health"))
                .respond_with(ResponseTemplate::new(200).set_body_string("OK")),
        );
    }

    /// `GET /v1/models` → an OpenAI model list containing `served` ids.
    pub fn mount_models(&self, served: &[&str]) {
        let data: Vec<serde_json::Value> = served
            .iter()
            .map(|id| serde_json::json!({"id": id, "object": "model"}))
            .collect();
        self.mount(
            Mock::given(method("GET"))
                .and(path("/v1/models"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "object": "list",
                    "data": data,
                }))),
        );
    }

    fn completion_body(
        message: serde_json::Value,
        usage: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let finish_reason = if message.get("tool_calls").is_some() {
            "tool_calls"
        } else {
            "stop"
        };
        let mut body = serde_json::json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion",
            "created": 0,
            "model": "fake-llama-served",
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason,
            }],
        });
        if let Some(usage) = usage {
            body["usage"] = usage;
        }
        body
    }

    fn default_usage() -> serde_json::Value {
        serde_json::json!({"prompt_tokens": 7, "completion_tokens": 12, "total_tokens": 19})
    }

    /// `POST /v1/chat/completions` → 200 with a plain assistant message + usage.
    pub fn mount_chat_text(&self, text: &str) {
        let body = Self::completion_body(
            serde_json::json!({"role": "assistant", "content": text}),
            Some(Self::default_usage()),
        );
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body)),
        );
    }

    /// Like [`mount_chat_text`] but with a degenerate `usage` payload — a
    /// proxy that omits usage entirely must not break the buffered path.
    pub fn mount_chat_text_without_usage(&self, text: &str) {
        let body = Self::completion_body(
            serde_json::json!({"role": "assistant", "content": text}),
            None,
        );
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body)),
        );
    }

    /// `POST /v1/chat/completions` → 502 with the verbatim Triton body the
    /// live proxy emits for an unloaded model.
    pub fn mount_chat_502_triton(&self, served_model: &str) {
        let body = format!(
            "{{\"detail\":\"Triton returned HTTP 404: {{\\\"error\\\":\\\"Request for unknown model: 'ensemble_{served_model}' is not found\\\"}}\"}}",
        );
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(502).set_body_raw(body.into_bytes(), "application/json"),
                ),
        );
    }

    /// `POST /v1/chat/completions` → 200 with an empty `choices` array. The
    /// HTTP call succeeds, so this is not an `HttpError`; rig fails to extract a
    /// message and the error classifies to `CompletionFailed` — NOT
    /// fallback-eligible, so the chain walker must fail fast on it.
    pub fn mount_chat_no_choices(&self) {
        let body = serde_json::json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion",
            "created": 0,
            "model": "fake-llama-served",
            "choices": [],
        });
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body)),
        );
    }

    /// Script a multi-turn tool round trip:
    /// - first request → assistant message with one `tool_calls` entry for
    ///   `tool_name(args)`;
    /// - the follow-up request (recognizable by its `role:"tool"` result
    ///   message) → a final assistant message `final_text`.
    pub fn mount_tool_call_then_final(
        &self,
        tool_name: &str,
        args: serde_json::Value,
        final_text: &str,
    ) {
        // The follow-up matcher is more specific; give it higher priority
        // (lower number) so it wins once the tool result is in the history.
        let final_body = Self::completion_body(
            serde_json::json!({"role": "assistant", "content": final_text}),
            Some(Self::default_usage()),
        );
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .and(body_string_contains("\"role\":\"tool\""))
                .respond_with(ResponseTemplate::new(200).set_body_json(final_body))
                .with_priority(1),
        );

        let tool_call_body = Self::completion_body(
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "call_fake_1",
                    "type": "function",
                    "function": {"name": tool_name, "arguments": args},
                }],
            }),
            Some(Self::default_usage()),
        );
        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(tool_call_body))
                .with_priority(5),
        );
    }

    /// `POST /v1/chat/completions` → an SSE stream emitting `chunks` as
    /// delta-content events, then a stop event, a usage event, and `[DONE]`.
    pub fn mount_chat_sse(&self, chunks: &[&str]) {
        let mut body = String::new();
        for chunk in chunks {
            let event = serde_json::json!({
                "id": "chatcmpl-fake",
                "object": "chat.completion.chunk",
                "created": 0,
                "model": "fake-llama-served",
                "choices": [{
                    "index": 0,
                    "delta": {"content": chunk},
                    "finish_reason": null,
                }],
            });
            body.push_str(&format!("data: {event}\n\n"));
        }
        let stop = serde_json::json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "fake-llama-served",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        });
        body.push_str(&format!("data: {stop}\n\n"));
        let usage = serde_json::json!({
            "id": "chatcmpl-fake",
            "object": "chat.completion.chunk",
            "created": 0,
            "model": "fake-llama-served",
            "choices": [],
            "usage": {"prompt_tokens": 5, "completion_tokens": 9, "total_tokens": 14},
        });
        body.push_str(&format!("data: {usage}\n\n"));
        body.push_str("data: [DONE]\n\n");

        self.mount(
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_raw(body.into_bytes(), "text/event-stream"),
                ),
        );
    }

    /// All requests the fixture has received so far.
    pub fn received_requests(&self) -> Vec<wiremock::Request> {
        self.rt
            .block_on(self.server.received_requests())
            .unwrap_or_default()
    }

    /// Bodies (parsed as JSON) of every `POST /v1/chat/completions` received.
    pub fn chat_request_bodies(&self) -> Vec<serde_json::Value> {
        self.received_requests()
            .into_iter()
            .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/v1/chat/completions")
            .filter_map(|r| serde_json::from_slice(&r.body).ok())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// FakeKlams: a hermetic klams MCP server over Streamable HTTP.
//
// klams speaks MCP over Streamable HTTP with a scoped bearer token. This
// fixture stands in for it: a wiremock server that (a) requires the exact
// bearer header on `/mcp` (a missing/wrong token simply does not match, so the
// rmcp handshake fails — that *is* the auth enforcement), (b) answers the
// JSON-RPC subset rmcp's client drives (initialize → notifications/initialized
// → tools/list → tools/call), and (c) returns `memory_search` results shaped
// like klams's `PublicMemory` knowledge items (see
// specs/010-klams-rag/contracts/klams-tool-surface.md).
//
// Streamable HTTP specifics handled here, learned from the rmcp 1.5 client:
//   - initialize MUST return an `Mcp-Session-Id` header (the client errors with
//     `MissingSessionIdInResponse` otherwise — it is not stateless by default);
//   - plain `application/json` JSON-RPC responses are accepted (no SSE needed);
//   - the client opens a background GET for an SSE stream — answering 405 maps
//     to `ServerDoesNotSupportSse`, which the client tolerates and skips.
// Each response echoes the request's JSON-RPC `id`, so a custom `Respond` impl
// (not a static template) is required.
// ---------------------------------------------------------------------------

/// One seeded knowledge chunk the fake `memory_search` will return.
pub struct KlamsChunk {
    pub text: String,
    pub source_path: String,
}

/// The single author id every `register_author` call returns. Real klams mints
/// a fresh UUIDv7 per call; the fake uses one fixed id so tests can assert that
/// every write carries it.
pub const FAKE_AUTHOR_ID: &str = "00000000-0000-7000-8000-00000000a001";

/// Mutable fake-klams state, shared between the responder and test accessors.
/// Sprint 011 makes the fixture stateful so write→recall round-trips are
/// provable across two CLI subprocesses talking to one fixture.
#[derive(Default)]
struct KlamsState {
    /// `register_author` argument objects, in call order.
    registrations: Vec<serde_json::Value>,
    /// Stored knowledge/fact `PublicMemory` items (from `memory_add`).
    knowledge: Vec<serde_json::Value>,
    /// Stored event `PublicMemory` items (from `memory_append_event`).
    events: Vec<serde_json::Value>,
    /// The `author_id` seen on every write, in order (attribution assertions).
    write_author_ids: Vec<String>,
    /// When set, writes are rejected with this `(code, message)` envelope —
    /// simulates e.g. the klams backup maintenance window.
    reject_writes: Option<(String, String)>,
}

/// Custom wiremock responder implementing the MCP JSON-RPC subset plus the
/// klams memory tools, backed by shared mutable state.
struct McpResponder {
    session_id: String,
    /// Seeded knowledge items (from `KlamsChunk`s), always returned by search.
    seeded: Vec<serde_json::Value>,
    state: std::sync::Arc<std::sync::Mutex<KlamsState>>,
}

impl McpResponder {
    /// A successful `tools/call` result wrapping `text` as a single content
    /// block — rig concatenates text content into the tool's return string.
    fn ok(id: &serde_json::Value, text: String) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"content": [{"type": "text", "text": text}], "isError": false},
        }))
    }

    /// A tool error result (`isError: true`) — rig maps this to `Err`, with
    /// `text` as the error message (so the code string reaches m-v's warning).
    fn tool_err(id: &serde_json::Value, text: String) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"content": [{"type": "text", "text": text}], "isError": true},
        }))
    }

    fn handle_tool_call(
        &self,
        id: &serde_json::Value,
        params: Option<&serde_json::Value>,
    ) -> ResponseTemplate {
        let name = params
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
            .unwrap_or("");
        let args = params
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        match name {
            "register_author" => {
                self.state.lock().unwrap().registrations.push(args);
                Self::ok(
                    id,
                    serde_json::json!({
                        "author_id": FAKE_AUTHOR_ID,
                        "agent_name": "mv-cli",
                        "created_at": "2026-06-12T00:00:00Z",
                    })
                    .to_string(),
                )
            }
            "memory_add" => self.handle_write(id, &args, false),
            "memory_append_event" => self.handle_write(id, &args, true),
            "memory_search" => {
                let st = self.state.lock().unwrap();
                let mut items = self.seeded.clone();
                items.extend(st.knowledge.iter().cloned());
                Self::ok(id, serde_json::to_string(&items).unwrap())
            }
            "event_search" => {
                let st = self.state.lock().unwrap();
                let want = args.get("payload_match").and_then(|m| m.as_object());
                let mut matched: Vec<serde_json::Value> = st
                    .events
                    .iter()
                    .filter(|e| match want {
                        None => true,
                        Some(m) => {
                            let payload = e.get("payload").and_then(|p| p.as_object());
                            m.iter().all(|(k, v)| {
                                payload
                                    .and_then(|p| p.get(k))
                                    .map(|pv| pv == v)
                                    .unwrap_or(false)
                            })
                        }
                    })
                    .cloned()
                    .collect();
                // Default order is newest-first.
                matched.reverse();
                Self::ok(id, serde_json::json!({"events": matched}).to_string())
            }
            "memory_delete" => Self::ok(
                id,
                serde_json::json!({
                    "id": args.get("id").cloned().unwrap_or(serde_json::Value::Null),
                    "deleted_at": "2026-06-12T00:00:00Z",
                })
                .to_string(),
            ),
            other => Self::tool_err(id, format!("unknown tool: {other}")),
        }
    }

    /// Shared `memory_add` / `memory_append_event` handling: enforce a present,
    /// known `author_id`, honor the reject switch, store the item, return it.
    fn handle_write(
        &self,
        id: &serde_json::Value,
        args: &serde_json::Value,
        is_event: bool,
    ) -> ResponseTemplate {
        let author = args.get("author_id").and_then(|a| a.as_str()).unwrap_or("");
        if author.is_empty() {
            return Self::tool_err(id, "MISSING_AUTHOR_ID: author_id is required".to_string());
        }
        if author != FAKE_AUTHOR_ID {
            return Self::tool_err(id, format!("UNKNOWN_AUTHOR_ID: {author}"));
        }

        let mut st = self.state.lock().unwrap();
        if let Some((code, msg)) = st.reject_writes.clone() {
            return Self::tool_err(id, format!("{code}: {msg}"));
        }
        st.write_author_ids.push(author.to_string());

        let item = if is_event {
            let idx = st.events.len();
            let ev = serde_json::json!({
                "id": format!("00000000-0000-7000-8000-{:012}", 1000 + idx),
                "kind": "event",
                "category": args.get("category").cloned().unwrap_or(serde_json::Value::Null),
                "payload": args.get("payload").cloned().unwrap_or_else(|| serde_json::json!({})),
                "tags": [],
                "author": {"id": author, "agent_name": "mv-cli"},
                "created_at": "2026-06-12T00:00:00Z",
                "updated_at": "2026-06-12T00:00:00Z",
            });
            st.events.push(ev.clone());
            ev
        } else {
            let idx = st.knowledge.len();
            let item = serde_json::json!({
                "id": format!("00000000-0000-7000-8000-{:012}", 2000 + idx),
                "kind": "knowledge",
                "text": args.get("text").cloned().unwrap_or(serde_json::Value::Null),
                "tags": args.get("tags").cloned().unwrap_or_else(|| serde_json::json!([])),
                "author": {"id": author, "agent_name": "mv-cli"},
                "created_at": "2026-06-12T00:00:00Z",
                "updated_at": "2026-06-12T00:00:00Z",
            });
            st.knowledge.push(item.clone());
            item
        };
        Self::ok(id, item.to_string())
    }
}

impl Respond for McpResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let msg: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        let id = msg.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "initialize" => {
                let requested = msg
                    .get("params")
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("2025-06-18");
                ResponseTemplate::new(200)
                    .insert_header("Mcp-Session-Id", self.session_id.as_str())
                    .set_body_json(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "protocolVersion": requested,
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "fake-klams", "version": "0.1.0"},
                        },
                    }))
            }
            // A notification (no id): acknowledge with 202 Accepted.
            "notifications/initialized" => ResponseTemplate::new(202),
            "tools/list" => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": memory_tools_list(),
            })),
            "tools/call" => self.handle_tool_call(&id, msg.get("params")),
            "ping" => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0", "id": id, "result": {},
            })),
            other => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("method not found: {other}")},
            })),
        }
    }
}

/// `tools/list` advertising the read + write memory tools m-v uses (contract
/// v1.1). Schemas are minimal — rig only needs name + an object schema.
fn memory_tools_list() -> serde_json::Value {
    let obj = |required: &[&str]| serde_json::json!({"type": "object", "properties": {}, "required": required});
    serde_json::json!({
        "tools": [
            {"name": "memory_search", "description": "Search memory (hybrid).", "inputSchema": obj(&["query"])},
            {"name": "register_author", "description": "Register an author.", "inputSchema": obj(&["agent_name"])},
            {"name": "memory_add", "description": "Add a memory.", "inputSchema": obj(&["author_id"])},
            {"name": "memory_append_event", "description": "Append an event.", "inputSchema": obj(&["author_id", "category", "payload"])},
            {"name": "event_search", "description": "Search events.", "inputSchema": obj(&[])},
            {"name": "memory_delete", "description": "Soft-delete a memory.", "inputSchema": obj(&["id"])},
        ],
    })
}

/// Seeded chunks as `PublicMemory` knowledge JSON values.
fn chunks_to_items(chunks: &[KlamsChunk]) -> Vec<serde_json::Value> {
    chunks
        .iter()
        .enumerate()
        .map(|(i, c)| {
            serde_json::json!({
                "id": format!("00000000-0000-7000-8000-{:012}", i),
                "kind": "knowledge",
                "text": c.text,
                "source_path": c.source_path,
                "tags": ["seed"],
                "author": {"id": "00000000-0000-7000-8000-000000000aaa", "agent_name": "klams-scanner"},
                "created_at": "2026-06-01T00:00:00Z",
                "updated_at": "2026-06-01T00:00:00Z",
            })
        })
        .collect()
}

/// Public accessor for the serialized seeded `PublicMemory` payload — lets
/// tests measure realistic `memory_search` output size (FR-006 cap gate).
pub fn public_memory_json(chunks: &[KlamsChunk]) -> String {
    serde_json::to_string(&chunks_to_items(chunks)).expect("serialize PublicMemory items")
}

/// A running fake klams MCP server. Dropping it shuts down the server and its
/// runtime.
pub struct FakeKlams {
    rt: tokio::runtime::Runtime,
    server: MockServer,
    state: std::sync::Arc<std::sync::Mutex<KlamsState>>,
}

impl FakeKlams {
    /// Start a fake klams that requires `bearer_token` and seeds `memory_search`
    /// with `chunks`.
    pub fn start(bearer_token: &str, chunks: &[KlamsChunk]) -> Self {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build fixture runtime");
        let server = rt.block_on(MockServer::start());

        let state = std::sync::Arc::new(std::sync::Mutex::new(KlamsState::default()));
        let responder = McpResponder {
            session_id: "klams-test-session".to_string(),
            seeded: chunks_to_items(chunks),
            state: state.clone(),
        };

        // POST /mcp — guarded by the exact bearer header. A request without it
        // does not match and falls through to wiremock's 404, so the handshake
        // fails: the bearer is genuinely required.
        rt.block_on(
            server.register(
                Mock::given(method("POST"))
                    .and(path("/mcp"))
                    .and(header(
                        "authorization",
                        format!("Bearer {bearer_token}").as_str(),
                    ))
                    .respond_with(responder),
            ),
        );

        // GET /mcp — the client's background SSE attempt. 405 → the client
        // records "server does not support SSE" and proceeds over plain JSON.
        rt.block_on(
            server.register(
                Mock::given(method("GET"))
                    .and(path("/mcp"))
                    .respond_with(ResponseTemplate::new(405)),
            ),
        );

        Self { rt, server, state }
    }

    /// The `url:` to put in an `mcp-servers.yaml` http entry.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.server.uri())
    }

    /// All requests received so far (for asserting the bearer reached the wire).
    pub fn received_requests(&self) -> Vec<wiremock::Request> {
        self.rt
            .block_on(self.server.received_requests())
            .unwrap_or_default()
    }

    /// `register_author` argument objects seen, in call order.
    pub fn registrations(&self) -> Vec<serde_json::Value> {
        self.state.lock().unwrap().registrations.clone()
    }

    /// The `author_id` carried by every write, in order — for attribution
    /// assertions (all must equal [`FAKE_AUTHOR_ID`]).
    pub fn write_author_ids(&self) -> Vec<String> {
        self.state.lock().unwrap().write_author_ids.clone()
    }

    /// Count of stored events (turn records).
    pub fn event_count(&self) -> usize {
        self.state.lock().unwrap().events.len()
    }

    /// Make subsequent writes fail with this `(code, message)` envelope —
    /// simulates the klams maintenance window / embedding-down paths.
    pub fn reject_writes(&self, code: &str, message: &str) {
        self.state.lock().unwrap().reject_writes = Some((code.to_string(), message.to_string()));
    }
}

/// Reserve an ephemeral TCP port, then drop the listener — the returned
/// `…/v1` URL is guaranteed to refuse connections for the test's duration.
/// Used to simulate a dead backend deterministically (no live-port races).
pub fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
}

/// Like [`dead_endpoint`] but shaped as an MCP `/mcp` URL — a guaranteed-dead
/// HTTP MCP server for degradation tests.
pub fn dead_mcp_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/mcp")
}

/// Write a models.yaml in `dir` with a single TRT-LLM model pointed at the
/// fake proxy. Returns the config path.
pub fn write_trtllm_models_yaml(
    dir: &std::path::Path,
    id: &str,
    served_name: &str,
    endpoint: &str,
) -> std::path::PathBuf {
    let path = dir.join("models.yaml");
    let yaml = format!(
        "models:\n  - id: {id}\n    provider: trtllm\n    served_name: {served_name}\n    endpoint: {endpoint}\n    default: true\n",
    );
    std::fs::write(&path, yaml).expect("write models.yaml");
    path
}
