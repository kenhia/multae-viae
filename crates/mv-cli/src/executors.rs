//! Concrete `PromptExecutor`/`ToolExecutor` implementations bridging the
//! workflow engine (mv-core, rig-free) to rig.

use std::collections::HashSet;

use rig::tool::server::ToolServerHandle;

use crate::providers::{GenParams, complete_chain};

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
        // Build the candidate chain from the step's preference list: each
        // preferred id contributes itself followed by its own configured
        // `fallback` entries, deduped, preserving order. So a bare `model:`
        // behaves exactly like the CLI path (model + its fallback), and a
        // `prefer:` list strings several such chains together.
        let mut chain: Vec<(&mv_core::ModelEntry, String)> = Vec::new();
        let mut seen: HashSet<&str> = HashSet::new();
        for id in models {
            // A typo'd model must fail loudly — silently substituting the
            // default would run the step elsewhere and report success.
            let entry =
                self.registry
                    .get(id)
                    .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                        model: id.clone(),
                        available: self.registry.available_ids().join(", "),
                    })?;
            if seen.insert(entry.id.as_str()) {
                chain.push((entry, entry.endpoint()));
            }
            if let Some(fallback_ids) = &entry.fallback {
                for fid in fallback_ids {
                    if let Some(fe) = self.registry.get(fid)
                        && seen.insert(fe.id.as_str())
                    {
                        chain.push((fe, fe.endpoint()));
                    }
                }
            }
        }

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
