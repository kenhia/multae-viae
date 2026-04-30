use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::MvError;

/// Transport mechanism for connecting to an MCP server.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportType {
    Stdio,
    Http,
}

/// A single MCP server entry from configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub transport: McpTransportType,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub url: Option<String>,
}

/// YAML wrapper for the MCP servers configuration file.
#[derive(Debug, Deserialize)]
pub struct McpServersConfig {
    pub servers: Vec<McpServerConfig>,
}

impl McpServersConfig {
    /// Load and validate an MCP servers config from a YAML file.
    pub fn load(path: &Path) -> Result<Self, MvError> {
        let content = std::fs::read_to_string(path).map_err(|_| MvError::McpConfigNotFound {
            path: path.display().to_string(),
        })?;
        let config: McpServersConfig =
            serde_yml::from_str(&content).map_err(|e| MvError::McpConfigParseError {
                path: path.display().to_string(),
                details: e.to_string(),
            })?;
        config.validate()?;
        Ok(config)
    }

    /// Resolve config: explicit path → ./mcp-servers.yaml → None.
    ///
    /// - If `config_path` is provided, load from that path (error if missing).
    /// - If not provided, try `mcp-servers.yaml` in the current directory.
    /// - If the default file doesn't exist, return `None` (no MCP servers).
    pub fn resolve(config_path: Option<&str>) -> Result<Option<Self>, MvError> {
        if let Some(path) = config_path {
            return Self::load(Path::new(path)).map(Some);
        }
        let default_path = Path::new("mcp-servers.yaml");
        if default_path.exists() {
            return Self::load(default_path).map(Some);
        }
        Ok(None)
    }

    /// Validate the configuration: check transport-specific requirements and uniqueness.
    fn validate(&self) -> Result<(), MvError> {
        if self.servers.is_empty() {
            tracing::warn!("MCP config has no servers defined");
            return Ok(());
        }

        let mut seen_names = std::collections::HashSet::new();
        for server in &self.servers {
            if !seen_names.insert(&server.name) {
                return Err(MvError::McpDuplicateServer {
                    name: server.name.clone(),
                });
            }

            match server.transport {
                McpTransportType::Stdio => {
                    if server.command.is_none() {
                        return Err(MvError::McpServerError {
                            server: server.name.clone(),
                            details: "stdio transport requires 'command'".to_string(),
                        });
                    }
                }
                McpTransportType::Http => {
                    if server.url.is_none() {
                        return Err(MvError::McpServerError {
                            server: server.name.clone(),
                            details: "http transport requires 'url'".to_string(),
                        });
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<McpServersConfig, MvError> {
        let config: McpServersConfig =
            serde_yml::from_str(yaml).map_err(|e| MvError::McpConfigParseError {
                path: "<test>".to_string(),
                details: e.to_string(),
            })?;
        config.validate()?;
        Ok(config)
    }

    #[test]
    fn parse_valid_stdio_server() {
        let yaml = r#"
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    env:
      NODE_ENV: production
"#;
        let config = parse(yaml).unwrap();
        assert_eq!(config.servers.len(), 1);
        let s = &config.servers[0];
        assert_eq!(s.name, "filesystem");
        assert_eq!(s.transport, McpTransportType::Stdio);
        assert_eq!(s.command.as_deref(), Some("npx"));
        assert_eq!(s.args.len(), 3);
        assert_eq!(s.env.get("NODE_ENV").unwrap(), "production");
    }

    #[test]
    fn parse_valid_http_server() {
        let yaml = r#"
servers:
  - name: rag
    transport: http
    url: http://192.168.1.100:8080/mcp
"#;
        let config = parse(yaml).unwrap();
        assert_eq!(config.servers.len(), 1);
        let s = &config.servers[0];
        assert_eq!(s.name, "rag");
        assert_eq!(s.transport, McpTransportType::Http);
        assert_eq!(s.url.as_deref(), Some("http://192.168.1.100:8080/mcp"));
    }

    #[test]
    fn parse_multiple_servers() {
        let yaml = r#"
servers:
  - name: filesystem
    transport: stdio
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
  - name: rag
    transport: http
    url: http://localhost:8080/mcp
"#;
        let config = parse(yaml).unwrap();
        assert_eq!(config.servers.len(), 2);
    }

    #[test]
    fn error_on_missing_command_for_stdio() {
        let yaml = r#"
servers:
  - name: bad
    transport: stdio
"#;
        let err = parse(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("stdio transport requires 'command'"), "{msg}");
    }

    #[test]
    fn error_on_missing_url_for_http() {
        let yaml = r#"
servers:
  - name: bad
    transport: http
"#;
        let err = parse(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("http transport requires 'url'"), "{msg}");
    }

    #[test]
    fn error_on_duplicate_server_names() {
        let yaml = r#"
servers:
  - name: myserver
    transport: stdio
    command: echo
  - name: myserver
    transport: stdio
    command: echo
"#;
        let err = parse(yaml).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("duplicate MCP server name: 'myserver'"),
            "{msg}"
        );
    }

    #[test]
    fn error_on_invalid_yaml() {
        let yaml = "not: [valid: yaml: {{";
        let err = parse(yaml).unwrap_err();
        assert!(matches!(err, MvError::McpConfigParseError { .. }));
    }

    #[test]
    fn empty_servers_is_valid_with_warning() {
        let yaml = "servers: []";
        let config = parse(yaml).unwrap();
        assert!(config.servers.is_empty());
    }

    #[test]
    fn defaults_for_optional_fields() {
        let yaml = r#"
servers:
  - name: minimal
    transport: stdio
    command: /usr/bin/server
"#;
        let config = parse(yaml).unwrap();
        let s = &config.servers[0];
        assert!(s.args.is_empty());
        assert!(s.env.is_empty());
        assert!(s.url.is_none());
    }
}
