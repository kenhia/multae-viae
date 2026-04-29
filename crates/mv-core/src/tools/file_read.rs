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
pub fn file_read(path: String) -> Result<String, ToolError> {
    let contents = std::fs::read_to_string(&path).map_err(
        |e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Cannot read file '{path}': {e}").into()
        },
    )?;

    Ok(truncate_output(&contents, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, "hello world").unwrap();

        let result = file_read(file_path.to_string_lossy().to_string()).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn read_missing_file() {
        let result = file_read("/nonexistent/file.txt".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn read_truncates_large_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("big.txt");
        let big_content = "x".repeat(20_000);
        std::fs::write(&file_path, &big_content).unwrap();

        let result = file_read(file_path.to_string_lossy().to_string()).unwrap();
        assert!(result.contains("[truncated"));
        assert!(result.len() < big_content.len());
    }
}
