pub mod prompt;
pub mod workflow;

use rig::tool::server::{ToolServer, ToolServerHandle};
use tracing::debug;

/// Load MCP config, connect to all servers on a dedicated MCP handle,
/// then register cleaned tool wrappers on the agent handle.
/// Returns live connections (for shutdown).
pub async fn connect_mcp_servers(
    mcp_config_path: Option<&str>,
    agent_handle: &ToolServerHandle,
) -> Result<Vec<mv_core::mcp::client::McpConnection>, mv_core::MvError> {
    let mcp_config = mv_core::mcp::config::McpServersConfig::resolve(mcp_config_path)?;
    match mcp_config {
        Some(config) => {
            // MCP tools are registered on a separate handle
            let mcp_server = ToolServer::new();
            let mcp_handle = mcp_server.run();

            let connections =
                mv_core::mcp::client::connect_all_servers(&config, mcp_handle.clone()).await;

            // Register cleaned MCP tools (no $schema) on the agent handle
            mv_core::mcp::registry::register_mcp_tools(&mcp_handle, agent_handle).await;

            Ok(connections)
        }
        None => {
            debug!("no MCP servers configured");
            Ok(vec![])
        }
    }
}
