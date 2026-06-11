//! Command-line surface: clap definitions only.

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "mv-cli", version, about = "Send a prompt to a local LLM")]
pub struct Cli {
    /// Increase log verbosity (repeat for more: -vv)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    pub verbose: u8,

    /// Enable OTLP trace export [default endpoint: http://localhost:4318]
    #[arg(long, num_args = 0..=1, default_missing_value = "http://localhost:4318", global = true)]
    pub otlp: Option<String>,

    /// Output response as JSON object
    #[arg(short, long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Send a prompt to a model
    Prompt(PromptArgs),
    /// Manage and execute workflows
    Workflow {
        #[command(subcommand)]
        action: WorkflowAction,
    },
}

#[derive(Parser, Debug)]
pub struct PromptArgs {
    /// The prompt to send to the model
    pub prompt: String,

    /// Model name (must exist in config or built-in registry)
    #[arg(short, long)]
    pub model: Option<String>,

    /// Backend endpoint override (any provider; defaults come from models.yaml)
    #[arg(short, long)]
    pub endpoint: Option<String>,

    /// Path to models.yaml config file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Path to MCP servers YAML config file [default: mcp-servers.yaml]
    #[arg(long)]
    pub mcp_config: Option<String>,

    /// Stream tokens to stdout as they arrive (TRT-LLM models only).
    ///
    /// Built-in and MCP tools are attached by default, and the TRT-LLM proxy
    /// streams tool calls as plain text rather than executable calls — so with
    /// tools attached, `--stream` falls back to buffered output (where tool
    /// calling works). Combine with `--no-tools` to stream without tools.
    #[arg(long)]
    pub stream: bool,

    /// Disable all tools (built-in and MCP) for this request.
    ///
    /// Required to actually stream from TRT-LLM: `--stream --no-tools` streams
    /// tokens with no tool access.
    #[arg(long)]
    pub no_tools: bool,
}

#[derive(Subcommand, Debug)]
pub enum WorkflowAction {
    /// Load, validate, and execute a workflow file
    Run(WorkflowRunArgs),
    /// Validate a workflow file without executing it
    Validate(WorkflowValidateArgs),
}

#[derive(Parser, Debug)]
pub struct WorkflowRunArgs {
    /// Path to the workflow YAML file
    pub file: String,

    /// Workflow input (repeatable, format: KEY=VALUE)
    #[arg(short, long = "input", value_parser = parse_key_value)]
    pub inputs: Vec<(String, String)>,

    /// Path to models.yaml config file
    #[arg(short, long)]
    pub config: Option<String>,

    /// Path to MCP servers YAML config file [default: mcp-servers.yaml]
    #[arg(long)]
    pub mcp_config: Option<String>,
}

#[derive(Parser, Debug)]
pub struct WorkflowValidateArgs {
    /// Path to the workflow YAML file
    pub file: String,
}

pub fn parse_key_value(s: &str) -> Result<(String, String), String> {
    let pos = s
        .find('=')
        .ok_or_else(|| format!("invalid KEY=VALUE: no '=' found in '{s}'"))?;
    Ok((s[..pos].to_string(), s[pos + 1..].to_string()))
}
