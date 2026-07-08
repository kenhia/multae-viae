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

    /// Path to a schedules YAML (cron → workflow). Omit to run no schedules.
    #[arg(long)]
    schedules: Option<PathBuf>,

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
    let state = AppState::build(
        registry,
        cli.mcp_servers.as_deref(),
        cli.workflows_dir.clone(),
    )
    .await?;

    // Parse + validate schedules at boot — a bad cron or missing workflow is a
    // startup error, not a silent no-op — then spawn one task per schedule.
    let scheduler_handles = match &cli.schedules {
        Some(path) => {
            let parsed = mv_server::scheduler::load_schedules(path, &cli.workflows_dir)?;
            info!(count = parsed.len(), "schedules loaded");
            mv_server::scheduler::spawn(state.clone(), parsed)
        }
        None => Vec::new(),
    };

    let app = build_router(state.clone());

    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    info!(addr = %cli.bind, "mv-server listening");

    // Serve until a shutdown signal; axum drains in-flight requests before the
    // future resolves.
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Ordered teardown after the listener stops accepting and in-flight
    // requests have drained: stop the scheduler, shut down MCP connections
    // (concurrent, time-bounded), then flush telemetry.
    for handle in &scheduler_handles {
        handle.abort();
    }
    state.mcp.shutdown().await;
    mv_server::telemetry::shutdown_tracing();
    info!("mv-server stopped");
    Ok(())
}

/// Resolve on SIGINT (Ctrl-C) or SIGTERM (the signal a service manager sends to
/// stop the daemon), so a `systemctl stop` triggers the same graceful drain.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    info!("shutdown signal received");
}
