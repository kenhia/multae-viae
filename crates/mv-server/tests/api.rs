//! In-process API tests via `tower::ServiceExt::oneshot` — no port binding,
//! no subprocess. A wiremock server stands in for a TRT-LLM backend where a
//! completion is needed; everything else is hermetic against the built-in
//! registry.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use http_body_util::BodyExt;
use mv_server::{AppState, build_router};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn body_json(resp: Response) -> Value {
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

fn post(uri: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

fn get(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}

/// A built-in-registry AppState with a temp (empty) workflows dir.
async fn state_builtin() -> (AppState, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::build(
        mv_core::ModelRegistry::built_in(),
        None,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();
    (state, dir)
}

/// Write a one-model TRT-LLM registry pointed at `endpoint`, load it.
fn trtllm_registry(endpoint: &str, served_name: &str) -> mv_core::ModelRegistry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("models.yaml");
    let yaml = format!(
        "models:\n  - id: test-model\n    provider: trtllm\n    served_name: {served_name}\n    endpoint: {endpoint}\n    default: true\n"
    );
    std::fs::write(&path, yaml).unwrap();
    mv_core::ModelRegistry::load(&path).unwrap()
}

#[tokio::test]
async fn health_reports_ok() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state).oneshot(get("/health")).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["status"], "ok");
    assert_eq!(body["mcp_servers_configured"], 0);
}

#[tokio::test]
async fn models_lists_the_registry() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state)
        .oneshot(get("/v1/models"))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let models = body["models"].as_array().unwrap();
    assert!(!models.is_empty(), "built-in registry has models");
    assert!(
        models.iter().any(|m| m["default"] == true),
        "one model is the default"
    );
    assert!(models.iter().all(|m| m["id"].is_string()));
}

#[tokio::test]
async fn prompt_empty_is_400_empty_prompt() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state)
        .oneshot(post("/v1/prompt", json!({"prompt": "   "})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "EMPTY_PROMPT");
}

#[tokio::test]
async fn prompt_unknown_model_is_404() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state)
        .oneshot(post(
            "/v1/prompt",
            json!({"prompt": "hi", "model": "nope-not-real"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "MODEL_NOT_IN_REGISTRY");
}

#[tokio::test]
async fn prompt_returns_completion_against_backend() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"id": "served-x", "object": "model"}],
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "c1",
            "object": "chat.completion",
            "created": 0,
            "model": "served-x",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello from the backend"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
        })))
        .mount(&server)
        .await;

    let registry = trtllm_registry(&format!("{}/v1", server.uri()), "served-x");
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::build(registry, None, dir.path().to_path_buf())
        .await
        .unwrap();

    let resp = build_router(state)
        .oneshot(post("/v1/prompt", json!({"prompt": "hi"})))
        .await
        .unwrap();
    let status = resp.status();
    let body = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["response"], "Hello from the backend");
    assert_eq!(body["model_used"], "test-model");
}

#[tokio::test]
async fn prompt_not_loaded_model_is_503() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
        .mount(&server)
        .await;
    // /v1/models does NOT list "served-x" → preflight reports the model unloaded.
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [{"id": "something-else", "object": "model"}],
        })))
        .mount(&server)
        .await;

    let registry = trtllm_registry(&format!("{}/v1", server.uri()), "served-x");
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::build(registry, None, dir.path().to_path_buf())
        .await
        .unwrap();

    let resp = build_router(state)
        .oneshot(post("/v1/prompt", json!({"prompt": "hi"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "MODEL_NOT_LOADED");
    assert!(
        body["error"]["hint"].as_str().unwrap().contains("load"),
        "hint mentions loading: {body}"
    );
}

#[tokio::test]
async fn workflow_traversal_is_rejected_400() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state)
        .oneshot(post(
            "/v1/workflows/run",
            json!({"workflow": "../../etc/passwd", "inputs": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "WORKFLOW_PATH_OUTSIDE_ROOT");
}

#[tokio::test]
async fn workflow_runs_a_tool_only_flow() {
    let dir = tempfile::tempdir().unwrap();
    // A backend-free workflow: one built-in tool step, surfaced as output.
    let wf = "name: list-only\nversion: \"1.0\"\ndescription: list a dir\n\nsteps:\n  - id: list\n    name: List\n    type: tool\n    output: file_listing\n    tool: file_list\n    inputs:\n      path: \".\"\n\noutputs:\n  - name: listing\n    from: list\n";
    std::fs::write(dir.path().join("list-only.yaml"), wf).unwrap();

    let state = AppState::build(
        mv_core::ModelRegistry::built_in(),
        None,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();

    let resp = build_router(state)
        .oneshot(post(
            "/v1/workflows/run",
            json!({"workflow": "list-only.yaml", "inputs": {}}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{:?}", resp);
    let body = body_json(resp).await;
    assert_eq!(body["workflow"], "list-only");
    // The tool step ran end-to-end and produced a directory listing (the
    // built-in file_list resolves "." against the process cwd).
    let listing = body["outputs"]["listing"].as_str().unwrap();
    assert!(
        !listing.is_empty(),
        "listing should be non-empty: {listing}"
    );
}
