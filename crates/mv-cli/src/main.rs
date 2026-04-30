use clap::Parser;
use rig::completion::Prompt;
use rig::tool::server::{ToolServer, ToolServerHandle};
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use std::sync::OnceLock;

static TRACER_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

const SYSTEM_PREAMBLE: &str = "\
You are a helpful assistant with access to local tools. \
Use the available tools to answer questions that require interacting with the local environment. \
If a question can be answered from your own knowledge, respond directly without using tools. \
When you use a tool, incorporate the result into a clear, human-readable response.";

#[derive(Parser, Debug)]
#[command(name = "mv-cli", version, about = "Send a prompt to a local LLM")]
struct Cli {
    /// The prompt to send to the model
    prompt: String,

    /// Model name (must exist in config or built-in registry)
    #[arg(short, long)]
    model: Option<String>,

    /// Ollama endpoint (overrides model config)
    #[arg(short, long)]
    endpoint: Option<String>,

    /// Path to models.yaml config file
    #[arg(short, long)]
    config: Option<String>,

    /// Path to MCP servers YAML config file [default: mcp-servers.yaml]
    #[arg(long)]
    mcp_config: Option<String>,

    /// Enable OTLP trace export [default endpoint: http://localhost:4318]
    #[arg(long, num_args = 0..=1, default_missing_value = "http://localhost:4318")]
    otlp: Option<String>,

    /// Output response as JSON object
    #[arg(short, long)]
    json: bool,

    /// Increase log verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[tracing::instrument(name = "mv_cli_request", skip(cli), fields(
    prompt = %cli.prompt,
    model = cli.model.as_deref().unwrap_or("default"),
))]
async fn run(cli: &Cli) -> std::result::Result<String, mv_core::MvError> {
    let prompt = mv_core::validate_prompt(&cli.prompt)?;

    // Load model registry
    let registry = mv_core::ModelRegistry::resolve(cli.config.as_deref())?;

    // Resolve the target model
    let entry = if let Some(ref model_id) = cli.model {
        registry
            .get(model_id)
            .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                model: model_id.clone(),
                available: registry.available_ids().join(", "),
            })?
    } else {
        registry.default_model()
    };

    let endpoint = cli.endpoint.clone().unwrap_or_else(|| entry.endpoint());
    let locality = entry.locality();

    debug!(
        model = %entry.id,
        provider = %entry.provider,
        endpoint = %endpoint,
        locality = %locality,
        "resolved model"
    );

    // Set up agent ToolServer with built-in tools
    let tool_server = ToolServer::new()
        .tool(mv_core::tools::file_list::FileList)
        .tool(mv_core::tools::file_read::FileRead)
        .tool(mv_core::tools::shell_exec::ShellExec)
        .tool(mv_core::tools::http_get::HttpGet);
    let agent_handle = tool_server.run();

    // Connect MCP servers to a separate handle, then register cleaned tools on the agent handle
    let mcp_connections = connect_mcp_servers(cli.mcp_config.as_deref(), &agent_handle).await?;

    let response = match entry.provider.as_str() {
        "ollama" => call_ollama(&entry.id, &endpoint, prompt, agent_handle).await?,
        "openai" => {
            let env_var = entry.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
            let api_key = std::env::var(env_var).map_err(|_| mv_core::MvError::ApiKeyMissing {
                provider: entry.provider.clone(),
                env_var: env_var.to_string(),
            })?;
            call_openai(&entry.id, &endpoint, &api_key, prompt, agent_handle).await?
        }
        other => {
            return Err(mv_core::MvError::CompletionFailed {
                details: format!("unsupported provider: {other}"),
            });
        }
    };

    // Gracefully shut down MCP connections
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    info!(len = response.len(), "received response");
    Ok(response)
}

/// Load MCP config, connect to all servers on a dedicated MCP handle,
/// then register cleaned tool wrappers on the agent handle.
/// Returns live connections (for shutdown).
async fn connect_mcp_servers(
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

#[tracing::instrument(name = "llm_completion", skip(handle), fields(
    gen_ai.system = "ollama",
    gen_ai.request.model = %model,
))]
async fn call_ollama(
    model: &str,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
) -> Result<String, mv_core::MvError> {
    use rig::client::{CompletionClient, Nothing};

    info!(model = %model, endpoint = %endpoint, locality = "local", "connecting to Ollama");

    let client = rig::providers::ollama::Client::builder()
        .api_key(Nothing)
        .base_url(endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
        })?;

    let agent = client
        .agent(model)
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(10)
        .build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        let msg = e.to_string();
        classify_rig_error(&msg, model, endpoint)
    })?;

    Ok(response)
}

