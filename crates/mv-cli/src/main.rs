use clap::{Parser, Subcommand};
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
    /// Increase log verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Enable OTLP trace export [default endpoint: http://localhost:4318]
    #[arg(long, num_args = 0..=1, default_missing_value = "http://localhost:4318", global = true)]
    otlp: Option<String>,

    /// Output response as JSON object
    #[arg(short, long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Send a prompt to a model
    Prompt(PromptArgs),
    /// Manage and execute workflows
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
}

#[derive(Parser, Debug)]
struct PromptArgs {
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
}

#[derive(Subcommand, Debug)]
enum WorkflowAction {
    /// Load, validate, and execute a workflow file
    Run(WorkflowRunArgs),
    /// Validate a workflow file without executing it
    Validate(WorkflowValidateArgs),
}

#[derive(Parser, Debug)]
struct WorkflowRunArgs {
    /// Path to the workflow YAML file
    file: String,

    /// Workflow input (repeatable, format: KEY=VALUE)
    #[arg(short, long = "input", value_parser = parse_key_value)]
    inputs: Vec<(String, String)>,

    /// Path to models.yaml config file
    #[arg(short, long)]
    config: Option<String>,

    /// Path to MCP servers YAML config file [default: mcp-servers.yaml]
    #[arg(long)]
    mcp_config: Option<String>,
}

#[derive(Parser, Debug)]
struct WorkflowValidateArgs {
    /// Path to the workflow YAML file
    file: String,
}

fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}

#[tracing::instrument(name = "mv_cli_request", skip(args), fields(
    prompt = %args.prompt,
    model = args.model.as_deref().unwrap_or("default"),
))]
async fn run_prompt(
    args: &PromptArgs,
    _json: bool,
) -> std::result::Result<String, mv_core::MvError> {
    let prompt = mv_core::validate_prompt(&args.prompt)?;

    // Load model registry
    let registry = mv_core::ModelRegistry::resolve(args.config.as_deref())?;

    // Resolve the target model
    let entry = if let Some(ref model_id) = args.model {
        registry
            .get(model_id)
            .ok_or_else(|| mv_core::MvError::ModelNotInRegistry {
                model: model_id.clone(),
                available: registry.available_ids().join(", "),
            })?
    } else {
        registry.default_model()
    };

    let endpoint = args.endpoint.clone().unwrap_or_else(|| entry.endpoint());
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
    let mcp_connections = connect_mcp_servers(args.mcp_config.as_deref(), &agent_handle).await?;

    let result = match entry.provider.as_str() {
        "ollama" => call_ollama(&entry.id, &endpoint, prompt, agent_handle).await,
        "openai" => {
            let env_var = entry.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
            match std::env::var(env_var) {
                Ok(api_key) => {
                    call_openai(&entry.id, &endpoint, &api_key, prompt, agent_handle).await
                }
                Err(_) => Err(mv_core::MvError::ApiKeyMissing {
                    provider: entry.provider.clone(),
                    env_var: env_var.to_string(),
                }),
            }
        }
        "trtllm" => call_trtllm(entry, &endpoint, prompt, agent_handle).await,
        other => Err(mv_core::MvError::CompletionFailed {
            details: format!("unsupported provider: {other}"),
        }),
    };

    // Always shut down MCP connections, even on error
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    let response = result?;
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
            hint: "Is Ollama running?".to_string(),
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
        classify_rig_error(&msg, model, endpoint, "Is Ollama running?")
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
            hint: "Check the endpoint URL.".to_string(),
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
        debug!(raw_error = %msg, "openai prompt failed");
        classify_rig_error(&msg, model, endpoint, "Check the endpoint URL.")
    })?;

    Ok(response)
}

