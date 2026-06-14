//! MCP connection lifecycle manager.
//!
//! Owns the long-lived MCP connections for the process. The pre-013 model was
//! a bare `Vec<McpConnection>` connected once per invocation and shut down
//! sequentially — fine for a one-shot CLI run, but a throwaway for a daemon
//! (fable finding F22): no keep-alive, no reconnect, and a sequential shutdown
//! one bad server could wedge. The manager replaces it as the single MCP
//! lifecycle implementation:
//!
//! - **One-shot mode** (CLI): [`McpManager::connect`] → use → [`McpManager::shutdown`].
//! - **Daemon mode** (mv-server): the same, plus [`McpManager::reconnect_dead`]
//!   driven on a health-monitor tick — a server that drops is reconnected with
//!   exponential backoff while every other server (and the rest of the daemon)
//!   keeps serving.
//!
//! Shutdown is **concurrent and time-bounded**: all servers are cancelled at
//! once via `join_all`, each under [`SHUTDOWN_TIMEOUT`], so one server that
//! hangs on cancel cannot extend process exit past that budget.
//!
//! Tool dispatch isolation is structural: MCP tools reach the model as
//! `CleanedMcpTool` wrappers that delegate to the shared MCP `ToolServerHandle`
//! and map any failure to a tool-level error (see [`crate::mcp::registry`]), so
//! a call into a dead server returns an error the model can route around — it
//! never panics or takes down the daemon.

use std::collections::HashMap;
use std::time::Duration;

use rig::tool::server::{ToolServer, ToolServerHandle};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::MvError;
use crate::mcp::client::{McpConnection, connect_one};
use crate::mcp::config::McpServersConfig;
use crate::mcp::registry::register_mcp_tools;

/// Per-server budget for a graceful shutdown. A server that does not finish
/// cancelling within this window is abandoned so process exit is never wedged.
pub const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Exponential backoff schedule for reconnect attempts.
#[derive(Clone, Copy, Debug)]
pub struct Backoff {
    /// Delay before the first retry; doubles each subsequent attempt.
    pub base: Duration,
    /// Ceiling on the delay — backoff grows to here and then holds.
    pub max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self {
            base: Duration::from_millis(500),
            max: Duration::from_secs(30),
        }
    }
}

impl Backoff {
    /// Delay before reconnect `attempt` (1-based): `base * 2^(attempt-1)`,
    /// saturating at `max`. `attempt == 0` is treated as the first attempt.
    /// A fresh success resets the caller's attempt counter, so the schedule
    /// starts over at `base` after any reconnection.
    pub fn delay(&self, attempt: u32) -> Duration {
        if attempt <= 1 {
            return self.base.min(self.max);
        }
        // Saturating shift: cap the exponent so 2^(attempt-1) cannot overflow,
        // then clamp the product to `max`.
        let shift = (attempt - 1).min(32);
        let scaled = self
            .base
            .saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX));
        scaled.min(self.max)
    }
}

/// Owns the process's MCP connections and their lifecycle.
pub struct McpManager {
    /// The agent-facing handle the model's cleaned tool wrappers register on.
    agent_handle: ToolServerHandle,
    /// The handle MCP servers connect to and `CleanedMcpTool` delegates calls
    /// to. Reconnecting a server re-registers it here, so the wrappers already
    /// on `agent_handle` keep working across a reconnect.
    mcp_handle: ToolServerHandle,
    /// The full configured server set, kept so dropped servers can be redialed.
    config: McpServersConfig,
    /// Currently live connections, keyed by server name.
    connections: Mutex<HashMap<String, McpConnection>>,
    backoff: Backoff,
}

impl McpManager {
    /// Connect every server in `config_path` (resolving the same default file
    /// the CLI uses) onto a fresh MCP handle, register their cleaned tools on
    /// `agent_handle`, and return the manager owning the live connections.
    ///
    /// Replaces the old `connect_mcp_servers` helper. A `None`/absent config
    /// yields an empty manager whose `shutdown` is a no-op — callers need no
    /// special case. Per-server connect failures are logged and skipped (an
    /// MCP server is never fatal to the run).
    pub async fn connect(
        config_path: Option<&str>,
        agent_handle: &ToolServerHandle,
    ) -> Result<Self, MvError> {
        let config = McpServersConfig::resolve(config_path)?.unwrap_or(McpServersConfig {
            servers: Vec::new(),
        });
        Self::connect_with(config, agent_handle.clone(), Backoff::default()).await
    }

