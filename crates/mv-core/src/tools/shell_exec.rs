use rig::tool::ToolError;
use rig::tool_macro as rig_tool;

use super::MAX_TOOL_OUTPUT_CHARS;
use super::SHELL_TIMEOUT_SECS;
use super::truncate_output;

#[tracing::instrument(level = "info", skip(), fields(tool.name = "shell_exec"))]
#[rig_tool(
    description = "Execute a shell command and return its output",
    params(command = "Shell command to execute"),
    required(command)
)]
pub async fn shell_exec(command: String) -> Result<String, ToolError> {
    if command.is_empty() {
        return Err(ToolError::ToolCallError(
            "Command must not be empty".to_string().into(),
        ));
    }

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to spawn command: {e}").into()
        })?;

    let timeout = std::time::Duration::from_secs(SHELL_TIMEOUT_SECS);
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Command timed out after {SHELL_TIMEOUT_SECS}s").into()
        })?
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Command execution failed: {e}").into()
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let mut result = String::new();
    if !stdout.is_empty() {
        result.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
    }

    if !output.status.success() && result.is_empty() {
        result = format!("Command exited with status: {}", output.status);
    }

    Ok(truncate_output(&result, MAX_TOOL_OUTPUT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn exec_successful_command() {
        let result = shell_exec("echo hello".to_string()).await.unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn exec_failed_command() {
        let result = shell_exec("false".to_string()).await.unwrap();
        // Should not error — returns output (possibly empty with status)
        assert!(result.contains("Command exited with status") || result.is_empty());
    }

    #[tokio::test]
    async fn exec_empty_command_rejected() {
        let result = shell_exec("".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn exec_output_truncation() {
        // Generate output larger than MAX_TOOL_OUTPUT_CHARS
        let cmd = format!(
            "python3 -c \"print('x' * {})\" 2>/dev/null || printf '%0.sx' $(seq 1 {})",
            super::MAX_TOOL_OUTPUT_CHARS + 1000,
            super::MAX_TOOL_OUTPUT_CHARS + 1000
        );
        let result = shell_exec(cmd).await.unwrap();
        assert!(result.contains("[truncated"));
    }
}
