pub mod prompt;
pub mod workflow;

// MCP connection lifecycle now lives in `mv_core::mcp::manager::McpManager`
// (connect → use → shutdown), shared with mv-server. The CLI drives it in
// one-shot mode: `McpManager::connect(...).await?` then `manager.shutdown()`.
