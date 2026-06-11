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
    exec_with_timeout(&command, SHELL_TIMEOUT_SECS).await
}

async fn exec_with_timeout(command: &str, timeout_secs: u64) -> Result<String, ToolError> {
    if command.is_empty() {
        return Err(ToolError::ToolCallError(
            "Command must not be empty".to_string().into(),
        ));
    }

    // kill_on_drop: when the timeout below fires, the future holding the child
    // is dropped — without this the child keeps running as an orphan.
    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Failed to spawn command: {e}").into()
        })?;

    let timeout = std::time::Duration::from_secs(timeout_secs);
    let output = tokio::time::timeout(timeout, child.wait_with_output())
        .await
        .map_err(|_| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Command timed out after {timeout_secs}s (process killed)").into()
        })?
        .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> {
            format!("Command execution failed: {e}").into()
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Label the streams when both are present so the model can tell them
    // apart; pass a single stream through unlabeled.
    let mut result = match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("stdout:\n{stdout}\nstderr:\n{stderr}"),
        (false, true) => stdout.into_owned(),
        (true, false) => stderr.into_owned(),
        (true, true) => String::new(),
    };

    // Always surface a non-zero exit status — a failing command with output
    // must not look like success to the model.
    if !output.status.success() {
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str(&format!("[{}]", output.status));
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
    async fn exec_failed_command_reports_status() {
        let result = shell_exec("false".to_string()).await.unwrap();
        assert!(result.contains("[exit status: 1]"), "got: {result}");
    }

    #[tokio::test]
    async fn exec_failure_with_output_still_reports_status() {
        let result = shell_exec("echo partial; exit 3".to_string())
            .await
            .unwrap();
        assert!(result.contains("partial"), "got: {result}");
        assert!(result.contains("[exit status: 3]"), "got: {result}");
    }

    #[tokio::test]
    async fn exec_labels_streams_when_both_present() {
        let result = shell_exec("echo out; echo err >&2".to_string())
            .await
            .unwrap();
        assert!(result.contains("stdout:\nout"), "got: {result}");
        assert!(result.contains("stderr:\nerr"), "got: {result}");
    }

    #[tokio::test]
    async fn exec_single_stream_is_unlabeled() {
        let result = shell_exec("echo only".to_string()).await.unwrap();
        assert!(!result.contains("stdout:"), "got: {result}");
        assert_eq!(result.trim(), "only");
    }

    #[tokio::test]
    async fn exec_empty_command_rejected() {
        let result = shell_exec("".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn exec_timeout_kills_child_and_errors() {
        let start = std::time::Instant::now();
        let result = exec_with_timeout("sleep 30", 1).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("timed out after 1s"),
            "timeout error expected"
        );
        // Must return promptly at the timeout, not wait out the sleep.
        assert!(start.elapsed() < std::time::Duration::from_secs(5));
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
