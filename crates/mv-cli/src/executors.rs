//! Concrete `PromptExecutor`/`ToolExecutor` implementations bridging the
//! workflow engine (mv-core, rig-free) to rig.

use rig::tool::server::ToolServerHandle;

use crate::providers::{GenParams, complete};

/// Prompt executor that routes workflow prompt steps through the shared
/// provider dispatch seam.
pub struct RigPromptExecutor {
    pub registry: mv_core::ModelRegistry,
    pub agent_handle: ToolServerHandle,
}

impl mv_core::workflow::engine::PromptExecutor for RigPromptExecutor {
    async fn execute_prompt(
        &self,
        prompt_text: &str,
        model: &str,
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String, mv_core::MvError> {
        // A typo'd model must fail loudly — silently substituting the default
        // model would run the step elsewhere and report success.
        let entry =
            self.registry
                .get(model)
                .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                    model: model.to_string(),
                    available: self.registry.available_ids().join(", "),
                })?;

        let params = GenParams {
            temperature,
            max_tokens,
        };
        complete(
            entry,
            &entry.endpoint(),
            prompt_text,
            self.agent_handle.clone(),
            &params,
        )
        .await
    }
}

/// Tool executor that runs workflow tool steps against the shared agent
/// ToolServer — the same merged built-in + MCP tool set the agent sees.
pub struct HandleToolExecutor {
    pub handle: ToolServerHandle,
}

impl mv_core::workflow::engine::ToolExecutor for HandleToolExecutor {
    #[tracing::instrument(level = "info", skip(self, inputs), fields(tool.name = %tool_name))]
    async fn execute_tool(
        &self,
        tool_name: &str,
        inputs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, mv_core::MvError> {
        let args = serde_json::to_string(inputs).map_err(|e| mv_core::MvError::ToolCallFailed {
            tool: tool_name.to_string(),
            details: format!("failed to encode inputs: {e}"),
        })?;
        self.handle.call_tool(tool_name, &args).await.map_err(|e| {
            mv_core::MvError::ToolCallFailed {
                tool: tool_name.to_string(),
                details: e.to_string(),
            }
        })
    }
}
