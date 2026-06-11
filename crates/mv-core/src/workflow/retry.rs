//! Tool-step error handling: the skip/fail/retry strategies.
//!
//! Retry semantics: only [`MvError::is_retryable`] errors are re-attempted —
//! a permanent failure (validation error, missing input) fails immediately
//! regardless of `on_error: retry`. Note that retrying re-executes the tool:
//! for side-effecting tools (`shell_exec`, `http_get` against non-idempotent
//! endpoints) every attempt runs the side effect again.

use tracing::warn;

use super::engine::ToolExecutor;
use super::types::{ErrorAction, ToolStep};
use crate::MvError;

/// Upper bound on a single retry backoff sleep — exponential growth past this
/// would stall a workflow for minutes.
const MAX_RETRY_DELAY_MS: u64 = 30_000;

/// Default base delay between attempts when the retry config does not set one.
const DEFAULT_BASE_DELAY_MS: u64 = 100;

pub(super) async fn execute_tool_with_error_handling<T: ToolExecutor>(
    ts: &ToolStep,
    rendered_inputs: &std::collections::HashMap<String, serde_json::Value>,
    tool_executor: &T,
) -> Result<String, MvError> {
    let execute = || async { tool_executor.execute_tool(&ts.tool, rendered_inputs).await };

    match &ts.on_error {
        ErrorAction::Fail => execute().await.map_err(|e| MvError::WorkflowStepError {
            step: ts.id.clone(),
            source: Box::new(e),
        }),
        ErrorAction::Skip => match execute().await {
            Ok(output) => Ok(output),
            Err(e) => {
                warn!(
                    step_id = %ts.id,
                    tool = %ts.tool,
                    error = %e,
                    "tool failed, skipping"
                );
                Ok(String::new())
            }
        },
        ErrorAction::Retry => {
            let retry = ts.retry.as_ref();
            let max_attempts = retry.map_or(3, |r| r.max_attempts);
            // Defense in depth: validation rejects max_attempts == 0, but the
            // engine is a library API callable without prior validation — a
            // zero here must be an error, never a panic.
            if max_attempts == 0 {
                return Err(MvError::WorkflowStepFailed {
                    step: ts.id.clone(),
                    details: "invalid retry config: max_attempts must be at least 1".to_string(),
                });
            }
            let is_exponential =
                retry.is_none_or(|r| r.backoff == super::types::BackoffStrategy::Exponential);
            let base_delay = retry
                .and_then(|r| r.base_delay_ms)
                .unwrap_or(DEFAULT_BASE_DELAY_MS);

            for attempt in 1..=max_attempts {
                match execute().await {
                    Ok(output) => return Ok(output),
                    Err(e) => {
                        // Permanent failures don't improve with repetition —
                        // fail now, preserving the typed source.
                        if !e.is_retryable() {
                            return Err(MvError::WorkflowStepError {
                                step: ts.id.clone(),
                                source: Box::new(e),
                            });
                        }
                        if attempt == max_attempts {
                            return Err(MvError::WorkflowStepFailed {
                                step: ts.id.clone(),
                                details: format!(
                                    "tool '{}' failed after {max_attempts} attempts: {e}",
                                    ts.tool
                                ),
                            });
                        }
                        let delay_ms = if is_exponential {
                            base_delay
                                .saturating_mul(2u64.saturating_pow(attempt - 1))
                                .min(MAX_RETRY_DELAY_MS)
                        } else {
                            base_delay
                        };
                        warn!(
                            step_id = %ts.id,
                            tool = %ts.tool,
                            attempt = attempt,
                            max_attempts = max_attempts,
                            "tool failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    }
                }
            }
            // The loop always returns on the final attempt (max_attempts >= 1).
            Err(MvError::WorkflowStepFailed {
                step: ts.id.clone(),
                details: "retry loop ended without a result".to_string(),
            })
        }
    }
}
