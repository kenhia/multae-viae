//! Concrete `PromptExecutor`/`ToolExecutor` implementations bridging the
//! workflow engine (mv-core, rig-free) to rig.

use rig::tool::server::ToolServerHandle;

use crate::providers::{GenParams, build_chain, complete_chain};

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
        models: &[String],
        temperature: Option<f64>,
        max_tokens: Option<u64>,
    ) -> Result<String, mv_core::MvError> {
        // Resolve each preferred id (a typo'd model must fail loudly —
        // silently substituting the default would run the step elsewhere and
        // report success), then expand the seeds through the shared chain
        // builder: each id contributes itself + its own `fallback` entries,
        // deduped. So a bare `model:` behaves exactly like the CLI path, and
        // a `prefer:` list strings several such chains together.
        let mut seeds: Vec<(&mv_core::ModelEntry, String)> = Vec::new();
        for id in models {
            let entry =
                self.registry
                    .get(id)
                    .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                        model: id.clone(),
                        available: self.registry.available_ids().join(", "),
                    })?;
            seeds.push((entry, entry.endpoint()));
        }
        let chain = build_chain(&self.registry, &seeds);

        let params = GenParams {
            temperature,
            max_tokens,
        };
        // The engine only needs the text; `model_used` is recorded on the trace.
        complete_chain(&chain, prompt_text, self.agent_handle.clone(), &params)
            .await
            .map(|outcome| outcome.text)
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
