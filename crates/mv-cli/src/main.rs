use clap::Parser;
use rig::client::{CompletionClient, Nothing};
use rig::completion::Prompt;
use rig::providers::ollama;
use tracing::{debug, info};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "mv-cli", version, about = "Send a prompt to a local LLM")]
struct Cli {
    /// The prompt to send to the model
    prompt: String,

    /// Model name
    #[arg(short, long, default_value = "qwen3:4b")]
    model: String,

    /// Ollama endpoint
    #[arg(short, long, default_value = "http://localhost:11434")]
    endpoint: String,

    /// Output response as JSON object
    #[arg(short, long)]
    json: bool,

    /// Increase log verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

async fn run(cli: &Cli) -> std::result::Result<String, mv_core::MvError> {
    let prompt = mv_core::validate_prompt(&cli.prompt)?;

    debug!(endpoint = %cli.endpoint, model = %cli.model, "backend config");
    info!(endpoint = %cli.endpoint, "connecting to Ollama");

    let client = ollama::Client::builder()
        .api_key(Nothing)
        .base_url(&cli.endpoint)
        .build()
        .map_err(|e| mv_core::MvError::BackendUnreachable {
            endpoint: format!("{}: {e}", cli.endpoint),
        })?;

    let agent = client.agent(&cli.model).build();

    info!("sending prompt to model");
    let response = agent.prompt(prompt).await.map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") || (msg.contains("model") && msg.contains("pull")) {
            mv_core::MvError::ModelNotFound {
                model: cli.model.clone(),
            }
        } else if msg.contains("connection")
            || msg.contains("Connection")
            || msg.contains("connect")
            || msg.contains("tcp")
            || msg.contains("error sending request")
            || msg.contains("HttpError")
        {
            mv_core::MvError::BackendUnreachable {
                endpoint: cli.endpoint.clone(),
            }
        } else {
            mv_core::MvError::CompletionFailed { details: msg }
        }
    })?;

    info!(len = response.len(), "received response");
    Ok(response)
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

fn init_tracing(verbose: u8) {
    let filter = if std::env::var("RUST_LOG").is_ok() {
        EnvFilter::from_default_env()
    } else {
        match verbose {
            0 => EnvFilter::new("warn"),
            1 => EnvFilter::new("info"),
            _ => EnvFilter::new("debug"),
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match run(&cli).await {
        Ok(response) => {
            print_success(&response, cli.json);
        }
        Err(err) => {
            print_error(&err, cli.json);
            std::process::exit(1);
        }
    }
}