#[tracing::instrument(name = "llm_completion", skip(handle), fields(
    gen_ai.system = "trtllm",
    gen_ai.request.model = %entry.model_name(),
    trtllm.architecture = entry.architecture.as_deref().unwrap_or(""),
    trtllm.quant = entry.quant.as_deref().unwrap_or(""),
    trtllm.expected_vram_gb = entry.expected_vram_gb.unwrap_or(0),
))]
async fn call_trtllm(
    entry: &mv_core::ModelEntry,
    endpoint: &str,
    prompt: &str,
    handle: ToolServerHandle,
) -> Result<String, mv_core::MvError> {
    let trtllm_hint = "Start the server with: trtllm-serve <model-path>".to_string();

    // Health check before sending the prompt
    let health = mv_core::trtllm::health::check_health(endpoint).await;
    match health {
        mv_core::trtllm::health::HealthCheckResult::Healthy => {}
        mv_core::trtllm::health::HealthCheckResult::Unhealthy { status, body } => {
            return Err(mv_core::MvError::BackendUnreachable {
                endpoint: endpoint.to_string(),
                hint: format!("Server returned {status}: {body}. {trtllm_hint}"),
            });
        }
        mv_core::trtllm::health::HealthCheckResult::Unreachable { error } => {
            return Err(mv_core::MvError::BackendUnreachable {
                endpoint: endpoint.to_string(),
                hint: format!("TRT-LLM server not reachable ({error}). {trtllm_hint}"),
            });
        }
    }

    let model_name = entry.model_name();
    info!(model = %model_name, endpoint = %endpoint, locality = "local", "connecting to TRT-LLM");

    // Use the Chat Completions API (not the Responses API) because
    // OpenAI-compatible proxies typically only implement /v1/chat/completions.
    use rig::client::CompletionClient;

    let client = rig::providers::openai::CompletionsClient::builder()
        .api_key("tensorrt_llm")
        .base_url(endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
            hint: trtllm_hint.clone(),
        })?;

    let agent = client
        .agent(model_name)
        .preamble(SYSTEM_PREAMBLE)
        .tool_server_handle(handle)
        .default_max_turns(10)
        .build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        let msg = e.to_string();
        debug!(raw_error = %msg, "trtllm prompt failed");
        classify_rig_error(&msg, model_name, endpoint, &trtllm_hint)
    })?;

    Ok(response)
}

fn classify_rig_error(msg: &str, model: &str, endpoint: &str, hint: &str) -> mv_core::MvError {
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
            hint: hint.to_string(),
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
    // Try parsing with subcommands first; fall back to treating the first arg as a prompt
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) => {
            // If clap fails because the user typed `mv-cli "some prompt"` (no subcommand),
            // re-parse treating the first positional as a prompt subcommand
            if e.kind() == clap::error::ErrorKind::InvalidSubcommand
                || e.kind() == clap::error::ErrorKind::UnknownArgument
            {
                // Rebuild args: insert "prompt" as the subcommand
                let mut args: Vec<String> = std::env::args().collect();
                args.insert(1, "prompt".to_string());
                match Cli::try_parse_from(&args) {
                    Ok(cli) => cli,
                    Err(e2) => {
                        e2.exit();
                    }
                }
            } else {
                e.exit();
            }
        }
    };

    init_tracing(cli.verbose, cli.otlp.as_deref());

    let result = match &cli.command {
        Some(Commands::Prompt(args)) => match run_prompt(args, cli.json).await {
            Ok(response) => {
                print_success(&response, cli.json);
                Ok(())
            }
            Err(err) => Err(err),
        },
        None => {
            // No subcommand and no prompt — show help
            eprintln!("Error: no prompt provided. Use: mv-cli <PROMPT> or mv-cli prompt <PROMPT>");
            shutdown_tracing();
            std::process::exit(2);
        }
        Some(Commands::Workflow { action }) => match action {
            WorkflowAction::Run(args) => run_workflow(args, cli.json).await,
            WorkflowAction::Validate(args) => run_workflow_validate(args).await,
        },
    };

    if let Err(err) = result {
        print_error(&err, cli.json);
        shutdown_tracing();
        std::process::exit(1);
    }

    shutdown_tracing();
}

