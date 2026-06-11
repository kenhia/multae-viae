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

use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

/// Reserve an ephemeral TCP port, then drop the listener — the returned
/// `…/v1` URL is guaranteed to refuse connections for the test's duration.
/// Used to simulate a dead backend deterministically (no live-port races).
pub fn dead_endpoint() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/v1")
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
