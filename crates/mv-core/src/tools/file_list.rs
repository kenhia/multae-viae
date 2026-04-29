use rig::tool::ToolError;
use rig::tool_macro as rig_tool;

use super::MAX_TOOL_OUTPUT_CHARS;
use super::truncate_output;

#[tracing::instrument(level = "info", skip(), fields(tool.name = "file_list"))]
#[rig_tool(
    description = "List the contents of a directory",
    params(path = "Directory path to list (default: current directory)")
)]
pub fn file_list(path: Option<String>) -> Result<String, ToolError> {
    let dir = path.unwrap_or_else(|| ".".to_string());

    let entries =
        std::fs::read_dir(&dir).map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Cannot read directory '{dir}': {e}").into()
        })?;

    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Error reading entry in '{dir}': {e}").into()
        })?;
        let name = entry.file_name().to_string_lossy().to_string();
        let suffix = if entry.path().is_dir() { "/" } else { "" };
        names.push(format!("{name}{suffix}"));
    }

    names.sort();
    let output = names.join("\n");
    Ok(truncate_output(&output, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_current_directory() {
        let result = file_list(None).unwrap();
        // Current dir should have some content
        assert!(!result.is_empty());
    }

    #[test]
    fn list_specific_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("b.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();

        let result = file_list(Some(dir.path().to_string_lossy().to_string())).unwrap();
        assert!(result.contains("a.txt"));
        assert!(result.contains("b.txt"));
        assert!(result.contains("subdir/"));
    }

    #[test]
    fn list_missing_directory() {
        let result = file_list(Some("/nonexistent/path/xyz".to_string()));
        assert!(result.is_err());
    }

    #[test]
    fn list_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let result = file_list(Some(dir.path().to_string_lossy().to_string())).unwrap();
        assert!(result.is_empty());
    }
}