async fn run_workflow(args: &WorkflowRunArgs, json: bool) -> Result<(), mv_core::MvError> {
    use std::path::Path;

    let path = Path::new(&args.file);
    let workflow = mv_core::workflow::parser::load_from_file(path)?;

    // Validate structure
    let validation_errors = mv_core::workflow::validate::validate(&workflow);
    if !validation_errors.is_empty() {
        let details = validation_errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(mv_core::MvError::WorkflowValidationError { details });
    }

    // Build inputs map
    let inputs: std::collections::HashMap<String, String> = args.inputs.iter().cloned().collect();

    // Set up executors
    let registry = mv_core::ModelRegistry::resolve(args.config.as_deref())?;

    let tool_server = ToolServer::new()
        .tool(mv_core::tools::file_list::FileList)
        .tool(mv_core::tools::file_read::FileRead)
        .tool(mv_core::tools::shell_exec::ShellExec)
        .tool(mv_core::tools::http_get::HttpGet);
    let agent_handle = tool_server.run();

    let mcp_connections = connect_mcp_servers(args.mcp_config.as_deref(), &agent_handle).await?;

    let prompt_exec = RigPromptExecutor {
        registry,
        agent_handle,
    };
    let tool_exec = NoopToolExecutor;

    let workflow_dir = path.parent().unwrap_or(Path::new("."));
    let result = mv_core::workflow::engine::execute_workflow(
        &workflow,
        inputs,
        &prompt_exec,
        &tool_exec,
        workflow_dir,
    )
    .await;

    // Always shut down MCP connections, even on error
    mv_core::mcp::client::shutdown_all(mcp_connections).await;

    let result = result?;

    // Print outputs
    if json {
        let obj = serde_json::json!({
            "workflow": workflow.name,
            "outputs": result.outputs,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        for (name, value) in &result.outputs {
            println!("## {name}\n");
            println!("{value}\n");
        }
    }

    Ok(())
}

async fn run_workflow_validate(args: &WorkflowValidateArgs) -> Result<(), mv_core::MvError> {
    let path = std::path::Path::new(&args.file);
    let workflow = mv_core::workflow::parser::load_from_file(path)?;

    let errors = mv_core::workflow::validate::validate(&workflow);
    if errors.is_empty() {
        println!(
            "\u{2713} workflow '{}' is valid ({} steps, {} input{}, {} output{})",
            workflow.name,
            workflow.steps.len(),
            workflow.inputs.len(),
            if workflow.inputs.len() == 1 { "" } else { "s" },
            workflow.outputs.len(),
            if workflow.outputs.len() == 1 { "" } else { "s" },
        );
        Ok(())
    } else {
        let details = errors
            .iter()
            .map(|e| format!("  - {e}"))
            .collect::<Vec<_>>()
            .join("\n");
        Err(mv_core::MvError::WorkflowValidationError { details })
    }
}

/// Prompt executor that uses rig-core (Ollama/OpenAI) for real LLM calls.
struct RigPromptExecutor {
    registry: mv_core::ModelRegistry,
    agent_handle: ToolServerHandle,
}

impl mv_core::workflow::engine::PromptExecutor for RigPromptExecutor {
    async fn execute_prompt(
        &self,
        prompt_text: &str,
        model: &str,
        _temperature: Option<f64>,
        _max_tokens: Option<u64>,
    ) -> Result<String, mv_core::MvError> {
        let entry = self
            .registry
            .get(model)
            .or_else(|| {
                // Fall back to default model if the specified one isn't in registry
                Some(self.registry.default_model())
            })
            .unwrap();

        let endpoint = entry.endpoint();

        match entry.provider.as_str() {
            "ollama" => {
                call_ollama(&entry.id, &endpoint, prompt_text, self.agent_handle.clone()).await
            }
            "openai" => {
                let env_var = entry.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
                let api_key =
                    std::env::var(env_var).map_err(|_| mv_core::MvError::ApiKeyMissing {
                        provider: entry.provider.clone(),
                        env_var: env_var.to_string(),
                    })?;
                call_openai(
                    &entry.id,
                    &endpoint,
                    &api_key,
                    prompt_text,
                    self.agent_handle.clone(),
                )
                .await
            }
            "trtllm" => call_trtllm(entry, &endpoint, prompt_text, self.agent_handle.clone()).await,
            other => Err(mv_core::MvError::CompletionFailed {
                details: format!("unsupported provider: {other}"),
            }),
        }
    }
}

/// Placeholder tool executor — tool steps are not yet fully wired to built-in tools.
struct NoopToolExecutor;

impl mv_core::workflow::engine::ToolExecutor for NoopToolExecutor {
    async fn execute_tool(
        &self,
        tool_name: &str,
        _inputs: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, mv_core::MvError> {
        Err(mv_core::MvError::WorkflowStepFailed {
            step: String::new(),
            details: format!("tool execution not yet implemented for '{tool_name}'"),
        })
    }
}
