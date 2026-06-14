use rig::tool::ToolError;
use rig::tool_macro as rig_tool;

use super::HTTP_TIMEOUT_SECS;
use super::MAX_TOOL_OUTPUT_CHARS;
use super::truncate_output;

#[tracing::instrument(level = "info", skip(), fields(tool.name = "http_get"))]
#[rig_tool(
    description = "Fetch a URL via HTTP GET and return the response body",
    params(url = "URL to fetch"),
    required(url)
)]
pub async fn http_get(url: String) -> Result<String, ToolError> {
    super::tool_policy()
        .check_url(&url)
        .map_err(|e| ToolError::ToolCallError(e.into()))?;

    // Shared pooled client; the tool's fetch timeout is applied per request.
    let response = crate::http::client()
        .get(&url)
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("HTTP request to '{url}' failed: {e}").into()
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(ToolError::ToolCallError(
            format!("HTTP {status} from '{url}'").into(),
        ));
    }

    let body = response
        .text()
        .await
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to read response body from '{url}': {e}").into()
        })?;

    Ok(truncate_output(&body, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_invalid_url() {
        let result = http_get("not-a-url".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_unreachable_host() {
        // Bind to an ephemeral port, then drop the listener: connecting to
        // the freed port is refused instantly, instead of waiting out the
        // 30s request timeout against a blackhole address.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let result = http_get(format!("http://127.0.0.1:{port}/test")).await;
        assert!(result.is_err());
    }
}
