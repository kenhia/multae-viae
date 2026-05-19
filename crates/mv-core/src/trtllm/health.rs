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
}
