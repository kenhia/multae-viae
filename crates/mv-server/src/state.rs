//! Shared application state.
//!
//! Built once at startup and cloned into every request (all fields are cheap to
//! clone — `Arc`s and a `ToolServerHandle`). It holds exactly what handlers need
//! to drive the shared `mv_core` runtime: the model registry, the agent tool
//! handle (built-ins + MCP), the MCP lifecycle manager, and the directory
//! workflow files resolve against.

use std::path::PathBuf;
use std::sync::Arc;

use mv_core::ModelRegistry;
use mv_core::MvError;
use mv_core::mcp::manager::McpManager;
use rig::tool::server::{ToolServer, ToolServerHandle};

/// Process-wide state shared across handlers.
#[derive(Clone)]
pub struct AppState {
    /// Resolved model registry (`models.yaml` or built-in defaults).
    pub registry: Arc<ModelRegistry>,
    /// The agent tool handle: built-in tools plus any registered MCP tools.
    pub agent_handle: ToolServerHandle,
    /// Owns the long-lived MCP connections (keep-alive, reconnect, shutdown).
    pub mcp: Arc<McpManager>,
    /// Directory that `POST /v1/workflows/run` resolves workflow names against;
    /// requests cannot escape it (see the path-boundary check in handlers).
    pub workflows_dir: PathBuf,
}

/// Build the agent tool handle with the built-in tools attached — the same set
/// the CLI gives a prompt run. MCP tools register onto this handle separately
/// via the [`McpManager`].
fn build_agent_tools() -> ToolServerHandle {
    ToolServer::new()
        .tool(mv_core::tools::file_list::FileList)
        .tool(mv_core::tools::file_read::FileRead)
        .tool(mv_core::tools::shell_exec::ShellExec)
        .tool(mv_core::tools::http_get::HttpGet)
        .run()
}

impl AppState {
    /// Assemble the state: attach built-in tools, connect MCP servers (which
    /// register their cleaned tools on the same handle), and keep the manager
    /// for its lifecycle. `mcp_config_path` of `None` yields an empty manager.
    pub async fn build(
        registry: ModelRegistry,
        mcp_config_path: Option<&str>,
        workflows_dir: PathBuf,
    ) -> Result<Self, MvError> {
        let agent_handle = build_agent_tools();
        let mcp = McpManager::connect(mcp_config_path, &agent_handle).await?;
        Ok(Self {
            registry: Arc::new(registry),
            agent_handle,
            mcp: Arc::new(mcp),
            workflows_dir,
        })
    }
}