#[tracing::instrument(name = "llm_completion", skip(handle, api_key), fields(
    gen_ai.system = "openai",
    gen_ai.request.model = %model,
))]
async fn call_openai(
    model: &str,
    endpoint: &str,
    api_key: &str,
    prompt: &str,
    handle: ToolServerHandle,
) -> Result<String, mv_core::MvError> {
    use rig::client::CompletionClient;

    info!(model = %model, endpoint = %endpoint, locality = "cloud", "connecting to OpenAI");

    let client = rig::providers::openai::Client::builder()
        .api_key(api_key)
        .base_url(endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
        })?;

    let agent = client
        .agent(model)
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(10)
        .build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        let msg = e.to_string();
        classify_rig_error(&msg, model, endpoint)
    })?;

    Ok(response)
}

fn classify_rig_error(msg: &str, model: &str, endpoint: &str) -> mv_core::MvError {
    if msg.contains("not found") || (msg.contains("model") && msg.contains("pull")) {
        mv_core::MvError::ModelNotFound {
            model: model.to_string(),
        }
    } else if msg.contains("connection")
        || msg.contains("Connection")
        || msg.contains("connect")
        || msg.contains("tcp")
        || msg.contains("error sending request")
        || msg.contains("HttpError")
    {
        mv_core::MvError::BackendUnreachable {
            endpoint: endpoint.to_string(),
        }
    } else {
        mv_core::MvError::CompletionFailed {
            details: msg.to_string(),
        }
    }
}

fn print_success(response: &str, json: bool) {
    if json {
        let obj = serde_json::json!({ "response": response });
        println!("{}", obj);
    } else {
        print!("{response}");
    }
}

fn print_error(err: &mv_core::MvError, json: bool) {
    if json {
        let obj = serde_json::json!({ "error": err.to_string() });
        println!("{}", obj);
    } else {
        eprintln!("Error: {err}");
    }
}

fn init_tracing(verbose: u8, otlp_endpoint: Option<&str>) {
    let console_filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        match verbose {
            0 => EnvFilter::new("warn"),
            1 => EnvFilter::new("info"),
            _ => EnvFilter::new("debug"),
        }
    };

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(console_filter);

    let otel_layer = otlp_endpoint.and_then(|endpoint| match init_otel_layer(endpoint) {
        Ok(layer) => Some(layer.with_filter(EnvFilter::new("info"))),
        Err(e) => {
            eprintln!("Warning: Failed to initialize OTLP exporter: {e}");
            None
        }
    });

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(otel_layer)
        .init();
}

fn init_otel_layer<S>(
    endpoint: &str,
) -> Result<
    tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::SdkTracer>,
    Box<dyn std::error::Error>,
>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::trace::TracerProvider;
    use opentelemetry_otlp::{SpanExporter, WithExportConfig};

    // with_endpoint() uses the URL as-is; append /v1/traces for OTLP/HTTP
    let traces_endpoint = if endpoint.ends_with("/v1/traces") {
        endpoint.to_string()
    } else {
        format!("{}/v1/traces", endpoint.trim_end_matches('/'))
    };

    let exporter = SpanExporter::builder()
        .with_http()
        .with_endpoint(&traces_endpoint)
        .build()?;

    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name("mv-cli")
                .build(),
        )
        .with_batch_exporter(exporter)
        .build();

    let tracer = provider.tracer("mv-cli");

    let _ = TRACER_PROVIDER.set(provider);

    Ok(tracing_opentelemetry::layer().with_tracer(tracer))
}

fn shutdown_tracing() {
    if let Some(provider) = TRACER_PROVIDER.get() {
        if let Err(e) = provider.force_flush() {
            eprintln!("Warning: failed to flush traces: {e:?}");
        }
        if let Err(e) = provider.shutdown() {
            eprintln!("Warning: failed to shutdown tracer: {e:?}");
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.otlp.as_deref());

    match run(&cli).await {
        Ok(response) => {
            print_success(&response, cli.json);
        }
        Err(err) => {
            print_error(&err, cli.json);
            shutdown_tracing();
            std::process::exit(1);
        }
    }

    shutdown_tracing();
}
