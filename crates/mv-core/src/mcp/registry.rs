//! Tool collision detection and resolution for merged built-in + MCP tool sets.
//!
//! Built-in tools always take precedence over MCP tools with the same name.
//! MCP tool parameter schemas are cleaned (strip `$schema`) for Ollama compatibility.

use rig::completion::ToolDefinition;
use rig::tool::server::ToolServerHandle;
use rig::tool::{ToolDyn, ToolError};
use rig::wasm_compat::WasmBoxedFuture;
use tracing::{info, warn};

/// Names of all built-in tools registered on the ToolServer.
pub const BUILT_IN_TOOL_NAMES: &[&str] = &["file_list", "file_read", "shell_exec", "http_get"];

/// MCP tool names that semantically overlap with built-in tools and should
/// be skipped to avoid confusing the model with near-duplicate options.
const SEMANTIC_OVERLAPS: &[&str] = &[
    // Overlaps with file_list
    "list_directory",
    "list_directory_with_sizes",
    "directory_tree",
    "list_allowed_directories",
    // Overlaps with file_read
    "read_file",
    "read_text_file",
    "read_media_file",
    "read_multiple_files",
    "get_file_info",
];

/// An MCP tool wrapper that strips `$schema` from parameter definitions
/// and delegates calls to the MCP-specific ToolServerHandle.
struct CleanedMcpTool {
    tool_name: String,
    description: String,
    parameters: serde_json::Value,
    mcp_handle: ToolServerHandle,
}

impl ToolDyn for CleanedMcpTool {
    fn name(&self) -> String {
        self.tool_name.clone()
    }

    fn definition<'a>(&'a self, _prompt: String) -> WasmBoxedFuture<'a, ToolDefinition> {
        Box::pin(async move {
            ToolDefinition {
                name: self.tool_name.clone(),
                description: self.description.clone(),
                parameters: self.parameters.clone(),
            }
        })
    }

    fn call<'a>(&'a self, args: String) -> WasmBoxedFuture<'a, Result<String, ToolError>> {
        Box::pin(async move {
            self.mcp_handle
                .call_tool(&self.tool_name, &args)
                .await
                .map_err(|e| ToolError::ToolCallError(e.to_string().into()))
        })
    }
}

/// Strip `$schema` from a JSON Schema value (non-recursive, top-level only).
fn strip_schema_field(mut schema: serde_json::Value) -> serde_json::Value {
    if let Some(obj) = schema.as_object_mut() {
        obj.remove("$schema");
    }
    schema
}

/// After MCP servers connect to `mcp_handle`, register cleaned wrapper tools
/// on `agent_handle`. Built-in tool names are skipped (built-in takes precedence).
///
/// Returns the number of MCP tools registered on the agent handle.
pub async fn register_mcp_tools(
    mcp_handle: &ToolServerHandle,
    agent_handle: &ToolServerHandle,
) -> usize {
    let mcp_defs = match mcp_handle.get_tool_defs(None).await {
        Ok(defs) => defs,
        Err(e) => {
            warn!(error = %e, "failed to list MCP tool definitions");
            return 0;
        }
    };

    let mut registered = 0;
    for def in mcp_defs {
        // Skip MCP tools that shadow built-in tools (exact name match)
        if BUILT_IN_TOOL_NAMES.contains(&def.name.as_str()) {
            warn!(
                tool = %def.name,
                "MCP tool shadows built-in tool; keeping built-in, skipping MCP version"
            );
            continue;
        }

        // Skip MCP tools that semantically overlap with built-in tools
        if SEMANTIC_OVERLAPS.contains(&def.name.as_str()) {
            info!(
                tool = %def.name,
                "MCP tool overlaps with built-in tool; skipping to reduce tool count"
            );
            continue;
        }

        let wrapper = CleanedMcpTool {
            tool_name: def.name.clone(),
            description: def.description.clone(),
            parameters: strip_schema_field(def.parameters),
            mcp_handle: mcp_handle.clone(),
        };

        if let Err(e) = agent_handle.add_tool(wrapper).await {
            warn!(tool = %def.name, error = %e, "failed to register cleaned MCP tool");
        } else {
            registered += 1;
        }
    }

    info!(count = registered, "MCP tools registered on agent");
    registered
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::tool::server::ToolServer;

    #[test]
    fn builtin_tool_names_are_known() {
        assert!(BUILT_IN_TOOL_NAMES.contains(&"file_list"));
        assert!(BUILT_IN_TOOL_NAMES.contains(&"file_read"));
        assert!(BUILT_IN_TOOL_NAMES.contains(&"shell_exec"));
        assert!(BUILT_IN_TOOL_NAMES.contains(&"http_get"));
        assert_eq!(BUILT_IN_TOOL_NAMES.len(), 4);
    }

    #[test]
    fn strip_schema_removes_dollar_schema() {
        let schema = serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            }
        });
        let cleaned = strip_schema_field(schema);
        assert!(cleaned.get("$schema").is_none());
        assert_eq!(cleaned.get("type").unwrap(), "object");
        assert!(cleaned.get("properties").is_some());
    }

    #[test]
    fn strip_schema_noop_when_absent() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {}
        });
        let cleaned = strip_schema_field(schema.clone());
        assert_eq!(cleaned, schema);
    }

    #[tokio::test]
    async fn register_skips_builtin_names() {
        // Simulate MCP handle with a tool named "file_list" (conflicts with built-in)
        let mcp_server = ToolServer::new().tool(crate::tools::file_list::FileList);
        let mcp_handle = mcp_server.run();

        let agent_server = ToolServer::new();
        let agent_handle = agent_server.run();

        let count = register_mcp_tools(&mcp_handle, &agent_handle).await;
        assert_eq!(count, 0, "should skip tools that shadow built-ins");
    }

    #[tokio::test]
    async fn register_empty_mcp_handle() {
        let mcp_handle = ToolServer::new().run();
        let agent_handle = ToolServer::new().run();

        let count = register_mcp_tools(&mcp_handle, &agent_handle).await;
        assert_eq!(count, 0);
    }
}
