use rig::tool::ToolError;
use rig::tool_macro as rig_tool;

use super::MAX_TOOL_OUTPUT_CHARS;
use super::truncate_output;

#[tracing::instrument(level = "info", skip(), fields(tool.name = "file_read"))]
#[rig_tool(
    description = "Read the contents of a file",
    params(path = "File path to read"),
    required(path)
)]
pub async fn file_read(path: String) -> Result<String, ToolError> {
    super::tool_policy()
        .check_path(&path)
        .map_err(|e| ToolError::ToolCallError(e.into()))?;
    // Async I/O: the daemon's runtime threads must not block on a slow read.
    let contents = tokio::fs::read_to_string(&path).await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Cannot read file '{path}': {e}").into()
        },
    )?;

    Ok(truncate_output(&contents, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let result = file_read(file_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(result, "hello world");
    }

    #[tokio::test]
    async fn read_missing_file() {
        let result = file_read("/nonexistent/file.txt".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn read_truncates_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("big.txt");
        let big_content = "x".repeat(20_000);
        std::fs::write(&file_path, &big_content).unwrap();

        let result = file_read(file_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(result.contains("[truncated"));
        assert!(result.len() < big_content.len());
    }
}
