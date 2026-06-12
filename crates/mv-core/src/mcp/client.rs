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
    // Suppress MCP server diagnostic output from reaching the user's terminal
    cmd.stderr(std::process::Stdio::null());
    // Suppress npm update notices when spawning npx-based MCP servers
    cmd.env("NPM_CONFIG_UPDATE_NOTIFIER", "false");
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

/// Build the reqwest client for an HTTP MCP server, attaching a bearer
/// `Authorization` default header when `auth_token_env` names a present
/// environment variable.
///
/// The token's *value* is read here and lives only inside the header map; it
/// is never logged, traced, or placed in an error (errors name the variable,
/// not its contents). A named-but-unset/empty variable is a hard error — the
/// caller asked for auth and we cannot provide it.
fn build_http_client(config: &McpServerConfig) -> Result<reqwest::Client, MvError> {
    let Some(var) = config.auth_token_env.as_deref() else {
        return Ok(reqwest::Client::new());
    };

    let token = std::env::var(var)
        .ok()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| MvError::McpServerError {
            server: config.name.clone(),
            details: format!("auth_token_env '{var}' is not set (or empty) in the environment"),
        })?;

    let mut value =
        reqwest::header::HeaderValue::try_from(format!("Bearer {token}")).map_err(|_| {
            MvError::McpServerError {
                server: config.name.clone(),
                // The token is malformed as a header; do not echo it.
                details: format!("token from '{var}' is not a valid HTTP header value"),
            }
        })?;
    value.set_sensitive(true);

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, value);

    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| MvError::McpServerError {
            server: config.name.clone(),
            details: format!("failed to build HTTP client: {e}"),
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

    let client = build_http_client(config)?;
    let http_config = StreamableHttpClientTransportConfig::with_uri(url.as_str());
    let transport = StreamableHttpClientTransport::with_client(client, http_config);

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
            auth_token_env: None,
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
            auth_token_env: None,
        };

        let result = connect_stdio(&config, handle).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("stdio transport requires 'command'"), "{err}");
    }

    fn http_config_with_auth(name: &str, auth_token_env: Option<&str>) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            transport: McpTransportType::Http,
            command: None,
            args: vec![],
            env: HashMap::new(),
            url: Some("http://127.0.0.1:1/mcp".to_string()),
            auth_token_env: auth_token_env.map(str::to_string),
        }
    }

    #[test]
    fn build_http_client_no_auth_succeeds() {
        let config = http_config_with_auth("plain", None);
        assert!(build_http_client(&config).is_ok());
    }

    #[test]
    fn build_http_client_missing_env_var_errors_naming_var() {
        // A variable name that is not set in the environment.
        let config = http_config_with_auth("klams", Some("MV_TEST_DEFINITELY_UNSET_TOKEN"));
        let err = build_http_client(&config).unwrap_err().to_string();
        assert!(err.contains("MV_TEST_DEFINITELY_UNSET_TOKEN"), "{err}");
        assert!(err.contains("klams"), "{err}");
        assert!(err.contains("not set"), "{err}");
    }

    #[test]
    fn build_http_client_with_token_succeeds_and_hides_value() {
        // SAFETY: single-threaded test; var is unique to this test.
        let var = "MV_TEST_KLAMS_TOKEN_OK";
        unsafe { std::env::set_var(var, "s3cr3t-value") };
        let config = http_config_with_auth("klams", Some(var));
        let client = build_http_client(&config);
        unsafe { std::env::remove_var(var) };
        let client = client.expect("client builds with a present token");
        // reqwest marks the header sensitive; its Debug must not leak the token.
        assert!(
            !format!("{client:?}").contains("s3cr3t-value"),
            "token value must not appear in client Debug"
        );
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
            auth_token_env: None,
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
            auth_token_env: None,
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
                    auth_token_env: None,
                },
                McpServerConfig {
                    name: "fail2".to_string(),
                    transport: McpTransportType::Stdio,
                    command: None,
                    args: vec![],
                    env: HashMap::new(),
                    url: None,
                    auth_token_env: None,
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
