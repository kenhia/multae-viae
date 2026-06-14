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

// --- Sessions (WS4) ---

#[tokio::test]
async fn session_lifecycle_create_list_delete() {
    let (state, _dir) = state_builtin().await;
    let router = || build_router(state.clone());

    // Create.
    let resp = router()
        .oneshot(post("/v1/sessions", json!({"name": "research"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = body_json(resp).await;
    assert_eq!(body["name"], "research");

    // Duplicate name → 409.
    let resp = router()
        .oneshot(post("/v1/sessions", json!({"name": "research"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    assert_eq!(body_json(resp).await["error"]["code"], "SESSION_EXISTS");

    // List shows it.
    let resp = router().oneshot(get("/v1/sessions")).await.unwrap();
    let body = body_json(resp).await;
    assert!(
        body["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "research")
    );

    // Delete → 204, then a turn on it 404s.
    let resp = router()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/v1/sessions/research")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn turn_on_unknown_session_is_404() {
    let (state, _dir) = state_builtin().await;
    let resp = build_router(state)
        .oneshot(post("/v1/sessions/ghost/turns", json!({"prompt": "hi"})))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert_eq!(body_json(resp).await["error"]["code"], "SESSION_NOT_FOUND");
}

#[tokio::test]
async fn session_turns_carry_context() {
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
                "message": {"role": "assistant", "content": "ack"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
        })))
        .mount(&server)
        .await;

    let registry = trtllm_registry(&format!("{}/v1", server.uri()), "served-x");
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::build(registry, None, dir.path().to_path_buf())
        .await
        .unwrap();
    let router = || build_router(state.clone());

    router()
        .oneshot(post("/v1/sessions", json!({"name": "s1"})))
        .await
        .unwrap();

    // Turn 1 establishes a fact.
    let resp = router()
        .oneshot(post(
            "/v1/sessions/s1/turns",
            json!({"prompt": "My name is Ken"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "{:?}", resp);

    // Turn 2: the second backend request must carry the first turn in history.
    let resp = router()
        .oneshot(post(
            "/v1/sessions/s1/turns",
            json!({"prompt": "What is my name?"}),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The last chat/completions request body should include the prior turn —
    // proof the held agent carried conversation context across turns.
    let chat_reqs: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/chat/completions")
        .collect();
    let last = chat_reqs.last().expect("a chat request was made");
    let body = String::from_utf8_lossy(&last.body);
    assert!(
        body.contains("My name is Ken"),
        "second turn must include prior history; body: {body}"
    );
}

#[tokio::test]
async fn scheduler_fires_a_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("fired.txt");
    // A backend-free workflow that touches a sentinel via the shell_exec tool,
    // so firing is observable without a model.
    let wf = format!(
        "name: touch\nversion: \"1.0\"\ndescription: t\n\nsteps:\n  - id: s\n    name: s\n    type: tool\n    output: o\n    tool: shell_exec\n    inputs:\n      command: \"touch {}\"\n\noutputs:\n  - name: o\n    from: s\n",
        sentinel.display()
    );
    std::fs::write(dir.path().join("touch.yaml"), wf).unwrap();
    // Every second.
    std::fs::write(
        dir.path().join("schedules.yaml"),
        "schedules:\n  - cron: \"* * * * * *\"\n    workflow: touch.yaml\n",
    )
    .unwrap();

    let state = AppState::build(
        mv_core::ModelRegistry::built_in(),
        None,
        dir.path().to_path_buf(),
    )
    .await
    .unwrap();
    let scheds =
        mv_server::scheduler::load_schedules(&dir.path().join("schedules.yaml"), dir.path())
            .unwrap();
    let handles = mv_server::scheduler::spawn(state, scheds);

    // Within a few ticks the sentinel should appear.
    let mut fired = false;
    for _ in 0..30 {
        if sentinel.exists() {
            fired = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    for h in handles {
        h.abort();
    }
    assert!(fired, "scheduled workflow should have fired within ~3s");
}

/// Live restart-recovery (SC-003): a session recorded to klams is recoverable
/// by name after the server state is rebuilt. `#[ignore]`d — requires a
/// reachable klams (`KLAMS_TOKEN`, URL via `KLAMS_URL`) and a model backend
/// (`KLAMS_MODEL`); run via `just test-klams`. Skips cleanly when unconfigured.
#[tokio::test]
#[ignore = "requires a reachable klams (KLAMS_TOKEN) and a model backend (KLAMS_MODEL)"]
async fn session_restart_recovery_live() {
    let Ok(_token) = std::env::var("KLAMS_TOKEN") else {
        eprintln!("skipping: KLAMS_TOKEN not set");
        return;
    };
    let Ok(model) = std::env::var("KLAMS_MODEL") else {
        eprintln!("skipping: KLAMS_MODEL not set");
        return;
    };
    let klams_url =
        std::env::var("KLAMS_URL").unwrap_or_else(|_| "http://kubs0:7777/mcp".to_string());

    let dir = tempfile::tempdir().unwrap();
    // models.yaml: rely on the project default registry by id; write a minimal
    // ollama entry so the named model resolves to a local backend.
    let models = dir.path().join("models.yaml");
    std::fs::write(
        &models,
        format!("models:\n  - id: {model}\n    provider: ollama\n    default: true\n"),
    )
    .unwrap();
    let mcp = dir.path().join("mcp-servers.yaml");
    std::fs::write(
        &mcp,
        format!(
            "servers:\n  - name: klams\n    transport: http\n    url: {klams_url}\n    auth_token_env: KLAMS_TOKEN\n"
        ),
    )
    .unwrap();

    let session_name = "mv-server-restart-test";
    let secret = "the-secret-is-BLUEFISH";

    // First "boot": create the session, record a fact.
    {
        let registry = mv_core::ModelRegistry::load(&models).unwrap();
        let state = AppState::build(
            registry,
            Some(mcp.to_str().unwrap()),
            dir.path().to_path_buf(),
        )
        .await
        .unwrap();
        let router = || build_router(state.clone());
        router()
            .oneshot(post("/v1/sessions", json!({"name": session_name})))
            .await
            .unwrap();
        let resp = router()
            .oneshot(post(
                &format!("/v1/sessions/{session_name}/turns"),
                json!({"prompt": format!("Please remember this for later: {secret}.")}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        state.mcp.shutdown().await;
    }

    // Second "boot": fresh state (the held session is gone). Recreate by name;
    // the first turn should recall the recorded fact from klams.
    {
        let registry = mv_core::ModelRegistry::load(&models).unwrap();
        let state = AppState::build(
            registry,
            Some(mcp.to_str().unwrap()),
            dir.path().to_path_buf(),
        )
        .await
        .unwrap();
        let router = || build_router(state.clone());
        router()
            .oneshot(post("/v1/sessions", json!({"name": session_name})))
            .await
            .unwrap();
        let resp = router()
            .oneshot(post(
                &format!("/v1/sessions/{session_name}/turns"),
                json!({"prompt": "What was the secret I asked you to remember?"}),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        let answer = body["response"].as_str().unwrap_or_default();
        assert!(
            answer.to_lowercase().contains("bluefish"),
            "recalled answer should mention the secret; got: {answer}"
        );
    }
}
