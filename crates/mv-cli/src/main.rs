use clap::Parser;
use rig::completion::Prompt;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

use std::sync::OnceLock;

static TRACER_PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

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

    let response = match entry.provider.as_str() {
        "ollama" => call_ollama(&entry.id, &endpoint, prompt).await?,
        "openai" => {
            let env_var = entry.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
            let api_key = std::env::var(env_var).map_err(|_| mv_core::MvError::ApiKeyMissing {
                provider: entry.provider.clone(),
                env_var: env_var.to_string(),
            })?;
            call_openai(&entry.id, &endpoint, &api_key, prompt).await?
        }
        other => {
            return Err(mv_core::MvError::CompletionFailed {
                details: format!("unsupported provider: {other}"),
            });
        }
    };

    info!(len = response.len(), "received response");
    Ok(response)
}

#[tracing::instrument(skip(prompt), fields(
    gen_ai.system = "ollama",
    gen_ai.request.model = %model,
    mv.model.locality = "local",
    prompt.len = prompt.len(),
))]
async fn call_ollama(
    model: &str,
    endpoint: &str,
    prompt: &str,
) -> Result<String, mv_core::MvError> {
    use rig::client::{CompletionClient, Nothing};

    info!(endpoint = %endpoint, "connecting to Ollama");

    let client = rig::providers::ollama::Client::builder()
        .api_key(Nothing)
        .base_url(endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
        })?;

    let agent = client.agent(model).build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        let msg = e.to_string();
        classify_rig_error(&msg, model, endpoint)
    })?;

    Ok(response)
}

#[tracing::instrument(skip(prompt, api_key), fields(
    gen_ai.system = "openai",
    gen_ai.request.model = %model,
    mv.model.locality = "cloud",
    prompt.len = prompt.len(),
))]
async fn call_openai(
    model: &str,
    endpoint: &str,
    api_key: &str,
    prompt: &str,
) -> Result<String, mv_core::MvError> {
    use rig::client::CompletionClient;

    info!(endpoint = %endpoint, "connecting to OpenAI");

    let client = rig::providers::openai::Client::builder()
        .api_key(api_key)
        .base_url(endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{endpoint}: {e}"),
        })?;

    let agent = client.agent(model).build();

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
    let filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        match verbose {
            0 => EnvFilter::new("warn"),
            1 => EnvFilter::new("info"),
            _ => EnvFilter::new("debug"),
        }
    };

    let fmt_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stderr);

    let otel_layer = otlp_endpoint.and_then(|endpoint| match init_otel_layer(endpoint) {
        Ok(layer) => Some(layer),
        Err(e) => {
            eprintln!("Warning: Failed to initialize OTLP exporter: {e}");
            None
        }
    });

    tracing_subscriber::registry()
        .with(filter)
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
