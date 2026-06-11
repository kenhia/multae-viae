use std::time::Duration;

/// Result of a TRT-LLM server health check.
#[derive(Debug)]
pub enum HealthCheckResult {
    /// Server responded with 200 OK.
    Healthy,
    /// Server responded with a non-200 status.
    Unhealthy { status: u16, body: String },
    /// Server is unreachable (connection failed or timed out).
    Unreachable { error: String },
}

/// Derive the health URL from a model endpoint.
///
/// Strips `/v1` suffix (if present) and appends `/health`.
/// Example: `http://localhost:8000/v1` → `http://localhost:8000/health`
fn health_url(endpoint: &str) -> String {
    let base = endpoint.trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/health")
}

/// Check whether a TRT-LLM server is healthy.
///
/// Sends a GET request to the `/health` endpoint with a 2-second timeout.
pub async fn check_health(endpoint: &str) -> HealthCheckResult {
    let url = health_url(endpoint);

    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return HealthCheckResult::Unreachable {
                error: e.to_string(),
            };
        }
    };

    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status().as_u16();
            if status == 200 {
                HealthCheckResult::Healthy
            } else {
                let body = resp.text().await.unwrap_or_default();
                HealthCheckResult::Unhealthy { status, body }
            }
        }
        Err(e) => HealthCheckResult::Unreachable {
            error: e.to_string(),
        },
    }
}

/// Derive the `/v1/models` URL from a model endpoint.
///
/// The endpoint already carries the `/v1` suffix, so this just appends
/// `/models`: `http://localhost:8003/v1` → `http://localhost:8003/v1/models`.
fn models_url(endpoint: &str) -> String {
    format!("{}/models", endpoint.trim_end_matches('/'))
}

/// Whether `model_name` appears in an OpenAI `/v1/models` list response body.
///
/// Parses the `{"data":[{"id":"..."}]}` shape leniently; a body that does not
/// parse yields `false`.
fn model_in_list(body: &str, model_name: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("data").and_then(|d| d.as_array()).cloned())
        .map(|data| {
            data.iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str()))
                .any(|id| id == model_name)
        })
        .unwrap_or(false)
}

/// Streaming preflight: report whether the proxy currently serves `model_name`.
///
/// rig's streaming layer swallows a proxy 502 (it logs an SSE parse error and
/// ends the turn with empty output), so the buffered 502 → `ModelNotLoaded`
/// mapping cannot fire on the streaming path. Querying `/v1/models` first lets
/// the streaming path surface the same `just load` hint for an unloaded model.
///
/// Returns `Some(true)`/`Some(false)` when the list was fetched, or `None` when
/// it could not be determined (request or parse failure) — callers should
/// proceed rather than block streaming on a flaky preflight.
pub async fn served_model_present(endpoint: &str, model_name: &str) -> Option<bool> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .ok()?;
    let resp = client.get(models_url(endpoint)).send().await.ok()?;
    if resp.status().as_u16() != 200 {
        return None;
    }
    let body = resp.text().await.ok()?;
    Some(model_in_list(&body, model_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_url_strips_v1() {
        assert_eq!(
            health_url("http://localhost:8000/v1"),
            "http://localhost:8000/health"
        );
    }

    #[test]
    fn health_url_strips_v1_trailing_slash() {
        assert_eq!(
            health_url("http://localhost:8000/v1/"),
            "http://localhost:8000/health"
        );
    }

    #[test]
    fn health_url_no_v1() {
        assert_eq!(
            health_url("http://localhost:8000"),
            "http://localhost:8000/health"
        );
    }

    #[test]
    fn health_url_custom_port() {
        assert_eq!(
            health_url("http://gpu-server:9000/v1"),
            "http://gpu-server:9000/health"
        );
    }

    #[tokio::test]
    async fn check_health_unreachable() {
        // Connect to a port that should not be listening
        let result = check_health("http://127.0.0.1:19999/v1").await;
        assert!(matches!(result, HealthCheckResult::Unreachable { .. }));
    }

    #[test]
    fn models_url_appends_models() {
        assert_eq!(
            models_url("http://localhost:8003/v1"),
            "http://localhost:8003/v1/models"
        );
        assert_eq!(
            models_url("http://localhost:8003/v1/"),
            "http://localhost:8003/v1/models"
        );
    }

    #[test]
    fn model_in_list_finds_served_model() {
        let body = r#"{"object":"list","data":[
            {"id":"llama-3_1-8b-awq","object":"model"},
            {"id":"llama-3_1-8b-fp8","object":"model"}
        ]}"#;
        assert!(model_in_list(body, "llama-3_1-8b-fp8"));
        assert!(model_in_list(body, "llama-3_1-8b-awq"));
    }

    #[test]
    fn model_in_list_rejects_absent_model() {
        let body = r#"{"object":"list","data":[{"id":"llama-3_1-8b-awq","object":"model"}]}"#;
        // `llama-fp8` (the bare alias, never a served_name) must not match.
        assert!(!model_in_list(body, "llama-fp8"));
        assert!(!model_in_list(body, "llama-3_1-8b-fp8"));
    }

    #[test]
    fn model_in_list_handles_unparseable_body() {
        assert!(!model_in_list("not json", "anything"));
        assert!(!model_in_list("{}", "anything"));
    }

    #[tokio::test]
    async fn served_model_present_unreachable_returns_none() {
        // Nothing listening → cannot determine; caller proceeds.
        assert_eq!(
            served_model_present("http://127.0.0.1:19999/v1", "llama-fp8").await,
            None
        );
    }
}
