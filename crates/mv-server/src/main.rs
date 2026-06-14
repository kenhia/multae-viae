//! mv-server binary: parse flags, build state, bind, serve.
//!
//! Deliberately thin — all routing/state logic lives in the library
//! ([`mv_server`]) so it is testable in-process. This file owns only process
//! concerns: CLI parsing, telemetry init, the listener, and shutdown.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::Parser;
use mv_server::{AppState, build_router};
use tracing::info;

/// Local-first agentic controller, as a REST daemon.
#[derive(Parser, Debug)]
#[command(name = "mv-server", version, about)]
struct Cli {
    /// Address to bind the HTTP server to.
    #[arg(long, default_value = "127.0.0.1:7077")]
    bind: SocketAddr,

    /// Path to models.yaml (defaults to ./models.yaml or built-in defaults).
    #[arg(long)]
    models: Option<String>,

    /// Path to mcp-servers.yaml (defaults to discovery; omit for no MCP).
    #[arg(long)]
    mcp_servers: Option<String>,

    /// Directory workflow names in `POST /v1/workflows/run` resolve against.
    #[arg(long, default_value = ".")]
    workflows_dir: PathBuf,

    /// Export OpenTelemetry spans to this OTLP/HTTP collector
    /// (default `http://localhost:4318` when the flag is given without a value).
    #[arg(long, num_args = 0..=1, default_missing_value = "http://localhost:4318")]
    otlp: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    mv_server::telemetry::init_tracing(cli.otlp.as_deref());

    let registry = mv_core::ModelRegistry::resolve(cli.models.as_deref())?;
    let state = AppState::build(registry, cli.mcp_servers.as_deref(), cli.workflows_dir).await?;

    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    info!(addr = %cli.bind, "mv-server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    mv_server::telemetry::shutdown_tracing();
    info!("mv-server stopped");
    Ok(())
}

/// Resolve when the process receives Ctrl-C (SIGINT). WS5 extends this to also
/// honor SIGTERM and to drain/stop the scheduler and MCP manager in order.
async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("shutdown signal received");
}