    /// Construct from an already-resolved config (the testable core of
    /// [`connect`], and the entry point a daemon uses when it wants a custom
    /// `Backoff`).
    pub async fn connect_with(
        config: McpServersConfig,
        agent_handle: ToolServerHandle,
        backoff: Backoff,
    ) -> Result<Self, MvError> {
        let mcp_handle = ToolServer::new().run();
        let mut connections = HashMap::new();

        for server in &config.servers {
            match connect_one(server, mcp_handle.clone()).await {
                Ok(conn) => {
                    connections.insert(conn.name.clone(), conn);
                }
                Err(e) => warn!(server = %server.name, error = %e, "MCP server failed to connect"),
            }
        }

        // Register cleaned wrappers for whatever connected. Built-in/overlap
        // names are skipped inside; cross-server name collisions keep the
        // first registration and log a warning (see register_mcp_tools).
        register_mcp_tools(&mcp_handle, &agent_handle).await;

        info!(
            connected = connections.len(),
            configured = config.servers.len(),
            "MCP manager ready"
        );

        Ok(Self {
            agent_handle,
            mcp_handle,
            config,
            connections: Mutex::new(connections),
            backoff,
        })
    }

    /// Names of every server in the configuration (live or not).
    pub fn configured_servers(&self) -> Vec<String> {
        self.config.servers.iter().map(|s| s.name.clone()).collect()
    }

