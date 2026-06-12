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

/// Custom wiremock responder implementing the MCP JSON-RPC subset.
struct McpResponder {
    session_id: String,
    tools_list_result: serde_json::Value,
    /// The text content block returned by a `tools/call` — a JSON-serialized
    /// array of `PublicMemory` knowledge items.
    search_result_text: String,
}

impl Respond for McpResponder {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let msg: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or_default();
        let id = msg.get("id").cloned();
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
                "result": self.tools_list_result,
            })),
            "tools/call" => ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": self.search_result_text}],
                    "isError": false,
                },
            })),
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

/// `tools/list` payload advertising the single `memory_search` tool, matching
/// the klams arg schema (`query` required; `top_k`, `kinds`, `tags` optional).
fn memory_search_tool_list() -> serde_json::Value {
    serde_json::json!({
        "tools": [{
            "name": "memory_search",
            "description": "Search the knowledge base (hybrid vector + full-text). \
                            Returns ranked memory items.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "natural-language query"},
                    "top_k": {"type": "integer", "description": "max results (1..50)"},
                },
                "required": ["query"],
            },
        }],
    })
}

/// Serialize seeded chunks as a JSON array of klams `PublicMemory` knowledge
/// items — the wire shape m-v's contract pins.
fn chunks_to_public_memory(chunks: &[KlamsChunk]) -> String {
    let items: Vec<serde_json::Value> = chunks
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
        .collect();
    serde_json::to_string(&items).expect("serialize PublicMemory items")
}

/// Public accessor for the serialized `PublicMemory` payload — lets tests
/// measure realistic `memory_search` output size (FR-006 cap gate).
pub fn public_memory_json(chunks: &[KlamsChunk]) -> String {
    chunks_to_public_memory(chunks)
}

/// A running fake klams MCP server. Dropping it shuts down the server and its
/// runtime.
pub struct FakeKlams {
    rt: tokio::runtime::Runtime,
    server: MockServer,
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

        let responder = McpResponder {
            session_id: "klams-test-session".to_string(),
            tools_list_result: memory_search_tool_list(),
            search_result_text: chunks_to_public_memory(chunks),
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

        Self { rt, server }
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
