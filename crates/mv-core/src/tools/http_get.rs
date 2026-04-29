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
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(HTTP_TIMEOUT_SECS))
        .build()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to create HTTP client: {e}").into()
        })?;

    let response =
        client
            .get(&url)
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
        let result = http_get("http://192.0.2.1:1/test".to_string()).await;
        assert!(result.is_err());
    }
}