    /// Names of the servers currently connected and alive (transport open).
    pub async fn live_servers(&self) -> Vec<String> {
        let conns = self.connections.lock().await;
        conns
            .iter()
            .filter(|(_, c)| c.is_alive())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// One health-monitor sweep: drop connections whose transport has closed,
    /// then dial every configured server that is not currently live. Returns
    /// the names that were (re)connected this sweep.
    ///
    /// This is the unit of work a daemon runs on a tick; pairing it with the
    /// [`Backoff`] schedule (sleep `backoff.delay(attempt)` between failed
    /// sweeps for a server) gives exponential-backoff reconnect without a
    /// long-lived task holding the lock. Reconnected servers re-register their
    /// tools on the shared MCP handle, so the model's tool wrappers recover
    /// transparently.
    pub async fn reconnect_dead(&self) -> Vec<String> {
        let mut conns = self.connections.lock().await;

        // Reap dead/closed connections so they are eligible for redial.
        conns.retain(|name, c| {
            let alive = c.is_alive();
            if !alive {
                warn!(server = %name, "MCP connection dropped; will attempt reconnect");
            }
            alive
        });

        let mut reconnected = Vec::new();
        for server in &self.config.servers {
            if conns.contains_key(&server.name) {
                continue;
            }
            match connect_one(server, self.mcp_handle.clone()).await {
                Ok(conn) => {
                    info!(server = %server.name, "MCP server reconnected");
                    conns.insert(server.name.clone(), conn);
                    reconnected.push(server.name.clone());
                }
                Err(e) => {
                    warn!(server = %server.name, error = %e, "MCP reconnect attempt failed")
                }
            }
        }

        if !reconnected.is_empty() {
            // New servers may bring tools the model hasn't seen yet.
            register_mcp_tools(&self.mcp_handle, &self.agent_handle).await;
        }
        reconnected
    }

    /// The reconnect backoff schedule (for a daemon's monitor loop to consult).
    pub fn backoff(&self) -> Backoff {
        self.backoff
    }

    /// Gracefully shut down every connection, concurrently and time-bounded.
    ///
    /// All servers are cancelled at once (`join_all`); each cancel is wrapped
    /// in [`SHUTDOWN_TIMEOUT`], so a server that hangs on shutdown is abandoned
    /// rather than allowed to wedge process exit.
    pub async fn shutdown(self) {
        let conns: Vec<McpConnection> = {
            let mut guard = self.connections.lock().await;
            guard.drain().map(|(_, c)| c).collect()
        };
        if conns.is_empty() {
            return;
        }

        let count = conns.len();
        let futs = conns.into_iter().map(|conn| async move {
            let name = conn.name.clone();
            if tokio::time::timeout(SHUTDOWN_TIMEOUT, conn.shutdown())
                .await
                .is_err()
            {
                warn!(server = %name, "MCP shutdown timed out; abandoning connection");
            }
        });
        futures::future::join_all(futs).await;
        info!(count, "MCP manager shut down");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_then_caps() {
        let b = Backoff {
            base: Duration::from_millis(100),
            max: Duration::from_secs(1),
        };
        assert_eq!(b.delay(0), Duration::from_millis(100)); // treated as first
        assert_eq!(b.delay(1), Duration::from_millis(100));
        assert_eq!(b.delay(2), Duration::from_millis(200));
        assert_eq!(b.delay(3), Duration::from_millis(400));
        assert_eq!(b.delay(4), Duration::from_millis(800));
        // 1600ms would exceed the 1s cap → clamped.
        assert_eq!(b.delay(5), Duration::from_secs(1));
        assert_eq!(b.delay(100), Duration::from_secs(1));
    }

    #[test]
    fn backoff_does_not_overflow_on_large_attempt() {
        let b = Backoff::default();
        // A pathologically high attempt count must saturate, not panic.
        assert_eq!(b.delay(u32::MAX), b.max);
    }

    #[tokio::test]
    async fn empty_config_connects_and_shuts_down_cleanly() {
        let agent = ToolServer::new().run();
        let mgr = McpManager::connect_with(
            McpServersConfig { servers: vec![] },
            agent,
            Backoff::default(),
        )
        .await
        .unwrap();
        assert!(mgr.configured_servers().is_empty());
        assert!(mgr.live_servers().await.is_empty());
        // Shutdown of an empty manager is a no-op and returns promptly.
        mgr.shutdown().await;
    }

    #[tokio::test]
    async fn failed_servers_are_skipped_not_fatal() {
        use crate::mcp::config::{McpServerConfig, McpTransportType};
        use std::collections::HashMap;

        let agent = ToolServer::new().run();
        let config = McpServersConfig {
            servers: vec![McpServerConfig {
                name: "dead".to_string(),
                transport: McpTransportType::Stdio,
                command: Some("/nonexistent/binary".to_string()),
                args: vec![],
                env: HashMap::new(),
                url: None,
                auth_token_env: None,
            }],
        };
        let mgr = McpManager::connect_with(config, agent, Backoff::default())
            .await
            .expect("connect never fails on a bad server — it logs and skips");
        // The server is configured but never became live.
        assert_eq!(mgr.configured_servers(), vec!["dead".to_string()]);
        assert!(mgr.live_servers().await.is_empty());
        mgr.shutdown().await;
    }

    /// A reconnect sweep redials a configured-but-absent server. The dead
    /// stdio server stays absent (the spawn keeps failing), proving the sweep
    /// neither panics nor wedges when reconnection itself fails — the daemon
    /// keeps running and will retry on the next tick per the backoff schedule.
    #[tokio::test]
    async fn reconnect_sweep_tolerates_persistent_failure() {
        use crate::mcp::config::{McpServerConfig, McpTransportType};
        use std::collections::HashMap;

        let agent = ToolServer::new().run();
        let config = McpServersConfig {
            servers: vec![McpServerConfig {
                name: "dead".to_string(),
                transport: McpTransportType::Stdio,
                command: Some("/nonexistent/binary".to_string()),
                args: vec![],
                env: HashMap::new(),
                url: None,
                auth_token_env: None,
            }],
        };
        let mgr = McpManager::connect_with(config, agent, Backoff::default())
            .await
            .unwrap();
        let reconnected = mgr.reconnect_dead().await;
        assert!(
            reconnected.is_empty(),
            "a server that cannot spawn does not reconnect"
        );
        assert!(mgr.live_servers().await.is_empty());
    }
}
