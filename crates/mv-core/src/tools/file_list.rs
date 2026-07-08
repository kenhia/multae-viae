use rig::tool::ToolError;
use rig::tool_macro as rig_tool;

use super::MAX_TOOL_OUTPUT_CHARS;
use super::truncate_output;

#[tracing::instrument(level = "info", skip(), fields(tool.name = "file_list"))]
#[rig_tool(
    description = "List the contents of a directory",
    params(path = "Directory path to list; use '.' for current directory")
)]
pub async fn file_list(path: String) -> Result<String, ToolError> {
    super::tool_policy()
        .check_path(&path)
        .map_err(|e| ToolError::ToolCallError(e.into()))?;
    let dir = if path.is_empty() { "." } else { &path };

    // Async I/O: the daemon's runtime threads must not block on a slow listing.
    let mut entries = tokio::fs::read_dir(dir).await.map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Cannot read directory '{dir}': {e}").into()
        },
    )?;

    let mut names: Vec<String> = Vec::new();
    while let Some(entry) =
        entries
            .next_entry()
            .await
            .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
                format!("Error reading entry in '{dir}': {e}").into()
            })?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let suffix = if is_dir { "/" } else { "" };
        names.push(format!("{name}{suffix}"));
    }

    names.sort();
    let output = names.join("\n");
    Ok(truncate_output(&output, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_current_directory() {
        let result = file_list(".".to_string()).await.unwrap();
        // Current dir should have some content
        assert!(!result.is_empty());
    }

    #[tokio::test]
    async fn list_specific_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let result = file_list(dir.path().to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(result.contains("a.txt"));
        assert!(result.contains("b.txt"));
        assert!(result.contains("subdir/"));
    }

    #[tokio::test]
    async fn list_missing_directory() {
        let result = file_list("/nonexistent/path/xyz".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = file_list(dir.path().to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(result.is_empty());
    }
}
