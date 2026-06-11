//! Per-provider preflight: a cheap "is this backend alive?" probe, decoupled
//! from completion. The fallback router uses it to skip dead locals before
//! burning an agent build; the TRT-LLM call paths use it as their single
//! reachability source (no bespoke health logic in the CLI).
//!
//! [`PreflightStatus::Dead`] carries the exact [`MvError`] the entry would
//! surface, so a caller can return or record it verbatim — the precise hint
//! (`just load <id>`, `trtllm-serve`, "Is Ollama running?") is preserved
//! whether the failure is discovered here or at completion time.

use std::time::Duration;

use crate::trtllm::START_HINT;
use crate::trtllm::health::{HealthCheckResult, check_health, served_model_present};
use crate::{ModelEntry, MvError, Provider};

/// Outcome of a preflight probe.
#[derive(Debug)]
pub enum PreflightStatus {
    /// The backend is reachable (and, where checkable, serving the model).
    Healthy,
    /// The backend is definitively unusable. Carries the error the entry would
    /// surface at completion, for verbatim return/recording.
    Dead(MvError),
    /// Could not be determined — no cheap probe (cloud APIs) or an
    /// indeterminate result. The caller should attempt the completion normally.
    Unknown,
}

/// Probe whether `entry`'s backend is alive at `endpoint` (the resolved
/// endpoint the completion will use — which may be a `--endpoint` override, not
/// `entry.endpoint()`). `timeout` bounds each network request so the router
/// never blocks on a hung host.
pub async fn preflight(entry: &ModelEntry, endpoint: &str, timeout: Duration) -> PreflightStatus {
    match entry.provider {
        Provider::Trtllm => preflight_trtllm(entry, endpoint, timeout).await,
        Provider::Ollama => preflight_ollama(endpoint, timeout).await,
        // Cloud APIs have no cheap liveness probe; let the completion attempt
        // be the source of truth (and `ApiKeyMissing` is caught before dispatch).
        Provider::Openai => PreflightStatus::Unknown,
    }
}

async fn preflight_trtllm(
    entry: &ModelEntry,
    endpoint: &str,
    timeout: Duration,
) -> PreflightStatus {
    match check_health(endpoint, timeout).await {
        HealthCheckResult::Unreachable { error } => {
            PreflightStatus::Dead(MvError::BackendUnreachable {
                endpoint: endpoint.to_string(),
                hint: format!("TRT-LLM server not reachable ({error}). {START_HINT}"),
            })
        }
        HealthCheckResult::Unhealthy { status, body } => {
            PreflightStatus::Dead(MvError::BackendUnreachable {
                endpoint: endpoint.to_string(),
                hint: format!("Server returned {status}: {body}. {START_HINT}"),
            })
        }
        HealthCheckResult::Healthy => {
            // Server is up. If it definitively does not serve this model, the
            // model is unloaded — surface the `just load` hint without a chat
            // attempt. `Some(true)` (serving) and `None` (indeterminate) both
            // mean "proceed".
            match served_model_present(endpoint, entry.model_name(), timeout).await {
                Some(false) => PreflightStatus::Dead(MvError::ModelNotLoaded {
                    model: entry.id.clone(),
                    hint: format!("Run: just load {}", entry.id),
                }),
                _ => PreflightStatus::Healthy,
            }
        }
    }
}

async fn preflight_ollama(endpoint: &str, timeout: Duration) -> PreflightStatus {
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(c) => c,
        Err(_) => return PreflightStatus::Unknown,
    };
    // Any HTTP response means the daemon is up; a transport error means it is
    // not. We don't verify the model is pulled — that surfaces at completion.
    match client.get(endpoint).send().await {
        Ok(_) => PreflightStatus::Healthy,
        Err(e) => PreflightStatus::Dead(MvError::BackendUnreachable {
            endpoint: endpoint.to_string(),
            hint: format!("Is Ollama running? ({e})"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Locality;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TIMEOUT: Duration = Duration::from_secs(2);

    fn entry(provider: Provider, endpoint: &str, served_name: Option<&str>) -> ModelEntry {
        ModelEntry {
            id: "test-model".to_string(),
            provider,
            locality: Some(Locality::Local),
            api_key_env: None,
            endpoint: Some(endpoint.to_string()),
            default: false,
            served_name: served_name.map(str::to_string),
            architecture: None,
            quant: None,
            expected_vram_gb: None,
            stop_sequences: None,
            max_turns: None,
            fallback: None,
        }
    }

    /// An endpoint on a just-freed port: connecting is refused instantly.
    fn dead_endpoint() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        format!("http://127.0.0.1:{port}/v1")
    }

    #[tokio::test]
    async fn openai_is_always_unknown() {
        let e = entry(Provider::Openai, "https://api.openai.com/v1", None);
        assert!(matches!(
            preflight(&e, &e.endpoint(), TIMEOUT).await,
            PreflightStatus::Unknown
        ));
    }

    #[tokio::test]
    async fn trtllm_unreachable_is_dead_with_hint() {
        let e = entry(Provider::Trtllm, &dead_endpoint(), None);
        match preflight(&e, &e.endpoint(), TIMEOUT).await {
            PreflightStatus::Dead(MvError::BackendUnreachable { hint, .. }) => {
                assert!(hint.contains("trtllm-serve"), "got: {hint}");
            }
            other => panic!("expected Dead(BackendUnreachable), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ollama_unreachable_is_dead() {
        let e = entry(Provider::Ollama, &dead_endpoint(), None);
        assert!(matches!(
            preflight(&e, &e.endpoint(), TIMEOUT).await,
            PreflightStatus::Dead(MvError::BackendUnreachable { .. })
        ));
    }

    #[tokio::test]
    async fn trtllm_healthy_and_served_is_healthy() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [{"id": "served-x", "object": "model"}],
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1", server.uri());
        let e = entry(Provider::Trtllm, &endpoint, Some("served-x"));
        assert!(matches!(
            preflight(&e, &e.endpoint(), TIMEOUT).await,
            PreflightStatus::Healthy
        ));
    }

    #[tokio::test]
    async fn trtllm_healthy_but_model_absent_is_dead_not_loaded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "object": "list",
                "data": [{"id": "some-other-model", "object": "model"}],
            })))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1", server.uri());
        let e = entry(Provider::Trtllm, &endpoint, Some("served-x"));
        match preflight(&e, &e.endpoint(), TIMEOUT).await {
            PreflightStatus::Dead(MvError::ModelNotLoaded { model, hint }) => {
                assert_eq!(model, "test-model");
                assert!(hint.contains("just load test-model"), "got: {hint}");
            }
            other => panic!("expected Dead(ModelNotLoaded), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn trtllm_healthy_with_indeterminate_models_proceeds() {
        // /health 200 but /v1/models not mounted (404) → can't tell → proceed.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(ResponseTemplate::new(200).set_body_string("OK"))
            .mount(&server)
            .await;

        let endpoint = format!("{}/v1", server.uri());
        let e = entry(Provider::Trtllm, &endpoint, Some("served-x"));
        assert!(matches!(
            preflight(&e, &e.endpoint(), TIMEOUT).await,
            PreflightStatus::Healthy
        ));
    }
}
