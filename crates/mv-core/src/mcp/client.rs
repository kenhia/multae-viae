use rig::tool::rmcp::McpClientHandler;
use rig::tool::server::ToolServerHandle;
use rmcp::model::{ClientCapabilities, ClientInfo, Implementation};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::TokioChildProcess;
use tracing::{debug, info, warn};

use crate::MvError;
use crate::mcp::config::{McpServerConfig, McpServersConfig, McpTransportType};

/// A running MCP server connection. Drop this to shut down the connection.
pub struct McpConnection {
    pub name: String,
    pub transport_type: McpTransportType,
    #[allow(dead_code)]
    service: RunningService<RoleClient, McpClientHandler>,
}

impl std::fmt::Debug for McpConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConnection")
            .field("name", &self.name)
            .field("transport_type", &self.transport_type)
            .finish_non_exhaustive()
    }
}

impl McpConnection {
    /// Gracefully shut down the MCP connection.
    pub async fn shutdown(self) {
        info!(server = %self.name, "shutting down MCP connection");
        if let Err(e) = self.service.cancel().await {
            warn!(server = %self.name, error = ?e, "error during MCP shutdown");
        }
    }
}

fn client_info() -> ClientInfo {
    ClientInfo::new(
        ClientCapabilities::default(),
        Implementation::new("mv-cli", env!("CARGO_PKG_VERSION")),
    )
}

/// Connect to a stdio-based MCP server.
#[tracing::instrument(skip(handle), fields(mcp.server.name = %config.name, mcp.transport = "stdio"))]
pub async fn connect_stdio(
    config: &McpServerConfig,
    handle: ToolServerHandle,
) -> Result<McpConnection, MvError> {
    let command = config
        .command
        .as_ref()
        .ok_or_else(|| MvError::McpServerError {
            server: config.name.clone(),
            details: "stdio transport requires 'command'".to_string(),
        })?;

    info!(server = %config.name, command = %command, "connecting to MCP server via stdio");

    let mut cmd = tokio::process::Command::new(command);
    cmd.args(&config.args);
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let child = TokioChildProcess::new(cmd).map_err(|e| MvError::McpServerError {
        server: config.name.clone(),
        details: format!("failed to spawn process: {e}"),
    })?;

    let handler = McpClientHandler::new(client_info(), handle);
    let service = handler
        .connect(child)
        .await
        .map_err(|e| MvError::McpServerError {
            server: config.name.clone(),
            details: format!("handshake failed: {e}"),
        })?;

    info!(server = %config.name, "MCP server connected");

    Ok(McpConnection {
        name: config.name.clone(),
        transport_type: McpTransportType::Stdio,
        service,
    })
}

/// Connect to an HTTP-based MCP server.
#[tracing::instrument(skip(handle), fields(mcp.server.name = %config.name, mcp.transport = "http"))]
pub async fn connect_http(
    config: &McpServerConfig,
    handle: ToolServerHandle,
) -> Result<McpConnection, MvError> {
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };

    let url = config.url.as_ref().ok_or_else(|| MvError::McpServerError {
        server: config.name.clone(),
        details: "http transport requires 'url'".to_string(),
    })?;

    info!(server = %config.name, url = %url, "connecting to MCP server via HTTP");

    let http_config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
    let transport = StreamableHttpClientTransport::with_client(reqwest::Client::new(), http_config);

    let handler = McpClientHandler::new(client_info(), handle);
    let service = handler
        .connect(transport)
        .await
        .map_err(|e| MvError::McpServerError {
            server: config.name.clone(),
            details: format!("connection failed: {e}"),
        })?;

    info!(server = %config.name, "MCP server connected via HTTP");

    Ok(McpConnection {
        name: config.name.clone(),
        transport_type: McpTransportType::Http,
        service,
    })
}

/// Connect to all configured MCP servers. Failures are logged and skipped.
#[tracing::instrument(skip(handle), fields(mcp.server.count = config.servers.len()))]
pub async fn connect_all_servers(
    config: &McpServersConfig,
    handle: ToolServerHandle,
) -> Vec<McpConnection> {
    let mut connections = Vec::new();

    for server in &config.servers {
        let result = match server.transport {
            McpTransportType::Stdio => connect_stdio(server, handle.clone()).await,
            McpTransportType::Http => connect_http(server, handle.clone()).await,
        };

        match result {
            Ok(conn) => {
                debug!(server = %conn.name, transport = ?conn.transport_type, "MCP server ready");
                connections.push(conn);
            }
            Err(e) => {
                warn!(server = %server.name, error = %e, "MCP server failed to connect");
            }
        }
    }

    info!(count = connections.len(), "MCP servers connected");
    connections
}

/// Gracefully shut down all MCP connections.
#[tracing::instrument(skip(connections), fields(mcp.shutdown.count = connections.len()))]
pub async fn shutdown_all(connections: Vec<McpConnection>) {
    for conn in connections {
        conn.shutdown().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig::tool::server::ToolServer;
    use std::collections::HashMap;

    #[test]
    fn client_info_has_correct_name() {
        let info = client_info();
        assert_eq!(info.client_info.name, "mv-cli");
    }

    #[tokio::test]
    async fn connect_stdio_spawn_failure() {
        let handle = ToolServer::new().run();
        let config = McpServerConfig {
            name: "bad-server".to_string(),
            transport: McpTransportType::Stdio,
            command: Some("/nonexistent/binary/that/does/not/exist".to_string()),
            args: vec![],
            env: HashMap::new(),
            url: None,
        };

        let result = connect_stdio(&config, handle).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("bad-server"),
            "error should mention server name: {err}"
        );
    }

    #[tokio::test]
    async fn connect_stdio_missing_command() {
        let handle = ToolServer::new().run();
        let config = McpServerConfig {
            name: "no-cmd".to_string(),
            transport: McpTransportType::Stdio,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: None,
        };

        let result = connect_stdio(&config, handle).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("stdio transport requires 'command'"), "{err}");
    }

    #[tokio::test]
    async fn connect_http_missing_url() {
        let handle = ToolServer::new().run();
        let config = McpServerConfig {
            name: "no-url".to_string(),
            transport: McpTransportType::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: None,
        };

        let result = connect_http(&config, handle).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("http transport requires 'url'"), "{err}");
    }

    #[tokio::test]
    async fn connect_http_unreachable() {
        let handle = ToolServer::new().run();
        let config = McpServerConfig {
            name: "dead-server".to_string(),
            transport: McpTransportType::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: Some("http://127.0.0.1:1/mcp".to_string()),
        };

        let result = connect_http(&config, handle).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("dead-server"),
            "error should mention server name: {err}"
        );
    }

    #[tokio::test]
    async fn connect_all_skips_failures() {
        let handle = ToolServer::new().run();
        let config = McpServersConfig {
            servers: vec![
                McpServerConfig {
                    name: "fail1".to_string(),
                    transport: McpTransportType::Stdio,
                    command: Some("/nonexistent".to_string()),
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                },
                McpServerConfig {
                    name: "fail2".to_string(),
                    transport: McpTransportType::Stdio,
                    command: None,
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                },
            ],
        };

        let connections = connect_all_servers(&config, handle).await;
        assert!(
            connections.is_empty(),
            "both servers should fail gracefully"
        );
    }
}
